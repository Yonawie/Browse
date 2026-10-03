//! Hybrid retrieval: FTS5 (lexical) ⊕ vector (semantic) fused with RRF.

use crate::store::{bytes_to_f32s, MemoryStore};
use crate::Result;
use core_types::Sensitivity;
use rusqlite::params;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    pub domain: Option<String>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    /// Exclude chunks above this sensitivity (e.g. when the results will be
    /// shown to a cloud model).
    pub max_sensitivity: Option<Sensitivity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub chunk_id: String,
    pub page_version_id: String,
    pub url: String,
    pub title: Option<String>,
    pub heading_path: Option<String>,
    pub text: String,
    pub score: f32,
    pub sensitivity: Sensitivity,
}

const RRF_K: f32 = 60.0;

impl MemoryStore {
    pub fn search_lexical(&self, query: &str, filters: &SearchFilters, k: usize) -> Result<Vec<(String, f32)>> {
        let fts_query = sanitize_fts(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }
        let mut stmt = self.conn().prepare(
            "SELECT c.id, bm25(chunks_fts) AS rank
             FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid
             JOIN page_versions pv ON pv.id = c.page_version_id
             WHERE chunks_fts MATCH ?1
               AND (?2 IS NULL OR c.domain = ?2)
               AND (?3 IS NULL OR pv.captured_at >= ?3)
               AND (?4 IS NULL OR pv.captured_at <= ?4)
             ORDER BY rank LIMIT ?5",
        )?;
        let rows = stmt
            .query_map(params![fts_query, filters.domain, filters.since_ms, filters.until_ms, k as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)? as f32))
            })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Brute-force cosine similarity over `chunk_vectors_raw`. With sqlite-vec
    /// enabled this is replaced by a `vec0` KNN query with the same signature.
    pub fn search_vector(
        &self,
        query_embedding: &[f32],
        filters: &SearchFilters,
        k: usize,
    ) -> Result<Vec<(String, f32)>> {
        let mut stmt = self.conn().prepare(
            "SELECT v.chunk_id, v.embedding
             FROM chunk_vectors_raw v JOIN chunks c ON c.id = v.chunk_id
             JOIN page_versions pv ON pv.id = c.page_version_id
             WHERE (?1 IS NULL OR c.domain = ?1)
               AND (?2 IS NULL OR pv.captured_at >= ?2)
               AND (?3 IS NULL OR pv.captured_at <= ?3)",
        )?;
        let qn = norm(query_embedding);
        let mut scored: Vec<(String, f32)> = stmt
            .query_map(params![filters.domain, filters.since_ms, filters.until_ms], |r| {
                let id: String = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                Ok((id, blob))
            })?
            .filter_map(|r| r.ok())
            .map(|(id, blob)| {
                let v = bytes_to_f32s(&blob);
                let dot: f32 = v.iter().zip(query_embedding).map(|(a, b)| a * b).sum();
                let sim = if qn == 0.0 { 0.0 } else { dot / (norm(&v) * qn).max(1e-9) };
                (id, sim)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    /// Hybrid search: reciprocal-rank fusion of lexical and vector results,
    /// deduplicated per page version, enriched with page metadata.
    pub fn hybrid_search(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        filters: &SearchFilters,
        k: usize,
    ) -> Result<Vec<SearchHit>> {
        let pool = (k * 5).max(50);
        let lexical = self.search_lexical(query, filters, pool)?;
        let vector = match query_embedding {
            Some(e) => self.search_vector(e, filters, pool)?,
            None => vec![],
        };
        let mut fused: HashMap<String, f32> = HashMap::new();
        for (rank, (id, _)) in lexical.iter().enumerate() {
            *fused.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
        for (rank, (id, _)) in vector.iter().enumerate() {
            *fused.entry(id.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
        let mut ranked: Vec<(String, f32)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut hits = Vec::new();
        let mut seen_versions = std::collections::HashSet::new();
        let mut stmt = self.conn().prepare(
            "SELECT c.page_version_id, p.url, pv.title, c.heading_path, c.text, c.sensitivity
             FROM chunks c JOIN page_versions pv ON pv.id = c.page_version_id JOIN pages p ON p.id = pv.page_id
             WHERE c.id = ?1",
        )?;
        for (chunk_id, score) in ranked {
            let row = stmt.query_row(params![chunk_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            let sensitivity = parse_sensitivity(&row.5);
            if let Some(max) = filters.max_sensitivity {
                if sensitivity > max {
                    continue;
                }
            }
            if !seen_versions.insert(row.0.clone()) {
                continue;
            }
            hits.push(SearchHit {
                chunk_id,
                page_version_id: row.0,
                url: row.1,
                title: row.2,
                heading_path: row.3,
                text: row.4,
                score,
                sensitivity,
            });
            if hits.len() >= k {
                break;
            }
        }
        Ok(hits)
    }
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn parse_sensitivity(s: &str) -> Sensitivity {
    match s {
        "personal" => Sensitivity::Personal,
        "private" => Sensitivity::Private,
        "secret" => Sensitivity::Secret,
        _ => Sensitivity::Public,
    }
}

/// Turn free text into a safe FTS5 query: quoted terms joined by OR-less
/// implicit AND, with a prefix match on the last term.
fn sanitize_fts(query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        return String::new();
    }
    let mut q = terms.join(" ");
    q.push('*');
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewChunk, NewPageVersion};
    use core_types::PageKind;

    fn seeded() -> MemoryStore {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("p", "user", "default").unwrap();
        let mut e1 = vec![0.0f32; 256];
        e1[0] = 1.0;
        let mut e2 = vec![0.0f32; 256];
        e2[1] = 1.0;
        let pv1 = s
            .insert_page_version(&NewPageVersion {
                url: "https://docs.example/servo",
                title: Some("Servo embedding"),
                lang: Some("en"),
                page_kind: PageKind::Doc,
                sensitivity: Sensitivity::Public,
                main_text: None,
                content_hash: "a",
            })
            .unwrap();
        s.insert_chunks(
            &pv1,
            &[NewChunk {
                ordinal: 0,
                heading_path: Some("Embedding"),
                text: "Servo can be embedded as a Rust crate since April 2026.",
                char_start: 0,
                char_end: 50,
                token_count: 12,
                suspect_injection: false,
                embedding: Some(&e1),
            }],
        )
        .unwrap();
        let pv2 = s
            .insert_page_version(&NewPageVersion {
                url: "https://mail.example/inbox",
                title: Some("Inbox"),
                lang: Some("en"),
                page_kind: PageKind::App,
                sensitivity: Sensitivity::Private,
                main_text: None,
                content_hash: "b",
            })
            .unwrap();
        s.insert_chunks(
            &pv2,
            &[NewChunk {
                ordinal: 0,
                heading_path: None,
                text: "Your Servo conference ticket is attached.",
                char_start: 0,
                char_end: 40,
                token_count: 9,
                suspect_injection: false,
                embedding: Some(&e2),
            }],
        )
        .unwrap();
        s
    }

    #[test]
    fn lexical_and_hybrid_find_matches() {
        let s = seeded();
        let lex = s.search_lexical("servo embedded", &SearchFilters::default(), 10).unwrap();
        assert_eq!(lex.len(), 1);
        let mut q = vec![0.0f32; 256];
        q[1] = 1.0;
        let hits = s.hybrid_search("servo", Some(&q), &SearchFilters::default(), 10).unwrap();
        assert_eq!(hits.len(), 2);
        // The vector hit (inbox) and the lexical hit both appear; page dedup keeps one per version.
        assert!(hits.iter().any(|h| h.url.contains("mail.example")));
    }

    #[test]
    fn sensitivity_filter_hides_private_chunks() {
        let s = seeded();
        let f = SearchFilters { max_sensitivity: Some(Sensitivity::Public), ..Default::default() };
        let hits = s.hybrid_search("servo", None, &f, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].url.contains("docs.example"));
    }

    #[test]
    fn domain_filter_works() {
        let s = seeded();
        let f = SearchFilters { domain: Some("mail.example".into()), ..Default::default() };
        let hits = s.hybrid_search("servo", None, &f, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].sensitivity, Sensitivity::Private);
    }
}
