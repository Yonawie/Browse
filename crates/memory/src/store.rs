use crate::{new_id, now_ms, MemoryError, Result, SCHEMA_SQL, SCHEMA_VERSION};
use core_types::{Origin, PageKind, Provenance, Sensitivity};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SitePermissionRecord {
    pub origin: String,
    pub permission: String,
    pub state: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSessionRecord {
    pub id: String,
    pub profile_id: String,
    pub name: String,
    pub tabs_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPageVersion {
    pub id: String,
    pub page_id: String,
    pub content_hash: String,
    pub title: Option<String>,
    pub main_text: Option<String>,
    pub captured_at: i64,
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
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS site_permissions (
               origin     TEXT NOT NULL,
               permission TEXT NOT NULL,
               state      TEXT NOT NULL CHECK (state IN ('allow','block','ask')),
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (origin, permission)
             );
             CREATE TABLE IF NOT EXISTS saved_sessions (
               id         TEXT PRIMARY KEY,
               profile_id TEXT NOT NULL,
               name       TEXT NOT NULL,
               tabs_json  TEXT NOT NULL,
               created_at INTEGER NOT NULL
             );",
        )?;
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

    pub fn list_profiles(&self) -> Result<Vec<ProfileRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, kind, name, created_at FROM profiles ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(ProfileRecord {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn create_profile(&self, kind: &str, name: &str) -> Result<ProfileRecord> {
        let id = new_id();
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO profiles(id, kind, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind, name, now],
        )?;
        Ok(ProfileRecord {
            id,
            kind: kind.to_string(),
            name: name.to_string(),
            created_at: now,
        })
    }

    pub fn delete_profile(&self, id: &str) -> Result<bool> {
        let count = self.conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    // -- site permissions -------------------------------------------------

    pub fn get_site_permissions(&self, origin: &str) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT permission, state FROM site_permissions WHERE origin = ?1")?;
        let rows = stmt.query_map(params![origin], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map = HashMap::new();
        for r in rows {
            let (k, v) = r?;
            map.insert(k, v);
        }
        Ok(map)
    }

    pub fn set_site_permission(&self, origin: &str, permission: &str, state: &str) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO site_permissions(origin, permission, state, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(origin, permission) DO UPDATE SET state = excluded.state, updated_at = excluded.updated_at",
            params![origin, permission, state, now],
        )?;
        Ok(())
    }

    pub fn list_site_permissions(&self) -> Result<Vec<SitePermissionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT origin, permission, state, updated_at FROM site_permissions ORDER BY origin ASC, permission ASC"
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SitePermissionRecord {
                origin: r.get(0)?,
                permission: r.get(1)?,
                state: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn clear_site_permissions(&self, origin: &str) -> Result<bool> {
        let count = self.conn.execute("DELETE FROM site_permissions WHERE origin = ?1", params![origin])?;
        Ok(count > 0)
    }

    // -- saved sessions & crash recovery ---------------------------------

    pub fn save_session(&self, id: Option<&str>, profile_id: &str, name: &str, tabs_json: &str) -> Result<SavedSessionRecord> {
        let id = id.map(ToString::to_string).unwrap_or_else(new_id);
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO saved_sessions(id, profile_id, name, tabs_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, tabs_json = excluded.tabs_json, created_at = excluded.created_at",
            params![id, profile_id, name, tabs_json, now],
        )?;
        Ok(SavedSessionRecord {
            id,
            profile_id: profile_id.to_string(),
            name: name.to_string(),
            tabs_json: tabs_json.to_string(),
            created_at: now,
        })
    }

    pub fn list_saved_sessions(&self, profile_id: &str) -> Result<Vec<SavedSessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, name, tabs_json, created_at FROM saved_sessions WHERE profile_id = ?1 AND name != '__last_active__' ORDER BY created_at DESC, rowid DESC"
        )?;
        let rows = stmt.query_map(params![profile_id], |r| {
            Ok(SavedSessionRecord {
                id: r.get(0)?,
                profile_id: r.get(1)?,
                name: r.get(2)?,
                tabs_json: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn get_saved_session(&self, id: &str) -> Result<Option<SavedSessionRecord>> {
        let row = self.conn.query_row(
            "SELECT id, profile_id, name, tabs_json, created_at FROM saved_sessions WHERE id = ?1",
            params![id],
            |r| {
                Ok(SavedSessionRecord {
                    id: r.get(0)?,
                    profile_id: r.get(1)?,
                    name: r.get(2)?,
                    tabs_json: r.get(3)?,
                    created_at: r.get(4)?,
                })
            },
        ).optional()?;
        Ok(row)
    }

    pub fn delete_saved_session(&self, id: &str) -> Result<bool> {
        let count = self.conn.execute("DELETE FROM saved_sessions WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    pub fn save_active_tabs(&self, profile_id: &str, tabs_json: &str) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO saved_sessions(id, profile_id, name, tabs_json, created_at) VALUES (?1, ?2, '__last_active__', ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET tabs_json = excluded.tabs_json, created_at = excluded.created_at",
            params![format!("last_active_{profile_id}"), profile_id, tabs_json, now],
        )?;
        Ok(())
    }

    pub fn load_active_tabs(&self, profile_id: &str) -> Result<Option<String>> {
        let id = format!("last_active_{profile_id}");
        let row = self.conn.query_row(
            "SELECT tabs_json FROM saved_sessions WHERE id = ?1",
            params![id],
            |r| r.get::<_, String>(0),
        ).optional()?;
        Ok(row)
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

    /// Get the most recently captured version of a page by URL.
    pub fn get_latest_version_for_url(&self, url: &str) -> Result<Option<StoredPageVersion>> {
        let row = self
            .conn
            .query_row(
                "SELECT pv.id, pv.page_id, pv.content_hash, pv.title, pv.main_text_zst, pv.captured_at
                 FROM page_versions pv
                 JOIN pages p ON p.id = pv.page_id
                 WHERE p.url = ?1
                 ORDER BY pv.captured_at DESC
                 LIMIT 1",
                params![url],
                |r| {
                    let text_blob: Option<Vec<u8>> = r.get(4)?;
                    let main_text = text_blob.map(|b| String::from_utf8_lossy(&b).to_string());
                    Ok(StoredPageVersion {
                        id: r.get(0)?,
                        page_id: r.get(1)?,
                        content_hash: r.get(2)?,
                        title: r.get(3)?,
                        main_text,
                        captured_at: r.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Get the previous (older) version of a page by URL.
    pub fn get_previous_version_for_url(&self, url: &str) -> Result<Option<StoredPageVersion>> {
        let row = self
            .conn
            .query_row(
                "SELECT pv.id, pv.page_id, pv.content_hash, pv.title, pv.main_text_zst, pv.captured_at
                 FROM page_versions pv
                 JOIN pages p ON p.id = pv.page_id
                 WHERE p.url = ?1
                 ORDER BY pv.captured_at DESC
                 LIMIT 1 OFFSET 1",
                params![url],
                |r| {
                    let text_blob: Option<Vec<u8>> = r.get(4)?;
                    let main_text = text_blob.map(|b| String::from_utf8_lossy(&b).to_string());
                    Ok(StoredPageVersion {
                        id: r.get(0)?,
                        page_id: r.get(1)?,
                        content_hash: r.get(2)?,
                        title: r.get(3)?,
                        main_text,
                        captured_at: r.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
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

    #[test]
    fn page_version_history_and_retrieval() {
        let s = store();
        let url = "https://example.com/pricing";

        // Version 1
        let v1 = NewPageVersion {
            url,
            title: Some("Pricing v1"),
            lang: Some("en"),
            page_kind: PageKind::Product,
            sensitivity: Sensitivity::Public,
            main_text: Some("Pro plan: $99/mo"),
            content_hash: "hash-v1",
        };
        s.insert_page_version(&v1).unwrap();

        let latest = s.get_latest_version_for_url(url).unwrap().expect("latest version present");
        assert_eq!(latest.content_hash, "hash-v1");
        assert_eq!(latest.main_text.as_deref(), Some("Pro plan: $99/mo"));

        let prev = s.get_previous_version_for_url(url).unwrap();
        assert!(prev.is_none());

        // Version 2 (captured later)
        let v2 = NewPageVersion {
            url,
            title: Some("Pricing v2"),
            lang: Some("en"),
            page_kind: PageKind::Product,
            sensitivity: Sensitivity::Public,
            main_text: Some("Pro plan: $79/mo"),
            content_hash: "hash-v2",
        };
        s.insert_page_version(&v2).unwrap();

        let latest2 = s.get_latest_version_for_url(url).unwrap().expect("latest version present");
        assert_eq!(latest2.content_hash, "hash-v2");
        assert_eq!(latest2.main_text.as_deref(), Some("Pro plan: $79/mo"));

        let prev2 = s.get_previous_version_for_url(url).unwrap().expect("previous version present");
        assert_eq!(prev2.content_hash, "hash-v1");
        assert_eq!(prev2.main_text.as_deref(), Some("Pro plan: $99/mo"));
    }

    #[test]
    fn profile_creation_listing_and_deletion() {
        let s = MemoryStore::open_in_memory().unwrap();
        let p1 = s.create_profile("user", "Personal").unwrap();
        let p2 = s.create_profile("user", "Work").unwrap();

        let list = s.list_profiles().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Personal");
        assert_eq!(list[1].name, "Work");

        let deleted = s.delete_profile(&p1.id).unwrap();
        assert!(deleted);
        let list2 = s.list_profiles().unwrap();
        assert_eq!(list2.len(), 1);
        assert_eq!(list2[0].id, p2.id);
    }

    #[test]
    fn site_permissions_setting_and_retrieval() {
        let s = MemoryStore::open_in_memory().unwrap();
        let origin = "https://maps.google.com";

        s.set_site_permission(origin, "geolocation", "allow").unwrap();
        s.set_site_permission(origin, "camera", "block").unwrap();

        let perms = s.get_site_permissions(origin).unwrap();
        assert_eq!(perms.get("geolocation").map(String::as_str), Some("allow"));
        assert_eq!(perms.get("camera").map(String::as_str), Some("block"));
        assert_eq!(perms.get("microphone"), None);

        // Update existing permission
        s.set_site_permission(origin, "geolocation", "block").unwrap();
        let perms_updated = s.get_site_permissions(origin).unwrap();
        assert_eq!(perms_updated.get("geolocation").map(String::as_str), Some("block"));

        let all = s.list_site_permissions().unwrap();
        assert_eq!(all.len(), 2);

        let cleared = s.clear_site_permissions(origin).unwrap();
        assert!(cleared);
        assert!(s.get_site_permissions(origin).unwrap().is_empty());
    }

    #[test]
    fn saved_sessions_and_crash_recovery() {
        let s = MemoryStore::open_in_memory().unwrap();
        let profile = "default";

        // 1. Save and list sessions
        let sess1 = s.save_session(None, profile, "Research Project", r#"[{"url":"https://github.com"}]"#).unwrap();
        let sess2 = s.save_session(None, profile, "Shopping", r#"[{"url":"https://shop.com"}]"#).unwrap();

        let list = s.list_saved_sessions(profile).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|s| s.name == "Shopping"));
        assert!(list.iter().any(|s| s.name == "Research Project"));

        let fetched = s.get_saved_session(&sess1.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Research Project");
        assert_eq!(fetched.tabs_json, r#"[{"url":"https://github.com"}]"#);

        let deleted = s.delete_saved_session(&sess2.id).unwrap();
        assert!(deleted);
        assert_eq!(s.list_saved_sessions(profile).unwrap().len(), 1);

        // 2. Active tabs auto-save for crash recovery
        s.save_active_tabs(profile, r#"[{"url":"https://crates.io"}]"#).unwrap();
        let active_json = s.load_active_tabs(profile).unwrap().unwrap();
        assert_eq!(active_json, r#"[{"url":"https://crates.io"}]"#);
    }
}

