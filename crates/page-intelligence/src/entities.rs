//! Entity & knowledge graph extraction from page chunks.
//!
//! Extracts entities (repositories, topics, products, organizations, people)
//! and co-occurrence relations from page chunks to populate the SQLite knowledge graph.

use core_types::{ExtractedEntity, ExtractedRelation};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeGraphExtraction {
    pub entities: Vec<ExtractedEntity>,
    pub relations: Vec<ExtractedRelation>,
}

const TECH_TOPICS: &[&str] = &[
    "rust", "typescript", "javascript", "python", "wasm", "sqlite",
    "chromium", "tokio", "react", "vue", "electron", "tauri", "docker",
    "linux", "windows", "macos", "ios", "android", "html", "css", "git",
    "github", "gitlab", "openai", "anthropic", "google", "apple", "microsoft",
];

const COMMON_SENTENCE_STARTERS: &[&str] = &[
    "the", "this", "that", "these", "those", "when", "what", "where",
    "how", "there", "with", "from", "here", "after", "before", "then",
    "some", "many", "such", "also", "each", "both", "all",
];

/// Extracts entities and co-occurrence edges from text chunks.
pub fn extract_entities_and_relations(text: &str) -> KnowledgeGraphExtraction {
    let mut entity_map: HashMap<String, ExtractedEntity> = HashMap::new();

    // 1. Tokenize into words
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '(' || c == ')' || c == '[' || c == ']' || c == '"' || c == '\'')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();

    for raw_word in &words {
        let clean = raw_word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '-' && c != '_');
        if clean.is_empty() {
            continue;
        }

        // 1a. GitHub / repository pattern: "org/repo"
        if let Some((org, repo)) = clean.split_once('/') {
            if !org.is_empty() && !repo.is_empty() && !org.contains('/') && !repo.contains('/')
                && org.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                && repo.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                && clean.len() >= 5
                && !clean.starts_with("and/")
                && !clean.starts_with("or/")
            {
                let norm = clean.to_lowercase();
                entity_map.entry(norm.clone()).or_insert_with(|| ExtractedEntity {
                    kind: "repo".into(),
                    name: clean.to_string(),
                    normalized: norm,
                    confidence: 0.90,
                });
                continue;
            }
        }

        let lower = clean.to_lowercase();

        // 1b. Known tech topics / ecosystems
        if TECH_TOPICS.contains(&lower.as_str()) {
            entity_map.entry(lower.clone()).or_insert_with(|| ExtractedEntity {
                kind: "topic".into(),
                name: capitalize_first(&lower),
                normalized: lower,
                confidence: 0.85,
            });
            continue;
        }

        // 1c. Capitalized proper nouns (projects, products, entities)
        if clean.len() >= 3 && clean.chars().next().is_some_and(|c| c.is_uppercase()) {
            let is_all_upper_short = clean.len() <= 5 && clean.chars().all(|c| c.is_uppercase());
            let is_capitalized = clean.chars().skip(1).all(|c| c.is_lowercase() || c.is_numeric());

            if (is_all_upper_short || is_capitalized) && !COMMON_SENTENCE_STARTERS.contains(&lower.as_str()) {
                entity_map.entry(lower.clone()).or_insert_with(|| ExtractedEntity {
                    kind: "project".into(),
                    name: clean.to_string(),
                    normalized: lower,
                    confidence: 0.70,
                });
            }
        }
    }

    let entities: Vec<ExtractedEntity> = entity_map.into_values().collect();

    // 2. Generate co-occurrence relations between distinct entities in the same chunk
    let mut relations = Vec::new();
    let mut seen_pairs = HashSet::new();

    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let a = &entities[i].normalized;
            let b = &entities[j].normalized;
            if a != b {
                let pair_key = if a < b { format!("{a}|{b}") } else { format!("{b}|{a}") };
                if seen_pairs.insert(pair_key) {
                    relations.push(ExtractedRelation {
                        src_normalized: a.clone(),
                        dst_normalized: b.clone(),
                        relation: "co_occurs_with".into(),
                        weight: 1.0,
                    });
                }
            }
        }
    }

    KnowledgeGraphExtraction { entities, relations }
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_repos_and_topics() {
        let text = "We are developing Browse with Rust, TypeScript and SQLite. Check out tokio-rs/tokio and Yonawie/Browse for details.";
        let res = extract_entities_and_relations(text);

        assert!(res.entities.iter().any(|e| e.kind == "repo" && e.normalized == "yonawie/browse"));
        assert!(res.entities.iter().any(|e| e.kind == "repo" && e.normalized == "tokio-rs/tokio"));
        assert!(res.entities.iter().any(|e| e.kind == "topic" && e.normalized == "rust"));
        assert!(res.entities.iter().any(|e| e.kind == "topic" && e.normalized == "sqlite"));

        // There should be co-occurrence edges
        assert!(!res.relations.is_empty());
        assert!(res.relations.iter().any(|r| r.src_normalized == "rust" || r.dst_normalized == "rust"));
    }
}
