use crate::{new_id, now_ms, MemoryStore, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A recorded history visit item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryVisitRecord {
    pub id: String,
    pub page_id: String,
    pub url: String,
    pub domain: String,
    pub title: Option<String>,
    pub transition: String,
    pub started_at: i64,
    pub dwell_ms: Option<i64>,
}

fn parse_domain_and_origin(url: &str) -> (String, String) {
    let lower = url.to_lowercase();
    let without_proto = if let Some(pos) = lower.find("://") {
        let proto = &lower[..pos + 3];
        let rest = &lower[pos + 3..];
        let host = rest.split(&['/', '?', '#', ':'][..]).next().unwrap_or("");
        let origin = format!("{proto}{host}");
        (host.trim_start_matches("www.").to_string(), origin)
    } else {
        (lower.clone(), lower)
    };
    without_proto
}

impl MemoryStore {
    /// Records a page navigation visit into history.
    pub fn record_visit(
        &self,
        profile_id: &str,
        url: &str,
        title: Option<&str>,
        transition: &str,
    ) -> Result<String> {
        self.ensure_profile(profile_id, "user", profile_id)?;
        let now = now_ms();
        let (domain, origin) = parse_domain_and_origin(url);
        let conn = self.conn();

        // 1. Ensure page exists or update last_seen_at
        let page_id: String = match conn.query_row(
            "SELECT id FROM pages WHERE url = ?1",
            params![url],
            |r| r.get(0),
        ) {
            Ok(existing_id) => {
                conn.execute(
                    "UPDATE pages SET last_seen_at = ?1 WHERE id = ?2",
                    params![now, existing_id],
                )?;
                existing_id
            }
            Err(_) => {
                let pid = new_id();
                conn.execute(
                    "INSERT INTO pages (id, url, origin, domain, first_seen_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![pid, url, origin, domain, now, now],
                )?;
                pid
            }
        };

        // 2. Insert into visits
        let visit_id = new_id();
        let transition_str = if transition.is_empty() { "link" } else { transition };

        conn.execute(
            "INSERT INTO visits (id, profile_id, page_id, transition, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![visit_id, profile_id, page_id, transition_str, now],
        )?;

        // If title provided, optionally update latest page_version title if one exists
        if let Some(t) = title {
            let _ = conn.execute(
                "UPDATE page_versions SET title = ?1
                 WHERE page_id = ?2 AND title IS NULL",
                params![t, page_id],
            );
        }

        Ok(visit_id)
    }

    /// List browsing history with optional keyword search.
    pub fn list_history(
        &self,
        profile_id: &str,
        limit: usize,
        offset: usize,
        search: Option<&str>,
    ) -> Result<Vec<HistoryVisitRecord>> {
        let conn = self.conn();
        let mut list = Vec::new();

        if let Some(q) = search {
            let pattern = format!("%{}%", q.trim());
            let mut stmt = conn.prepare(
                "SELECT v.id, v.page_id, p.url, p.domain, pv.title, v.transition, v.started_at, v.dwell_ms
                 FROM visits v
                 JOIN pages p ON p.id = v.page_id
                 LEFT JOIN page_versions pv ON pv.page_id = p.id
                 WHERE v.profile_id = ?1 AND (p.url LIKE ?2 OR p.domain LIKE ?2 OR pv.title LIKE ?2)
                 ORDER BY v.started_at DESC
                 LIMIT ?3 OFFSET ?4"
            )?;
            let rows = stmt.query_map(params![profile_id, pattern, limit as i64, offset as i64], Self::map_history_row)?;
            for r in rows {
                list.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT v.id, v.page_id, p.url, p.domain, pv.title, v.transition, v.started_at, v.dwell_ms
                 FROM visits v
                 JOIN pages p ON p.id = v.page_id
                 LEFT JOIN page_versions pv ON pv.page_id = p.id
                 WHERE v.profile_id = ?1
                 ORDER BY v.started_at DESC
                 LIMIT ?2 OFFSET ?3"
            )?;
            let rows = stmt.query_map(params![profile_id, limit as i64, offset as i64], Self::map_history_row)?;
            for r in rows {
                list.push(r?);
            }
        }

        Ok(list)
    }

    /// Delete a single visit by its ID.
    pub fn delete_history_visit(&self, visit_id: &str) -> Result<bool> {
        let affected = self.conn().execute("DELETE FROM visits WHERE id = ?1", params![visit_id])?;
        Ok(affected > 0)
    }

    /// Clear browsing history for a profile (optionally only since a given timestamp).
    pub fn clear_history(&self, profile_id: &str, since_ms: Option<i64>) -> Result<usize> {
        let affected = if let Some(cutoff) = since_ms {
            self.conn().execute(
                "DELETE FROM visits WHERE profile_id = ?1 AND started_at >= ?2",
                params![profile_id, cutoff],
            )?
        } else {
            self.conn().execute(
                "DELETE FROM visits WHERE profile_id = ?1",
                params![profile_id],
            )?
        };
        Ok(affected)
    }

    /// Retrieve the most frequently visited domains.
    pub fn top_history_domains(&self, profile_id: &str, limit: usize) -> Result<Vec<(String, usize)>> {
        let mut stmt = self.conn().prepare(
            "SELECT p.domain, COUNT(v.id) as visit_count
             FROM visits v
             JOIN pages p ON p.id = v.page_id
             WHERE v.profile_id = ?1
             GROUP BY p.domain
             ORDER BY visit_count DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![profile_id, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn map_history_row(row: &rusqlite::Row) -> rusqlite::Result<HistoryVisitRecord> {
        Ok(HistoryVisitRecord {
            id: row.get(0)?,
            page_id: row.get(1)?,
            url: row.get(2)?,
            domain: row.get(3)?,
            title: row.get(4)?,
            transition: row.get(5)?,
            started_at: row.get(6)?,
            dwell_ms: row.get(7)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> MemoryStore {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("p_test", "user", "default").unwrap();
        s
    }

    #[test]
    fn records_lists_and_searches_history() {
        let s = test_store();
        let v1 = s.record_visit("p_test", "https://rust-lang.org", Some("Rust Language"), "typed").unwrap();
        let v2 = s.record_visit("p_test", "https://github.com/rust-lang", Some("GitHub Rust"), "link").unwrap();

        let all = s.list_history("p_test", 10, 0, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, v2);
        assert_eq!(all[1].id, v1);

        // Search
        let filtered = s.list_history("p_test", 10, 0, Some("github")).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].domain, "github.com");

        // Top domains
        let top = s.top_history_domains("p_test", 5).unwrap();
        assert_eq!(top.len(), 2);

        // Delete single
        assert!(s.delete_history_visit(&v2).unwrap());
        assert_eq!(s.list_history("p_test", 10, 0, None).unwrap().len(), 1);

        // Clear all
        let cleared = s.clear_history("p_test", None).unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(s.list_history("p_test", 10, 0, None).unwrap().len(), 0);
    }
}
