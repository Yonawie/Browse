use crate::{new_id, now_ms, MemoryStore, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Status of a file download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Quarantined,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Quarantined => "quarantined",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "quarantined" => Self::Quarantined,
            _ => Self::InProgress,
        }
    }
}

/// Security classification for downloaded files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadDangerLevel {
    Safe,
    Suspicious,
    Dangerous,
}

impl DownloadDangerLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Suspicious => "suspicious",
            Self::Dangerous => "dangerous",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "dangerous" => Self::Dangerous,
            "suspicious" => Self::Suspicious,
            _ => Self::Safe,
        }
    }
}

/// A recorded download entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: String,
    pub profile_id: String,
    pub url: String,
    pub filename: String,
    pub target_path: String,
    pub total_bytes: Option<i64>,
    pub downloaded_bytes: i64,
    pub sha256: Option<String>,
    pub mime_type: Option<String>,
    pub status: DownloadStatus,
    pub danger_level: DownloadDangerLevel,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

/// Input payload to register a new download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewDownload {
    pub url: String,
    pub filename: String,
    pub target_path: String,
    pub total_bytes: Option<i64>,
    pub mime_type: Option<String>,
}

/// Analyzes filename and URL to detect executables, scripts, or double-extension spoofing.
pub fn assess_download_danger(filename: &str, _url: &str) -> DownloadDangerLevel {
    let lower = filename.to_lowercase();

    // High danger executable / script extensions
    let dangerous_exts = [
        ".exe", ".scr", ".bat", ".cmd", ".ps1", ".vbs", ".msi", ".reg", ".hta", ".wsf", ".cpl",
    ];
    for ext in dangerous_exts {
        if lower.ends_with(ext) {
            return DownloadDangerLevel::Dangerous;
        }
    }

    // Double extension spoofing (e.g., photo.jpg.exe or invoice.pdf.js)
    let parts: Vec<&str> = lower.split('.').collect();
    if parts.len() > 2 {
        let last = parts.last().unwrap_or(&"");
        let second_last = parts.get(parts.len() - 2).unwrap_or(&"");
        let common_doc_exts = ["pdf", "doc", "docx", "xls", "xlsx", "jpg", "jpeg", "png", "txt"];
        let script_exts = ["js", "vbs", "ps1", "bat", "cmd", "hta", "exe", "scr"];
        if common_doc_exts.contains(second_last) && script_exts.contains(last) {
            return DownloadDangerLevel::Dangerous;
        }
    }

    // Suspicious archive / container formats
    let suspicious_exts = [".iso", ".img", ".jar", ".apk", ".bin", ".dll", ".sys"];
    for ext in suspicious_exts {
        if lower.ends_with(ext) {
            return DownloadDangerLevel::Suspicious;
        }
    }

    DownloadDangerLevel::Safe
}

impl MemoryStore {
    /// Ensure the downloads schema table exists.
    pub fn ensure_downloads_schema(&self) -> Result<()> {
        self.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS downloads (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                filename TEXT NOT NULL,
                target_path TEXT NOT NULL,
                total_bytes INTEGER,
                downloaded_bytes INTEGER NOT NULL DEFAULT 0,
                sha256 TEXT,
                mime_type TEXT,
                status TEXT NOT NULL,
                danger_level TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_downloads_profile ON downloads(profile_id, started_at DESC);",
        )?;
        Ok(())
    }

    /// Record a newly started download.
    pub fn record_download(&self, profile_id: &str, d: &NewDownload) -> Result<String> {
        self.ensure_profile(profile_id, "user", profile_id)?;
        self.ensure_downloads_schema()?;
        let id = new_id();
        let now = now_ms();
        let danger = assess_download_danger(&d.filename, &d.url);

        self.conn().execute(
            "INSERT INTO downloads (id, profile_id, url, filename, target_path, total_bytes, downloaded_bytes, sha256, mime_type, status, danger_level, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11, NULL)",
            params![
                id,
                profile_id,
                d.url,
                d.filename,
                d.target_path,
                d.total_bytes,
                0i64,
                d.mime_type,
                DownloadStatus::InProgress.as_str(),
                danger.as_str(),
                now
            ],
        )?;

        Ok(id)
    }

