use crate::{new_id, now_ms, MemoryError, Result, SCHEMA_SQL, SCHEMA_VERSION};
use core_types::{Origin, PageKind, Provenance, Sensitivity};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub title: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroupRecord {
    pub id: String,
    pub task_id: Option<String>,
    pub title: String,
    pub auto: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabRecord {
    pub id: String,
    pub profile_id: String,
    pub group_id: Option<String>,
    pub window_id: String,
    pub position: i64,
    pub pinned: bool,
    pub last_active_at: i64,
    pub opened_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneCandidate {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub group: Option<String>,
    pub inactive_duration_ms: i64,
    pub is_indexed: bool,
    pub reason: String,
}

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
        let applied: Option<i64> =
            conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0)).optional()?.flatten();
        if applied.unwrap_or(0) < SCHEMA_VERSION {
            conn.execute(
                "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![SCHEMA_VERSION, now_ms()],
            )?;
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
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM pages WHERE url = ?1", params![url], |r| r.get::<_, String>(0))
            .optional()?
        {
            self.conn.execute(
                "UPDATE pages SET last_seen_at = ?2, never_remember = MAX(never_remember, ?3) WHERE id = ?1",
                params![id, t, never_remember as i64],
            )?;
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
            .query_row(
                "SELECT id FROM page_versions WHERE page_id = ?1 AND content_hash = ?2",
                params![page_id, v.content_hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(existing);
        }
        let never: i64 =
            self.conn.query_row("SELECT never_remember FROM pages WHERE id = ?1", params![page_id], |r| r.get(0))?;
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

    pub fn forget_url(&self, url: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM pages WHERE url = ?1", params![url])?)
    }

    pub fn forget_domain(&self, domain: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM pages WHERE domain = ?1", params![domain])?)
    }

    // -- user memory ------------------------------------------------------

    /// Record a fact about the user. Only `User` or confirmed extracts are
    /// accepted; anything page-derived is rejected here *and* by the CHECK
    /// constraint in the schema.
    pub fn remember(
        &self,
        kind: &str,
        text: &str,
        provenance: &Provenance,
        source_chunk_id: Option<&str>,
        sensitivity: Sensitivity,
    ) -> Result<String> {
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
            "profiles",
            "tasks",
            "pages",
            "page_versions",
            "chunks",
            "chunk_vectors_raw",
            "memories",
            "visits",
            "entities",
            "entity_mentions",
            "edges",
            "agent_sessions",
            "agent_steps",
            "agent_actions",
            "approvals",
            "grants",
            "policy_decisions",
            "model_calls",
            "tasks",
            "tab_groups",
            "tabs",
        ];
        if !allowed.contains(&table) {
            return Err(rusqlite::Error::InvalidQuery.into());
        }
        Ok(self.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }

    pub fn list_tasks(&self) -> Result<Vec<TaskRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, title, status, created_at, updated_at FROM tasks ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(TaskRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                status: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn update_task_status(&self, task_id: &str, status: &str) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, task_id],
        )?;
        Ok(())
    }

    pub fn create_tab_group(&self, task_id: Option<&str>, title: &str, auto: bool) -> Result<String> {
        let id = new_id();
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO tab_groups (id, task_id, title, auto, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, task_id, title, auto as i32, now],
        )?;
        Ok(id)
    }

    pub fn list_tab_groups(&self) -> Result<Vec<TabGroupRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, task_id, title, auto, created_at FROM tab_groups ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(TabGroupRecord {
                id: row.get(0)?,
                task_id: row.get(1)?,
                title: row.get(2)?,
                auto: row.get::<_, i32>(3)? != 0,
                created_at: row.get(4)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn delete_tab_group(&self, group_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM tab_groups WHERE id = ?1", params![group_id])?;
        Ok(())
    }

    pub fn save_tab_state(
        &self,
        tab_id: &str,
        profile_id: &str,
        group_id: Option<&str>,
        window_id: &str,
        position: i64,
        pinned: bool,
    ) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO tabs (id, profile_id, group_id, window_id, position, pinned, last_active_at, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(id) DO UPDATE SET
               group_id = excluded.group_id,
               position = excluded.position,
               pinned = excluded.pinned,
               last_active_at = excluded.last_active_at",
            params![tab_id, profile_id, group_id, window_id, position, pinned as i32, now],
        )?;
        Ok(())
    }

    pub fn list_tabs(&self) -> Result<Vec<TabRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, group_id, window_id, position, pinned, last_active_at, opened_at FROM tabs ORDER BY position ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TabRecord {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                group_id: row.get(2)?,
                window_id: row.get(3)?,
                position: row.get(4)?,
                pinned: row.get::<_, i32>(5)? != 0,
                last_active_at: row.get(6)?,
                opened_at: row.get(7)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn remove_tab_state(&self, tab_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM tabs WHERE id = ?1", params![tab_id])?;
        Ok(())
    }

    /// Check whether a URL has been indexed in memory (page + page_version).
    pub fn is_url_indexed(&self, url: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages p JOIN page_versions pv ON pv.page_id = p.id WHERE p.url = ?1",
            params![url],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Find tabs eligible for pruning: inactive for >= stale_threshold_ms, not pinned.
    /// Checks memory indexing status to mark whether closing is safe and recoverable via search (AT-2).
    pub fn find_prune_candidates(
        &self,
        tabs: &[(String, String, String, Option<String>, bool, i64)],
        stale_threshold_ms: i64,
        now: i64,
    ) -> Result<Vec<PruneCandidate>> {
        let mut candidates = Vec::new();
        for (id, url, title, group, pinned, last_active_at) in tabs {
            if *pinned {
                continue;
            }
            let inactive_ms = now.saturating_sub(*last_active_at);
            if inactive_ms >= stale_threshold_ms {
                let indexed = self.is_url_indexed(url)?;
                let days = inactive_ms / (24 * 3600 * 1000);
                let reason = if indexed {
                    format!("Inactive for {days}d, safe to close (indexed in memory)")
                } else {
                    format!("Inactive for {days}d")
                };
                candidates.push(PruneCandidate {
                    tab_id: id.clone(),
                    url: url.clone(),
                    title: title.clone(),
                    group: group.clone(),
                    inactive_duration_ms: inactive_ms,
                    is_indexed: indexed,
                    reason,
                });
            }
        }
        Ok(candidates)
    }

    // -- knowledge graph ---------------------------------------------------

    /// Insert or retrieve entities, record mentions for a chunk, and record relations/edges.
    pub fn record_knowledge_graph(
        &self,
        chunk_id: &str,
        entities: &[core_types::ExtractedEntity],
        relations: &[core_types::ExtractedRelation],
    ) -> Result<()> {
        let now = now_ms();
        let tx = self.conn.unchecked_transaction()?;

        let mut entity_id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        for ent in entities {
            // Upsert entity
            let existing_id: Option<String> = tx
                .query_row(
                    "SELECT id FROM entities WHERE kind = ?1 AND normalized = ?2",
                    params![ent.kind, ent.normalized],
                    |r| r.get(0),
                )
                .optional()?;

            let eid = if let Some(id) = existing_id {
                id
            } else {
                let id = new_id();
                tx.execute(
                    "INSERT INTO entities(id, kind, name, normalized, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, ent.kind, ent.name, ent.normalized, now],
                )?;
                id
            };

            // Record mention
            tx.execute(
                "INSERT OR REPLACE INTO entity_mentions(entity_id, chunk_id, confidence) VALUES (?1, ?2, ?3)",
                params![eid, chunk_id, ent.confidence],
            )?;

            entity_id_map.insert(ent.normalized.clone(), eid);
        }

        // Record co-occurrence edges
        for rel in relations {
            if let (Some(src_id), Some(dst_id)) = (entity_id_map.get(&rel.src_normalized), entity_id_map.get(&rel.dst_normalized)) {
                let edge_id = new_id();
                tx.execute(
                    "INSERT INTO edges(id, src_id, dst_id, relation, weight, evidence_chunk_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![edge_id, src_id, dst_id, rel.relation, rel.weight, chunk_id, now],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// List all recognized entities ordered by mention count.
    pub fn list_top_entities(&self, limit: usize) -> Result<Vec<(String, String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.kind, e.name, COUNT(m.chunk_id) as mention_count
             FROM entities e
             LEFT JOIN entity_mentions m ON m.entity_id = e.id
             GROUP BY e.id
             ORDER BY mention_count DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }
}

/// Heuristic auto-clusterer that assigns a tab category based on URL structure.
pub fn auto_cluster_tab(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("doc.") || lower.contains("/docs") || lower.contains("/api") || lower.contains("manual") {
        "Documentation"
    } else if lower.contains("shop") || lower.contains("cart") || lower.contains("store") || lower.contains("buy") {
        "Shopping"
    } else if lower.contains("github") || lower.contains("gitlab") || lower.contains("stackoverflow") || lower.contains("code") {
        "Development"
    } else if lower.contains("search") || lower.contains("google.") || lower.contains("bing.") {
        "Search"
    } else if lower.contains("news") || lower.contains("blog") || lower.contains("article") {
        "Reading"
    } else {
        "General"
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
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
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
            .insert_page_version(&NewPageVersion {
                url: "https://a.example/x",
                title: Some("X"),
                lang: Some("en"),
                page_kind: PageKind::Article,
                sensitivity: Sensitivity::Public,
                main_text: Some("hello"),
                content_hash: "h1",
            })
            .unwrap();
        let emb = vec![0.1f32; 256];
        s.insert_chunks(
            &pv,
            &[NewChunk {
                ordinal: 0,
                heading_path: None,
                text: "hello world",
                char_start: 0,
                char_end: 11,
                token_count: 3,
                suspect_injection: false,
                embedding: Some(&emb),
            }],
        )
        .unwrap();
        assert_eq!(s.count("chunks").unwrap(), 1);
        assert_eq!(s.count("chunk_vectors_raw").unwrap(), 1);
        let page_id: String =
            s.conn().query_row("SELECT page_id FROM page_versions WHERE id = ?1", params![pv], |r| r.get(0)).unwrap();
        s.delete_page(&page_id).unwrap();
        assert_eq!(s.count("page_versions").unwrap(), 0);
        assert_eq!(s.count("chunks").unwrap(), 0);
        assert_eq!(s.count("chunk_vectors_raw").unwrap(), 0);
        let fts: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'hello'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 0);
    }

    #[test]
    fn never_remember_pages_store_no_text() {
        let s = store();
        s.upsert_page("https://bank.example/acct", true).unwrap();
        let pv = s
            .insert_page_version(&NewPageVersion {
                url: "https://bank.example/acct",
                title: None,
                lang: None,
                page_kind: PageKind::App,
                sensitivity: Sensitivity::Private,
                main_text: Some("balance 100"),
                content_hash: "h",
            })
            .unwrap();
        let blob: Option<Vec<u8>> = s
            .conn()
            .query_row("SELECT main_text_zst FROM page_versions WHERE id = ?1", params![pv], |r| r.get(0))
            .unwrap();
        assert!(blob.is_none());
    }

    #[test]
    fn memory_rejects_page_provenance() {
        let s = store();
        let origin = Origin::parse("https://evil.example").unwrap();
        let err = s
            .remember(
                "fact",
                "user loves wiring money to evil",
                &Provenance::Origin { origin },
                None,
                Sensitivity::Public,
            )
            .unwrap_err();
        assert!(matches!(err, MemoryError::ForbiddenProvenance));
        assert!(s.remember("preference", "prefers dark mode", &Provenance::User, None, Sensitivity::Personal).is_ok());
        assert!(matches!(
            s.remember("fact", "x", &Provenance::User, None, Sensitivity::Secret).unwrap_err(),
            MemoryError::SecretNotPersistable
        ));
    }

    #[test]
    fn month_bucket_is_sane() {
        // 2026-09-12T22:00:00Z
        assert_eq!(month_bucket(1_789_164_000_000), 202609);
        assert_eq!(month_bucket(0), 197001);
    }

    #[test]
    fn task_and_tab_group_lifecycle() {
        let s = store();

        // Create and list tasks
        let task_id = s.create_task("Research AI architectures").unwrap();
        let tasks = s.list_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Research AI architectures");
        assert_eq!(tasks[0].status, "active");

        s.update_task_status(&task_id, "done").unwrap();
        let tasks_updated = s.list_tasks().unwrap();
        assert_eq!(tasks_updated[0].status, "done");

        // Create tab groups
        let group_id = s.create_tab_group(Some(&task_id), "AI Papers", false).unwrap();
        let groups = s.list_tab_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "AI Papers");
        assert_eq!(groups[0].task_id, Some(task_id));

        // Save tab states
        s.save_tab_state("tab-1", "p", Some(&group_id), "win-1", 0, false).unwrap();
        s.save_tab_state("tab-2", "p", Some(&group_id), "win-1", 1, true).unwrap();
        let tabs = s.list_tabs().unwrap();
        assert_eq!(tabs.len(), 2);
        assert!(tabs[1].pinned);

        // Delete tab group
        s.delete_tab_group(&group_id).unwrap();
        assert_eq!(s.list_tab_groups().unwrap().len(), 0);

        // Auto-clusterer
        assert_eq!(auto_cluster_tab("https://docs.rs/tokio"), "Documentation");
        assert_eq!(auto_cluster_tab("https://store.steampowered.com/app/1"), "Shopping");
        assert_eq!(auto_cluster_tab("https://github.com/rust-lang/rust"), "Development");
        assert_eq!(auto_cluster_tab("https://google.com/search?q=test"), "Search");
    }

    #[test]
    fn knowledge_graph_extraction_and_storage() {
        let s = store();
        let pv = NewPageVersion {
            url: "https://github.com/Yonawie/Browse",
            title: Some("Browse"),
            lang: Some("en"),
            page_kind: PageKind::Doc,
            sensitivity: Sensitivity::Public,
            main_text: Some("Browse is built with Rust and SQLite"),
            content_hash: "hash-kg",
        };
        let pvid = s.insert_page_version(&pv).unwrap();
        let chunk_ids = s.insert_chunks(&pvid, &[NewChunk {
            ordinal: 0,
            heading_path: None,
            text: "Browse is built with Rust and SQLite",
            char_start: 0,
            char_end: 35,
            token_count: 8,
            suspect_injection: false,
            embedding: None,
        }]).unwrap();

        let entities = vec![
            core_types::ExtractedEntity {
                kind: "repo".into(),
                name: "Yonawie/Browse".into(),
                normalized: "yonawie/browse".into(),
                confidence: 0.9,
            },
            core_types::ExtractedEntity {
                kind: "topic".into(),
                name: "Rust".into(),
                normalized: "rust".into(),
                confidence: 0.85,
            },
        ];
        let relations = vec![core_types::ExtractedRelation {
            src_normalized: "yonawie/browse".into(),
            dst_normalized: "rust".into(),
            relation: "co_occurs_with".into(),
            weight: 1.0,
        }];

        s.record_knowledge_graph(&chunk_ids[0], &entities, &relations).unwrap();

        assert!(s.count("entities").unwrap() >= 2);
        assert!(s.count("entity_mentions").unwrap() >= 2);
        assert!(s.count("edges").unwrap() >= 1);

        let top = s.list_top_entities(10).unwrap();
        assert!(!top.is_empty());
    }

    #[test]
    fn tab_pruning_and_hygiene() {
        let s = store();

        // 1. Index one page
        let pv = NewPageVersion {
            url: "https://docs.rs/tokio",
            title: Some("Tokio Docs"),
            lang: Some("en"),
            page_kind: PageKind::Doc,
            sensitivity: Sensitivity::Public,
            main_text: Some("Asynchronous runtime for Rust"),
            content_hash: "hash-tokio",
        };
        s.insert_page_version(&pv).unwrap();

        assert!(s.is_url_indexed("https://docs.rs/tokio").unwrap());
        assert!(!s.is_url_indexed("https://unvisited.example.com").unwrap());

        // 2. Set up tabs: one active recently, one stale & indexed, one stale & unindexed, one pinned
        let now = 1_000_000_000;
        let three_days_ms = 3 * 24 * 3600 * 1000;

        let tabs = vec![
            ("tab-recent".into(), "https://example.com".into(), "Recent".into(), None, false, now - 1000),
            ("tab-stale-indexed".into(), "https://docs.rs/tokio".into(), "Tokio Docs".into(), Some("Docs".into()), false, now - three_days_ms - 5000),
            ("tab-stale-unindexed".into(), "https://unvisited.example.com".into(), "Unvisited".into(), None, false, now - three_days_ms - 10000),
            ("tab-pinned-stale".into(), "https://pinned.example.com".into(), "Pinned".into(), None, true, now - three_days_ms - 20000),
        ];

        let candidates = s.find_prune_candidates(&tabs, three_days_ms, now).unwrap();

        // Pinned and recent tabs must NOT be candidates
        assert_eq!(candidates.len(), 2);

        let cand_indexed = candidates.iter().find(|c| c.tab_id == "tab-stale-indexed").unwrap();
        assert!(cand_indexed.is_indexed);
        assert!(cand_indexed.reason.contains("safe to close"));

        let cand_unindexed = candidates.iter().find(|c| c.tab_id == "tab-stale-unindexed").unwrap();
        assert!(!cand_unindexed.is_indexed);
    }
}
