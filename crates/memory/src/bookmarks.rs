use crate::{new_id, now_ms, MemoryStore, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Stored Bookmark record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkRecord {
    pub id: String,
    pub url: String,
    pub title: String,
    pub folder: String,
    pub favicon_url: Option<String>,
    pub tags: Vec<String>,
    pub created_at: i64,
}

/// Parameters for creating a new bookmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewBookmark {
    pub url: String,
    pub title: String,
    pub folder: Option<String>,
    pub favicon_url: Option<String>,
    pub tags: Vec<String>,
}

impl MemoryStore {
    /// Ensure the bookmarks schema table exists.
    pub fn ensure_bookmarks_schema(&self) -> Result<()> {
        self.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS bookmarks (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                folder TEXT NOT NULL DEFAULT 'Bookmarks Bar',
                favicon_url TEXT,
                tags TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_bookmarks_folder ON bookmarks(folder);
            CREATE INDEX IF NOT EXISTS idx_bookmarks_created ON bookmarks(created_at DESC);"
        )?;
        Ok(())
    }

    /// Add a new bookmark.
    pub fn add_bookmark(&self, b: &NewBookmark) -> Result<String> {
        self.ensure_bookmarks_schema()?;
        let id = new_id();
        let folder = b.folder.as_deref().unwrap_or("Bookmarks Bar");
        let tags_str = b.tags.join(",");
        let now = now_ms();

        self.conn().execute(
            "INSERT INTO bookmarks (id, url, title, folder, favicon_url, tags, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, b.url, b.title, folder, b.favicon_url, tags_str, now],
        )?;

        Ok(id)
    }

    /// List bookmarks, optionally filtered by folder.
    pub fn list_bookmarks(&self, folder: Option<&str>) -> Result<Vec<BookmarkRecord>> {
        self.ensure_bookmarks_schema()?;
        let conn = self.conn();
        let mut list = Vec::new();

        if let Some(f) = folder {
            let mut stmt = conn.prepare(
                "SELECT id, url, title, folder, favicon_url, tags, created_at
                 FROM bookmarks WHERE folder = ?1 ORDER BY created_at DESC"
            )?;
            let rows = stmt.query_map(params![f], Self::map_bookmark_row)?;
            for r in rows {
                list.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, url, title, folder, favicon_url, tags, created_at
                 FROM bookmarks ORDER BY folder ASC, created_at DESC"
            )?;
            let rows = stmt.query_map([], Self::map_bookmark_row)?;
            for r in rows {
                list.push(r?);
            }
        }

        Ok(list)
    }

    /// Search bookmarks by title or URL.
    pub fn search_bookmarks(&self, query: &str) -> Result<Vec<BookmarkRecord>> {
        self.ensure_bookmarks_schema()?;
        let pattern = format!("%{}%", query.trim());
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, url, title, folder, favicon_url, tags, created_at
             FROM bookmarks
             WHERE title LIKE ?1 OR url LIKE ?1 OR tags LIKE ?1
             ORDER BY created_at DESC LIMIT 50"
        )?;
        let rows = stmt.query_map(params![pattern], Self::map_bookmark_row)?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Delete a bookmark by ID.
    pub fn delete_bookmark(&self, id: &str) -> Result<bool> {
        self.ensure_bookmarks_schema()?;
        let affected = self.conn().execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    fn map_bookmark_row(row: &rusqlite::Row) -> rusqlite::Result<BookmarkRecord> {
        let tags_str: Option<String> = row.get(5)?;
        let tags = tags_str
            .map(|s| s.split(',').filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string()).collect())
            .unwrap_or_default();

        Ok(BookmarkRecord {
            id: row.get(0)?,
            url: row.get(1)?,
            title: row.get(2)?,
            folder: row.get(3)?,
            favicon_url: row.get(4)?,
            tags,
            created_at: row.get(6)?,
        })
    }

