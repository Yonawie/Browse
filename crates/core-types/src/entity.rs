//! Knowledge graph entity and relation types shared across crates.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub normalized: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityMentionRecord {
    pub entity_id: String,
    pub chunk_id: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub id: String,
    pub src_id: String,
    pub dst_id: String,
    pub relation: String,
    pub weight: f32,
    pub evidence_chunk_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub kind: String,
    pub name: String,
    pub normalized: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub src_normalized: String,
    pub dst_normalized: String,
    pub relation: String,
    pub weight: f32,
}
