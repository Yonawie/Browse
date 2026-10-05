//! Memory layer (ADR-004): one SQLite database for history, pages, chunks,
//! vectors, knowledge graph, agent sessions and audit.
//!
//! The schema lives in `schema/memory.sql` and is embedded at compile time.
//! Vector search uses `sqlite-vec` when the feature is enabled and the
//! extension is available; otherwise a brute-force cosine scan over
//! `chunk_vectors_raw` (fine for tests and small profiles).

pub mod bookmarks;
pub mod export;
pub mod grouping;
pub mod journal;
pub mod search;
pub mod store;

pub use bookmarks::{BookmarkRecord, NewBookmark};
pub use export::{ExportFormat, ExportedDocument, MemoryExportReport};
pub use grouping::{cluster_tabs, AutoTabGroup, TabForClustering};
pub use journal::{JournalEntry, ModelCallRecord, ModelUsageRow};
pub use search::{SearchFilters, SearchHit};
pub use store::{
    auto_cluster_tab, MemoryStore, NewChunk, NewPageVersion, PruneCandidate, StoredPageVersion, TabGroupRecord,
    TabRecord, TaskRecord,
};

pub const SCHEMA_SQL: &str = include_str!("../../../schema/memory.sql");
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("memory provenance must be `user` or `confirmed` (anti-poisoning)")]
    ForbiddenProvenance,
    #[error("secret data is never persisted")]
    SecretNotPersistable,
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
}

pub type Result<T> = std::result::Result<T, MemoryError>;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
