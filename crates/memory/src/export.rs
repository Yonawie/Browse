use crate::{MemoryStore, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Obsidian,
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedDocument {
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryExportReport {
    pub format: ExportFormat,
    pub total_pages: usize,
    pub total_entities: usize,
    pub documents: Vec<ExportedDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryImportReport {
    pub imported_bookmarks: usize,
    pub imported_reading_items: usize,
    pub total_items: usize,
}

impl MemoryStore {
    /// Export the memory store into portable documents (Obsidian markdown vault or JSON).
    pub fn export_memory(&self, format: ExportFormat) -> Result<MemoryExportReport> {
        match format {
            ExportFormat::Obsidian => self.export_obsidian_vault(),
            ExportFormat::Markdown => self.export_combined_markdown(),
            ExportFormat::Json => self.export_json(),
        }
    }

    /// Export the memory store directly into a folder on disk.
    pub fn export_to_directory(
        &self,
        dir_path: impl AsRef<Path>,
        format: ExportFormat,
    ) -> Result<MemoryExportReport> {
        let report = self.export_memory(format)?;
        let base_dir = dir_path.as_ref();
        fs::create_dir_all(base_dir).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        for doc in &report.documents {
            let file_path = base_dir.join(&doc.filename);
            if let Some(parent) = file_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(file_path, &doc.content)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }

        Ok(report)
    }

    fn export_obsidian_vault(&self) -> Result<MemoryExportReport> {
        let mut stmt = self.conn().prepare(
            "SELECT p.id, p.url, p.domain, pv.title, pv.page_kind, pv.captured_at, pv.main_text_zst
             FROM pages p
             JOIN page_versions pv ON pv.page_id = p.id
             WHERE p.never_remember = 0
             GROUP BY p.id
             ORDER BY pv.captured_at DESC",
        )?;

        let mut docs = Vec::new();
        let mut page_summaries = Vec::new();

        let rows = stmt.query_map([], |r| {
            let page_id: String = r.get(0)?;
            let url: String = r.get(1)?;
            let domain: String = r.get(2)?;
            let title: Option<String> = r.get(3)?;
            let kind: String = r.get(4)?;
            let captured_at: i64 = r.get(5)?;
            let text_blob: Option<Vec<u8>> = r.get(6)?;
            let main_text = text_blob.map(|b| String::from_utf8_lossy(&b).to_string());
            Ok((page_id, url, domain, title, kind, captured_at, main_text))
        })?;

        let mut page_count = 0;
        for row in rows {
            let (page_id, url, domain, title, kind, captured_at, main_text) = row?;
            page_count += 1;

            let safe_title = title.as_deref().unwrap_or("Untitled Page");
            let filename = format!("{}_{}.md", sanitize_filename(&domain), sanitize_filename(safe_title));

            // Fetch entities associated with chunks of this page
            let entities = self.get_entities_for_page(&page_id)?;

            let mut md = String::new();
            md.push_str("---\n");
            md.push_str(&format!("url: \"{}\"\n", url));
            md.push_str(&format!("title: \"{}\"\n", safe_title));
            md.push_str(&format!("domain: \"{}\"\n", domain));
            md.push_str(&format!("kind: \"{}\"\n", kind));
            md.push_str(&format!("captured_at: {}\n", captured_at));
            md.push_str("tags:\n  - browse-memory\n");
            md.push_str("---\n\n");

            md.push_str(&format!("# {}\n\n", safe_title));
            if let Some(text) = main_text {
                md.push_str(&text);
                md.push_str("\n\n");
            }

            if !entities.is_empty() {
                md.push_str("## Linked Entities\n");
                for (name, kind) in &entities {
                    md.push_str(&format!("- [[{}]]: {}\n", name, kind));
                }
                md.push('\n');
            }

            page_summaries.push((safe_title.to_string(), filename.clone(), domain.clone()));
            docs.push(ExportedDocument { filename, content: md });
        }

        // Generate vault index note
        let top_entities = self.list_top_entities(20).unwrap_or_default();
        let entity_count = top_entities.len();

        let mut index_md = String::new();
        index_md.push_str("# Browse Memory Index\n\n");
        index_md.push_str(&format!("Indexed Pages: {}\n", page_count));
        index_md.push_str(&format!("Tracked Entities: {}\n\n", entity_count));

        if !top_entities.is_empty() {
            index_md.push_str("## Knowledge Graph (Top Entities)\n");
            for (_id, kind, name, mentions) in &top_entities {
                index_md.push_str(&format!("- [[{}]]: {} ({} mentions)\n", name, kind, mentions));
            }
            index_md.push('\n');
        }

        index_md.push_str("## Pages\n");
        for (title, filename, domain) in page_summaries {
            index_md.push_str(&format!("- [[{}|{}]] ({})\n", filename.trim_end_matches(".md"), title, domain));
        }

        docs.insert(
            0,
            ExportedDocument {
                filename: "_index.md".to_string(),
                content: index_md,
            },
        );

        Ok(MemoryExportReport {
            format: ExportFormat::Obsidian,
            total_pages: page_count,
            total_entities: entity_count,
            documents: docs,
        })
    }

    fn export_combined_markdown(&self) -> Result<MemoryExportReport> {
        let vault = self.export_obsidian_vault()?;
        let mut combined = String::new();
        combined.push_str("# Browse Memory Export\n\n");

        for doc in &vault.documents {
            if doc.filename == "_index.md" {
                continue;
            }
            combined.push_str(&doc.content);
            combined.push_str("\n\n---\n\n");
        }

        Ok(MemoryExportReport {
            format: ExportFormat::Markdown,
            total_pages: vault.total_pages,
            total_entities: vault.total_entities,
            documents: vec![ExportedDocument {
                filename: "browse_memory_export.md".to_string(),
                content: combined,
            }],
        })
    }

    fn export_json(&self) -> Result<MemoryExportReport> {
        let mut stmt = self.conn().prepare(
            "SELECT p.id, p.url, p.domain, pv.title, pv.page_kind, pv.captured_at
             FROM pages p
             JOIN page_versions pv ON pv.page_id = p.id
             WHERE p.never_remember = 0
             GROUP BY p.id",
        )?;

        let pages_json: Vec<serde_json::Value> = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?,
                    "url": r.get::<_, String>(1)?,
                    "domain": r.get::<_, String>(2)?,
                    "title": r.get::<_, Option<String>>(3)?,
                    "kind": r.get::<_, String>(4)?,
                    "captured_at": r.get::<_, i64>(5)?,
                }))
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        let top_entities = self.list_top_entities(100).unwrap_or_default();
        let entities_json: Vec<serde_json::Value> = top_entities
            .into_iter()
            .map(|(id, kind, name, mentions)| {
                serde_json::json!({
                    "id": id,
                    "kind": kind,
                    "name": name,
                    "mentions": mentions
                })
            })
            .collect();

        let bookmarks = self.list_bookmarks(None).unwrap_or_default();
        let reading_list = self.list_reading_items("default", false).unwrap_or_default();
        let downloads = self.list_downloads("default", 100).unwrap_or_default();

        let export_json = serde_json::json!({
            "schema_version": crate::SCHEMA_VERSION,
            "exported_at": crate::now_ms(),
            "pages": pages_json,
            "entities": entities_json,
            "bookmarks": bookmarks,
            "reading_list": reading_list,
            "downloads": downloads,
        });

        let json_str = serde_json::to_string_pretty(&export_json)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let total_pages = pages_json.len();
        let total_entities = entities_json.len();

        Ok(MemoryExportReport {
            format: ExportFormat::Json,
            total_pages,
            total_entities,
            documents: vec![ExportedDocument {
                filename: "browse_memory_export.json".to_string(),
                content: json_str,
            }],
        })
    }

    fn get_entities_for_page(&self, page_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT e.name, e.kind
             FROM entity_mentions em
             JOIN entities e ON e.id = em.entity_id
             JOIN chunks c ON c.id = em.chunk_id
             JOIN page_versions pv ON pv.id = c.page_version_id
             WHERE pv.page_id = ?1
             ORDER BY em.confidence DESC
             LIMIT 10",
        )?;

        let rows = stmt.query_map(params![page_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Import bookmarks, reading list items, and metadata from exported JSON string.
    pub fn import_memory_json(&self, profile_id: &str, json_str: &str) -> Result<MemoryImportReport> {
        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let mut imported_bookmarks = 0;
        let mut imported_reading_items = 0;

        if let Some(bms) = parsed.get("bookmarks").and_then(serde_json::Value::as_array) {
            for b in bms {
                if let (Some(url), Some(title)) = (
                    b.get("url").and_then(serde_json::Value::as_str),
                    b.get("title").and_then(serde_json::Value::as_str),
                ) {
                    let folder = b.get("folder").and_then(serde_json::Value::as_str).map(ToString::to_string);
                    let favicon_url = b.get("favicon_url").and_then(serde_json::Value::as_str).map(ToString::to_string);
                    let tags: Vec<String> = b.get("tags")
                        .and_then(serde_json::Value::as_array)
                        .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).map(ToString::to_string).collect())
                        .unwrap_or_default();

                    let new_bm = crate::NewBookmark {
                        url: url.to_string(),
                        title: title.to_string(),
                        folder,
                        favicon_url,
                        tags,
                    };
                    if self.add_bookmark(&new_bm).is_ok() {
                        imported_bookmarks += 1;
                    }
                }
            }
        }

        if let Some(items) = parsed.get("reading_list").and_then(serde_json::Value::as_array) {
            for it in items {
                if let (Some(url), Some(title)) = (
                    it.get("url").and_then(serde_json::Value::as_str),
                    it.get("title").and_then(serde_json::Value::as_str),
                ) {
                    let excerpt = it.get("excerpt").and_then(serde_json::Value::as_str).unwrap_or("");
                    let reading_time = it.get("reading_time_min").and_then(serde_json::Value::as_u64).unwrap_or(3) as u32;
                    let is_read = it.get("is_read").and_then(serde_json::Value::as_bool).unwrap_or(false);

                    if let Ok(rec) = self.add_reading_item(None, profile_id, url, title, excerpt, reading_time) {
                        if is_read {
                            let _ = self.toggle_reading_item_read(&rec.id, true);
                        }
                        imported_reading_items += 1;
                    }
                }
            }
        }

        Ok(MemoryImportReport {
            imported_bookmarks,
            imported_reading_items,
            total_items: imported_bookmarks + imported_reading_items,
        })
    }
}