    /// Update progress, completion status, or verification hash of an active download.
    pub fn update_download_progress(
        &self,
        id: &str,
        downloaded_bytes: i64,
        status: DownloadStatus,
        sha256: Option<&str>,
    ) -> Result<bool> {
        self.ensure_downloads_schema()?;
        let now = now_ms();
        let finished_at = if status == DownloadStatus::InProgress {
            None
        } else {
            Some(now)
        };

        let updated = self.conn().execute(
            "UPDATE downloads
             SET downloaded_bytes = ?1,
                 status = ?2,
                 sha256 = COALESCE(?3, sha256),
                 finished_at = COALESCE(?4, finished_at)
             WHERE id = ?5",
            params![downloaded_bytes, status.as_str(), sha256, finished_at, id],
        )?;

        Ok(updated > 0)
    }

    /// List downloads for a profile ordered from most recent to oldest.
    pub fn list_downloads(&self, profile_id: &str, limit: usize) -> Result<Vec<DownloadRecord>> {
        self.ensure_downloads_schema()?;
        let mut stmt = self.conn().prepare(
            "SELECT id, profile_id, url, filename, target_path, total_bytes, downloaded_bytes, sha256, mime_type, status, danger_level, started_at, finished_at
             FROM downloads
             WHERE profile_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![profile_id, limit as i64], |row| {
            let status_str: String = row.get(9)?;
            let danger_str: String = row.get(10)?;
            Ok(DownloadRecord {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                url: row.get(2)?,
                filename: row.get(3)?,
                target_path: row.get(4)?,
                total_bytes: row.get(5)?,
                downloaded_bytes: row.get(6)?,
                sha256: row.get(7)?,
                mime_type: row.get(8)?,
                status: DownloadStatus::parse_str(&status_str),
                danger_level: DownloadDangerLevel::parse_str(&danger_str),
                started_at: row.get(11)?,
                finished_at: row.get(12)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Delete a single download entry from history.
    pub fn delete_download(&self, id: &str) -> Result<bool> {
        self.ensure_downloads_schema()?;
        let count = self.conn().execute("DELETE FROM downloads WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    /// Clear all download records for a given profile.
    pub fn clear_downloads(&self, profile_id: &str) -> Result<usize> {
        self.ensure_downloads_schema()?;
        let count = self.conn().execute("DELETE FROM downloads WHERE profile_id = ?1", params![profile_id])?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn danger_assessment_detects_dangerous_and_double_extensions() {
        assert_eq!(
            assess_download_danger("installer.exe", "https://example.com"),
            DownloadDangerLevel::Dangerous
        );
        assert_eq!(
            assess_download_danger("invoice.pdf.vbs", "https://example.com"),
            DownloadDangerLevel::Dangerous
        );
        assert_eq!(
            assess_download_danger("setup.msi", "https://example.com"),
            DownloadDangerLevel::Dangerous
        );
        assert_eq!(
            assess_download_danger("disk.iso", "https://example.com"),
            DownloadDangerLevel::Suspicious
        );
        assert_eq!(
            assess_download_danger("report.pdf", "https://example.com"),
            DownloadDangerLevel::Safe
        );
        assert_eq!(
            assess_download_danger("photo.png", "https://example.com"),
            DownloadDangerLevel::Safe
        );
    }

    #[test]
    fn records_lists_and_updates_download_lifecycle() {
        let store = MemoryStore::open_in_memory().unwrap();

        let new_dl = NewDownload {
            url: "https://example.com/archive.tar.gz".to_string(),
            filename: "archive.tar.gz".to_string(),
            target_path: "/downloads/archive.tar.gz".to_string(),
            total_bytes: Some(1024 * 1024),
            mime_type: Some("application/gzip".to_string()),
        };

        let id = store.record_download("default", &new_dl).unwrap();
        let list = store.list_downloads("default", 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].status, DownloadStatus::InProgress);
        assert_eq!(list[0].danger_level, DownloadDangerLevel::Safe);

        // Update progress and completion
        let ok = store.update_download_progress(
            &id,
            1024 * 1024,
            DownloadStatus::Completed,
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        ).unwrap();
        assert!(ok);

        let list_updated = store.list_downloads("default", 10).unwrap();
        assert_eq!(list_updated[0].status, DownloadStatus::Completed);
        assert_eq!(list_updated[0].downloaded_bytes, 1024 * 1024);
        assert!(list_updated[0].finished_at.is_some());
        assert_eq!(
            list_updated[0].sha256.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );

        // Delete download
        let del = store.delete_download(&id).unwrap();
        assert!(del);
        assert!(store.list_downloads("default", 10).unwrap().is_empty());
    }
}
