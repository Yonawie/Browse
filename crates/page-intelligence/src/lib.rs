//! Page Intelligence: turns sensor output into chunks, page kinds and
//! budgeted observations. Deterministic; models are called by the layers above
//! through Model Gateway.

pub mod budget;
pub mod chunker;
pub mod citations;
pub mod dark_patterns;
pub mod entities;
pub mod injection;
pub mod page_kind;
pub mod phishing;

pub use budget::{trim_observation, ObservationBudget};
pub use chunker::{chunk_text, Chunk, ChunkerConfig};
pub use citations::{extract_citation_ids, format_citation_report, verify_citations, CitationMatch, CitationReport};
pub use core_types::{ExtractedEntity, ExtractedRelation};
pub use dark_patterns::{detect_dark_patterns, DarkPatternCategory, DarkPatternFinding};
pub use entities::{extract_entities_and_relations, KnowledgeGraphExtraction};
pub use injection::injection_signal;
pub use page_kind::detect_page_kind;
pub use phishing::{inspect_url_phishing, PhishingReport, PhishingSeverity};

/// Rough token estimate. Local tokenizers vary; 4 chars/token is a safe upper
/// bound for Latin scripts, ~2.5 for Cyrillic/CJK. We use a blended estimate.
pub fn approx_tokens(text: &str) -> u32 {
    let mut latin = 0usize;
    let mut other = 0usize;
    for c in text.chars() {
        if c.is_ascii() {
            latin += 1;
        } else {
            other += 1;
        }
    }
    ((latin as f32 / 4.0) + (other as f32 / 2.5)).ceil() as u32
}

/// Content hash used for `page_versions.content_hash` and observation diffs.
pub fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}