fn sanitize_filename(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '.' {
            out.push('_');
        }
    }
    if out.is_empty() {
        "doc".to_string()
    } else if out.chars().count() > 40 {
        out.chars().take(40).collect()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewPageVersion;
    use core_types::{PageKind, Sensitivity};

    fn test_store() -> MemoryStore {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("p", "user", "default").unwrap();
        s
    }

    #[test]
    fn exports_obsidian_vault_with_wikilinks_and_index() {
        let s = test_store();
        let v1 = NewPageVersion {
            url: "https://doc.rust-lang.org/book",
            title: Some("The Rust Book"),
            lang: Some("en"),
            page_kind: PageKind::Doc,
            sensitivity: Sensitivity::Public,
            main_text: Some("Rust is a systems programming language focused on safety."),
            content_hash: "hash-rust-book",
        };
        s.insert_page_version(&v1).unwrap();

        let report = s.export_memory(ExportFormat::Obsidian).unwrap();
        assert_eq!(report.total_pages, 1);
        assert!(!report.documents.is_empty());

        let index_doc = report.documents.iter().find(|d| d.filename == "_index.md").unwrap();
        assert!(index_doc.content.contains("# Browse Memory Index"));
        assert!(index_doc.content.contains("The Rust Book"));

        let page_doc = report.documents.iter().find(|d| d.filename.contains("rust")).unwrap();
        assert!(page_doc.content.contains("url: \"https://doc.rust-lang.org/book\""));
        assert!(page_doc.content.contains("Rust is a systems programming language"));
    }

    #[test]
    fn exports_json_format() {
        let s = test_store();
        let v = NewPageVersion {
            url: "https://example.com",
            title: Some("Example"),
            lang: Some("en"),
            page_kind: PageKind::Doc,
            sensitivity: Sensitivity::Public,
            main_text: Some("Text"),
            content_hash: "hash-ex",
        };
        s.insert_page_version(&v).unwrap();

        let report = s.export_memory(ExportFormat::Json).unwrap();
        assert_eq!(report.total_pages, 1);
        assert_eq!(report.documents.len(), 1);
        assert!(report.documents[0].content.contains("\"url\": \"https://example.com\""));
        assert!(report.documents[0].content.contains("\"bookmarks\":"));
        assert!(report.documents[0].content.contains("\"reading_list\":"));
        assert!(report.documents[0].content.contains("\"downloads\":"));
    }

    #[test]
    fn imports_memory_json() {
        let s = test_store();

        let json_data = r#"{
            "schema_version": 1,
            "exported_at": 1700000000000,
            "bookmarks": [
                {
                    "url": "https://crates.io",
                    "title": "Rust Package Registry",
                    "folder": "Dev",
                    "tags": ["rust", "cargo"]
                }
            ],
            "reading_list": [
                {
                    "url": "https://blog.rust-lang.org",
                    "title": "Rust Blog",
                    "excerpt": "Announcements and releases",
                    "reading_time_min": 5,
                    "is_read": true
                }
            ]
        }"#;

        let res = s.import_memory_json("default", json_data).unwrap();
        assert_eq!(res.imported_bookmarks, 1);
        assert_eq!(res.imported_reading_items, 1);
        assert_eq!(res.total_items, 2);

        let bms = s.list_bookmarks(Some("Dev")).unwrap();
        assert_eq!(bms.len(), 1);
        assert_eq!(bms[0].title, "Rust Package Registry");

        let rl = s.list_reading_items("default", false).unwrap();
        assert_eq!(rl.len(), 1);
        assert_eq!(rl[0].title, "Rust Blog");
        assert!(rl[0].is_read);
    }
}