    /// Import bookmarks from a standard Netscape Bookmark HTML file (Chrome, Firefox, Edge, Safari export format).
    pub fn import_netscape_bookmarks(&self, html: &str) -> Result<usize> {
        self.ensure_bookmarks_schema()?;
        let mut imported = 0;
        let mut current_folder = "Bookmarks Bar".to_string();

        for line in html.lines() {
            let trimmed = line.trim();

            // Detect Folder: <H3 ...>Folder Name</H3>
            if let Some(start_idx) = trimmed.find("<H3") {
                if let Some(close_tag) = trimmed[start_idx..].find('>') {
                    let text_start = start_idx + close_tag + 1;
                    if let Some(end_idx) = trimmed[text_start..].find("</H3>") {
                        let folder_name = &trimmed[text_start..text_start + end_idx];
                        if !folder_name.trim().is_empty() {
                            current_folder = folder_name.trim().to_string();
                        }
                    }
                }
            }

            // Detect Bookmark: <A HREF="url" ...>Title</A>
            if let Some(href_idx) = trimmed.find("HREF=\"") {
                let url_start = href_idx + 6;
                if let Some(quote_end) = trimmed[url_start..].find('"') {
                    let url = &trimmed[url_start..url_start + quote_end];

                    // Find title
                    let tag_close = trimmed[url_start + quote_end..].find('>');
                    let title = if let Some(tc) = tag_close {
                        let text_start = url_start + quote_end + tc + 1;
                        if let Some(a_end) = trimmed[text_start..].find("</A>") {
                            trimmed[text_start..text_start + a_end].trim()
                        } else {
                            url
                        }
                    } else {
                        url
                    };

                    if url.starts_with("http://") || url.starts_with("https://") {
                        let _ = self.add_bookmark(&NewBookmark {
                            url: url.to_string(),
                            title: if title.is_empty() { url.to_string() } else { title.to_string() },
                            folder: Some(current_folder.clone()),
                            favicon_url: None,
                            tags: Vec::new(),
                        });
                        imported += 1;
                    }
                }
            }
        }

        Ok(imported)
    }

    /// Export bookmarks to standard Netscape Bookmark HTML format.
    pub fn export_netscape_bookmarks(&self) -> Result<String> {
        let bookmarks = self.list_bookmarks(None)?;
        let mut out = String::new();
        out.push_str("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n");
        out.push_str("<!-- This is an automatically generated file. It will be read and overwritten. Do Not Edit! -->\n");
        out.push_str("<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n");
        out.push_str("<TITLE>Bookmarks</TITLE>\n");
        out.push_str("<H1>Bookmarks</H1>\n");
        out.push_str("<DL><p>\n");

        // Group by folder
        let mut by_folder: std::collections::BTreeMap<String, Vec<&BookmarkRecord>> = std::collections::BTreeMap::new();
        for b in &bookmarks {
            by_folder.entry(b.folder.clone()).or_default().push(b);
        }

        for (folder, items) in by_folder {
            out.push_str(&format!("    <DT><H3>{folder}</H3>\n"));
            out.push_str("    <DL><p>\n");
            for b in items {
                let title = if b.title.is_empty() { &b.url } else { &b.title };
                out.push_str(&format!(
                    "        <DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>\n",
                    b.url,
                    b.created_at / 1000,
                    html_escape(title)
                ));
            }
            out.push_str("    </DL><p>\n");
        }

        out.push_str("</DL><p>\n");
        Ok(out)
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> MemoryStore {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("p", "user", "default").unwrap();
        s
    }

    #[test]
    fn creates_lists_and_searches_bookmarks() {
        let s = test_store();
        let b1 = NewBookmark {
            url: "https://doc.rust-lang.org".into(),
            title: "Rust Documentation".into(),
            folder: Some("Programming".into()),
            favicon_url: None,
            tags: vec!["rust".into(), "docs".into()],
        };
        let id1 = s.add_bookmark(&b1).unwrap();

        let list = s.list_bookmarks(Some("Programming")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id1);
        assert_eq!(list[0].title, "Rust Documentation");
        assert_eq!(list[0].tags, vec!["rust", "docs"]);

        // Search
        let hits = s.search_bookmarks("rust").unwrap();
        assert_eq!(hits.len(), 1);

        // Delete
        assert!(s.delete_bookmark(&id1).unwrap());
        assert_eq!(s.list_bookmarks(Some("Programming")).unwrap().len(), 0);
    }

    #[test]
    fn imports_and_exports_netscape_html() {
        let s = test_store();
        let netscape_html = r#"
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>Dev</H3>
    <DL><p>
        <DT><A HREF="https://github.com" ADD_DATE="1600000000">GitHub</A>
        <DT><A HREF="https://crates.io" ADD_DATE="1600000001">Crates</A>
    </DL><p>
</DL><p>
"#;

        let imported_count = s.import_netscape_bookmarks(netscape_html).unwrap();
        assert_eq!(imported_count, 2);

        let dev_bookmarks = s.list_bookmarks(Some("Dev")).unwrap();
        assert_eq!(dev_bookmarks.len(), 2);

        let exported = s.export_netscape_bookmarks().unwrap();
        assert!(exported.contains("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
        assert!(exported.contains("<H3>Dev</H3>"));
        assert!(exported.contains("https://github.com"));
        assert!(exported.contains("GitHub"));
    }
}
