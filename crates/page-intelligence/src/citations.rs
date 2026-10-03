//! Citation extraction, verification, and formatting for Page Intelligence answers.
//!
//! When the model answers questions, summarizes, or translates a page, it is instructed
//! to cite source chunks by ID (e.g. `[c0]`, `[c1]`). This module extracts those citations,
//! checks them against the actual `ContentChunk` list in the observation, detects hallucinated
//! or invalid source IDs, and extracts source snippet context for UI and audit logs.

use core_types::ContentChunk;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationMatch {
    pub obs_id: String,
    pub heading_path: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitationReport {
    /// Unique citation IDs referenced in the response (in order of appearance).
    pub cited_ids: Vec<String>,
    /// Citations that matched known chunks.
    pub verified: Vec<CitationMatch>,
    /// Citation IDs cited by the model that did not exist in the observation chunks (hallucinations).
    pub hallucinated: Vec<String>,
    /// Chunks available in the observation that were never cited.
    pub uncited_chunk_ids: Vec<String>,
    /// Ratio of valid citations to total citations (1.0 if all cited IDs exist; 0.0 if none cited).
    pub validity_ratio: f32,
}

/// Extract candidate citation IDs from text.
///
/// Looks for bracketed tokens like `[c0]`, `[c0, c1]`, `[c0][c1]`.
/// Skips markdown links where `]` is immediately followed by `(`.
pub fn extract_citation_ids(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'[' {
            let start = i + 1;
            let mut j = start;
            while j < len && bytes[j] != b']' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b']' {
                // Check if this is a markdown link [text](url)
                let is_markdown_link = j + 1 < len && bytes[j + 1] == b'(';
                if !is_markdown_link {
                    let inner = &text[start..j];
                    for raw_part in inner.split([',', ';', ' ']) {
                        let token = raw_part.trim().trim_start_matches(['#', '$']);
                        if is_potential_citation_id(token) && seen.insert(token.to_string()) {
                            results.push(token.to_string());
                        }
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }

    results
}

fn is_potential_citation_id(token: &str) -> bool {
    if token.is_empty() || token.len() > 32 {
        return false;
    }
    // Matches patterns like c0, c1, chunk_0, obs_1, or simple alphanumeric IDs
    let starts_with_valid_char = token.starts_with('c') || token.starts_with("obs") || token.starts_with("chunk");
    if starts_with_valid_char {
        return token.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    }
    // Or alphanumeric token ending with digits (e.g. s1, p2)
    token.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && token.chars().any(|c| c.is_ascii_digit())
}

/// Verify citations in the model answer against the supplied chunks.
pub fn verify_citations(answer: &str, chunks: &[ContentChunk]) -> CitationReport {
    let chunk_map: HashMap<&str, &ContentChunk> =
        chunks.iter().map(|c| (c.obs_id.as_str(), c)).collect();

    let cited_ids = extract_citation_ids(answer);
    let mut verified = Vec::new();
    let mut hallucinated = Vec::new();
    let mut cited_set = HashSet::new();

    for id in &cited_ids {
        if let Some(chunk) = chunk_map.get(id.as_str()) {
            cited_set.insert(id.clone());
            verified.push(CitationMatch {
                obs_id: id.clone(),
                heading_path: chunk.heading_path.clone(),
                snippet: make_snippet(&chunk.text, 120),
            });
        } else {
            hallucinated.push(id.clone());
        }
    }

    let uncited_chunk_ids: Vec<String> = chunks
        .iter()
        .filter(|c| !cited_set.contains(c.obs_id.as_str()))
        .map(|c| c.obs_id.clone())
        .collect();

    let validity_ratio = if cited_ids.is_empty() {
        0.0
    } else {
        verified.len() as f32 / cited_ids.len() as f32
    };

    CitationReport {
        cited_ids,
        verified,
        hallucinated,
        uncited_chunk_ids,
        validity_ratio,
    }
}

/// Create a clean one-line snippet from a chunk's text.
fn make_snippet(text: &str, max_len: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_len {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(max_len).collect();
        format!("{truncated}...")
    }
}

/// Formats the citation report into a human-readable string.
pub fn format_citation_report(report: &CitationReport) -> String {
    if report.cited_ids.is_empty() {
        return "No citations referenced in response.".into();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Citations ({} verified, {} invalid):\n",
        report.verified.len(),
        report.hallucinated.len()
    ));

    for v in &report.verified {
        let heading = v.heading_path.as_deref().unwrap_or("Page");
        out.push_str(&format!("  [{}] \"{}\" ({})\n", v.obs_id, v.snippet, heading));
    }

    if !report.hallucinated.is_empty() {
        out.push_str(&format!(
            "  Warning: Unknown citation IDs: {}\n",
            report.hallucinated.join(", ")
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk(id: &str, heading: &str, text: &str) -> ContentChunk {
        ContentChunk {
            obs_id: id.into(),
            heading_path: Some(heading.into()),
            text: text.into(),
            char_start: 0,
            char_end: text.len() as u32,
            suspect_injection: false,
        }
    }

    #[test]
    fn extracts_bracketed_citation_tokens() {
        let text = "Here is a fact [c0], another detail [c1, c2] and [c3][c4].";
        let ids = extract_citation_ids(text);
        assert_eq!(ids, vec!["c0", "c1", "c2", "c3", "c4"]);
    }

    #[test]
    fn skips_markdown_links() {
        let text = "Check [this link](https://example.com) for info [c0].";
        let ids = extract_citation_ids(text);
        assert_eq!(ids, vec!["c0"]);
    }

    #[test]
    fn verifies_matches_and_detects_hallucinations() {
        let chunks = vec![
            test_chunk("c0", "Intro", "Browse is a fast browser."),
            test_chunk("c1", "Features", "Supports local inference."),
        ];

        let answer = "Browse is fast [c0] and private [c1], but also runs on Mars [c99].";
        let report = verify_citations(answer, &chunks);

        assert_eq!(report.cited_ids, vec!["c0", "c1", "c99"]);
        assert_eq!(report.verified.len(), 2);
        assert_eq!(report.verified[0].obs_id, "c0");
        assert_eq!(report.verified[1].obs_id, "c1");
        assert_eq!(report.hallucinated, vec!["c99"]);
        assert!(report.uncited_chunk_ids.is_empty());
        assert!((report.validity_ratio - 2.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn formats_citation_report_cleanly() {
        let chunks = vec![test_chunk("c0", "Intro", "Smart browser core.")];
        let report = verify_citations("Based on [c0] and hallucinated [c99].", &chunks);
        let formatted = format_citation_report(&report);

        assert!(formatted.contains("1 verified, 1 invalid"));
        assert!(formatted.contains("[c0]"));
        assert!(formatted.contains("Intro"));
        assert!(formatted.contains("c99"));
    }
}
