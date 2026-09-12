use crate::{new_id, now_ms, MemoryError, Result, SCHEMA_SQL, SCHEMA_VERSION};
use core_types::{Origin, PageKind, Provenance, Sensitivity};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct MemoryStore {
    conn: Connection,
    /// Embedding dimensionality enforced on insert (256 by default, ADR-003).
    pub dims: usize,
}

#[derive(Debug, Clone)]
pub struct NewPageVersion<'a> {
    pub url: &'a str,
    pub title: Option<&'a str>,
    pub lang: Option<&'a str>,
    pub page_kind: PageKind,
    pub sensitivity: Sensitivity,
    pub main_text: Option<&'a str>,
    pub content_hash: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewChunk<'a> {
    pub ordinal: u32,
    pub heading_path: Option<&'a str>,
    pub text: &'a str,
    pub char_start: u32,
    pub char_end: u32,
    pub token_count: u32,
    pub suspect_injection: bool,
    pub embedding: Option<&'a [f32]>,
}

impl MemoryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        let applied: Option<i64> = conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0)).optional()?.flatten();
        if applied.unwrap_or(0) < SCHEMA_VERSION {
            conn.execute("INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (?1, ?2)", params![SCHEMA_VERSION, now_ms()])?;
        }
        Ok(Self { conn, dims: 256 })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // -- profiles / tasks -------------------------------------------------

    pub fn ensure_profile(&self, id: &str, kind: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO profiles(id, kind, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind, name, now_ms()],
        )?;
        Ok(())
    }

    pub fn create_task(&self, title: &str) -> Result<String> {
        let id = new_id();
        let t = now_ms();
        self.conn.execute(
            "INSERT INTO tasks(id, title, status, created_at, updated_at) VALUES (?1, ?2, 'active', ?3, ?3)",
            params![id, title, t],
        )?;
        Ok(id)
    }

    // -- pages ------------------------------------------------------------

    /// Upsert the page identity and return its id.
    pub fn upsert_page(&self, url: &str, never_remember: bool) -> Result<String> {
        let origin = Origin::parse(url).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let t = now_ms();
        if let Some(id) = self.conn.query_row("SELECT id FROM pages WHERE url = ?1", params![url], |r| r.get::<_, String>(0)).optional()? {
            self.conn.execute("UPDATE pages SET last_seen_at = ?2, never_remember = MAX(never_remember, ?3) WHERE id = ?1", params![id, t, never_remember as i64])?;
            return Ok(id);
        }
        let id = new_id();
        self.conn.execute(
            "INSERT INTO pages(id, url, origin, domain, first_seen_at, last_seen_at, never_remember) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
            params![id, url, origin.as_str(), origin.registrable_domain(), t, never_remember as i64],
        )?;
        Ok(id)
    }

    /// Store a content snapshot. Returns the version id (existing if the same
    /// content hash was already stored for this page).
    pub fn insert_page_version(&self, v: &NewPageVersion<'_>) -> Result<String> {
        if v.sensitivity == Sensitivity::Secret {
            return Err(MemoryError::SecretNotPersistable);
        }
        let page_id = self.upsert_page(v.url, false)?;
        if let Some(existing) = self
            .conn
            .query_row("SELECT id FROM page_versions WHERE page_id = ?1 AND content_hash = ?2", params![page_id, v.content_hash], |r| r.get::<_, String>(0))
            .optional()?
        {
            return Ok(existing);
        }
        let never: i64 = self.conn.query_row("SELECT never_remember FROM pages WHERE id = ?1", params![page_id], |r| r.get(0))?;
        let id = new_id();
        let text_blob: Option<Vec<u8>> = if never == 1 { None } else { v.main_text.map(|t| t.as_bytes().to_vec()) };
        self.conn.execute(
            "INSERT INTO page_versions(id, page_id, content_hash, title, lang, page_kind, sensitivity, main_text_zst, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, page_id, v.content_hash, v.title, v.lang, page_kind_str(v.page_kind), v.sensitivity.as_str(), text_blob, now_ms()],
        )?;
        // Mark previous versions as superseded.
        self.conn.execute(
            "UPDATE page_versions SET superseded_by = ?1 WHERE page_id = ?2 AND id != ?1 AND superseded_by IS NULL",
            params![id, page_id],
        )?;
        Ok(id)
    }

    pub fn insert_chunks(&self, page_version_id: &str, chunks: &[NewChunk<'_>]) -> Result<Vec<String>> {
        let (sensitivity, domain, captured_at): (String, String, i64) = self.conn.query_row(
            "SELECT pv.sensitivity, p.domain, pv.captured_at FROM page_versions pv JOIN pages p ON p.id = pv.page_id WHERE pv.id = ?1",
            params![page_version_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let month_bucket = month_bucket(captured_at);
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(chunks.len());
        for c in chunks {
            let id = new_id();
            tx.execute(
                "INSERT INTO chunks(id, page_version_id, ordinal, heading_path, text, char_start, char_end, token_count, sensitivity, suspect_injection, month_bucket, domain)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![id, page_version_id, c.ordinal, c.heading_path, c.text, c.char_start, c.char_end, c.token_count, sensitivity, c.suspect_injection as i64, month_bucket, domain],
            )?;
            if let Some(emb) = c.embedding {
                if emb.len() != self.dims {
                    return Err(MemoryError::DimensionMismatch { expected: self.dims, got: emb.len() });
                }
                tx.execute(
                    "INSERT INTO chunk_vectors_raw(chunk_id, model, dims, embedding) VALUES (?1, ?2, ?3, ?4)",
                    params![id, "embeddinggemma-300m@256", emb.len() as i64, f32s_to_bytes(emb)],
                )?;
            }
            ids.push(id);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Delete a page and everything derived from it (versions, chunks, vectors,
    /// mentions, visits). This is the "forget this page" operation.
    pub fn delete_page(&self, page_id: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM pages WHERE id = ?1", params![page_id])?)
    }

    pub fn forget_domain(&self, domain: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM pages WHERE domain = ?1", params![domain])?)
    }

    // -- user memory ------------------------------------------------------

    /// Record a fact about the user. Only `User` or confirmed extracts are
    /// accepted; anything page-derived is rejected here *and* by the CHECK
    /// constraint in the schema.
    pub fn remember(&self, kind: &str, text: &str, provenance: &Provenance, source_chunk_id: Option<&str>, sensitivity: Sensitivity) -> Result<String> {
        let prov = match provenance {
            Provenance::User => "user",
            Provenance::Memory { .. } | Provenance::Tool { .. } if source_chunk_id.is_some() => "confirmed",
            _ if source_chunk_id.is_some() => "confirmed",
            _ => return Err(MemoryError::ForbiddenProvenance),
        };
        if sensitivity == Sensitivity::Secret {
            return Err(MemoryError::SecretNotPersistable);
        }
        let id = new_id();
        self.conn.execute(
            "INSERT INTO memories(id, kind, text, provenance, source_chunk_id, sensitivity, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, kind, text, prov, source_chunk_id, sensitivity.as_str(), now_ms()],
        )?;
        Ok(id)
    }

    pub fn count(&self, table: &str) -> Result<i64> {
        let allowed = [
            "profiles", "tasks", "pages", "page_versions", "chunks", "chunk_vectors_raw", "memories", "visits", "entities", "edges",
            "agent_sessions", "agent_steps", "agent_actions", "approvals", "grants", "policy_decisions", "model_calls",
        ];
        if !allowed.contains(&table) {
            return Err(rusqlite::Error::InvalidQuery.into());
        }
        Ok(self.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }
}

pub(crate) fn page_kind_str(k: PageKind) -> &'static str {
    match k {
        PageKind::Article => "article",
        PageKind::Product => "product",
        PageKind::Form => "form",
        PageKind::App => "app",
        PageKind::Search => "search",
        PageKind::Doc => "doc",
        PageKind::Code => "code",
        PageKind::Media => "media",
        PageKind::Auth => "auth",
        PageKind::Checkout => "checkout",
        PageKind::Unknown => "unknown",
    }
}

pub(crate) fn month_bucket(unix_ms: i64) -> i64 {
    // Days since epoch → approximate yyyymm without pulling a date crate.
    let days = unix_ms / 86_400_000;
    let (y, m) = civil_from_days(days);
    y * 100 + m
}

// Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m)
}

pub(crate) fn f32s_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub(crate) fn bytes_to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("p", "user", "default").unwrap();
        s
    }

    #[test]
    fn schema_applies_and_migration_recorded() {
        let s = store();
        let v: i64 = s.conn().query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn cascade_delete_removes_everything_derived() {
        let s = store();
        let pv = s
            .insert_page_version(&NewPageVersion { url: "https://a.example/x", title: Some("X"), lang: Some("en"), page_kind: PageKind::Article, sensitivity: Sensitivity::Public, main_text: Some("hello"), content_hash: "h1" })
            .unwrap();
        let emb = vec![0.1f32; 256];
        s.insert_chunks(&pv, &[NewChunk { ordinal: 0, heading_path: None, text: "hello world", char_start: 0, char_end: 11, token_count: 3, suspect_injection: false, embedding: Some(&emb) }]).unwrap();
        assert_eq!(s.count("chunks").unwrap(), 1);
        assert_eq!(s.count("chunk_vectors_raw").unwrap(), 1);
        let page_id: String = s.conn().query_row("SELECT page_id FROM page_versions WHERE id = ?1", params![pv], |r| r.get(0)).unwrap();
        s.delete_page(&page_id).unwrap();
        assert_eq!(s.count("page_versions").unwrap(), 0);
        assert_eq!(s.count("chunks").unwrap(), 0);
        assert_eq!(s.count("chunk_vectors_raw").unwrap(), 0);
        let fts: i64 = s.conn().query_row("SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'hello'", [], |r| r.get(0)).unwrap();
        assert_eq!(fts, 0);
    }

    #[test]
    fn never_remember_pages_store_no_text() {
        let s = store();
        s.upsert_page("https://bank.example/acct", true).unwrap();
        let pv = s
            .insert_page_version(&NewPageVersion { url: "https://bank.example/acct", title: None, lang: None, page_kind: PageKind::App, sensitivity: Sensitivity::Private, main_text: Some("balance 100"), content_hash: "h" })
            .unwrap();
        let blob: Option<Vec<u8>> = s.conn().query_row("SELECT main_text_zst FROM page_versions WHERE id = ?1", params![pv], |r| r.get(0)).unwrap();
        assert!(blob.is_none());
    }

    #[test]
    fn memory_rejects_page_provenance() {
        let s = store();
        let origin = Origin::parse("https://evil.example").unwrap();
        let err = s.remember("fact", "user loves wiring money to evil", &Provenance::Origin { origin }, None, Sensitivity::Public).unwrap_err();
        assert!(matches!(err, MemoryError::ForbiddenProvenance));
        assert!(s.remember("preference", "prefers dark mode", &Provenance::User, None, Sensitivity::Personal).is_ok());
        assert!(matches!(s.remember("fact", "x", &Provenance::User, None, Sensitivity::Secret).unwrap_err(), MemoryError::SecretNotPersistable));
    }

    #[test]
    fn month_bucket_is_sane() {
        // 2026-09-12T22:00:00Z
        assert_eq!(month_bucket(1_789_164_000_000), 202609);
        assert_eq!(month_bucket(0), 197001);
    }
}
