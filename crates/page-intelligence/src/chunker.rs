//! Heading-aware text chunker (200–400 tokens, 10% overlap by default).

use crate::approx_tokens;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    pub target_tokens: u32,
    pub max_tokens: u32,
    pub overlap_ratio: f32,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self { target_tokens: 300, max_tokens: 400, overlap_ratio: 0.10 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub ordinal: u32,
    pub heading_path: Option<String>,
    pub text: String,
    pub char_start: u32,
    pub char_end: u32,
    pub token_count: u32,
}

/// Input block from the sensor: a paragraph/list/table cell with the heading
/// path it lives under.
#[derive(Debug, Clone)]
pub struct Block<'a> {
    pub heading_path: Option<&'a str>,
    pub text: &'a str,
    pub char_start: u32,
}

/// Chunk a sequence of blocks. Blocks under the same heading path are packed
/// together up to `target_tokens`; an oversized block is split on sentence
/// boundaries; consecutive chunks overlap by `overlap_ratio` of the previous
/// chunk's tail (sentence-aligned).
pub fn chunk_text(blocks: &[Block<'_>], cfg: &ChunkerConfig) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current: Vec<(String, u32, u32)> = Vec::new(); // (text, start, end)
    let mut current_heading: Option<String> = None;
    let mut current_tokens = 0u32;

    let flush = |chunks: &mut Vec<Chunk>, current: &mut Vec<(String, u32, u32)>, heading: &Option<String>, tokens: &mut u32| {
        if current.is_empty() {
            return;
        }
        let text = current.iter().map(|(t, _, _)| t.as_str()).collect::<Vec<_>>().join("\n");
        let start = current.first().map(|c| c.1).unwrap_or(0);
        let end = current.last().map(|c| c.2).unwrap_or(start);
        let ordinal = chunks.len() as u32;
        chunks.push(Chunk { ordinal, heading_path: heading.clone(), text: text.clone(), char_start: start, char_end: end, token_count: approx_tokens(&text) });
        // Overlap: keep the last sentences worth `overlap_ratio` of tokens.
        let keep_tokens = (*tokens as f32 * cfg.overlap_ratio) as u32;
        let mut kept = Vec::new();
        let mut kept_tokens = 0;
        for item in current.iter().rev() {
            let t = approx_tokens(&item.0);
            if kept_tokens + t > keep_tokens {
                break;
            }
            kept_tokens += t;
            kept.push(item.clone());
        }
        kept.reverse();
        *current = kept;
        *tokens = kept_tokens;
    };

    for block in blocks {
        let heading = block.heading_path.map(|s| s.to_string());
        if heading != current_heading && !current.is_empty() {
            flush(&mut chunks, &mut current, &current_heading, &mut current_tokens);
            // Overlap must not leak across headings.
            current.clear();
            current_tokens = 0;
        }
        current_heading = heading;

        for (sentence, s_start, s_end) in split_sentences(block.text, block.char_start) {
            let t = approx_tokens(&sentence);
            if current_tokens + t > cfg.max_tokens && !current.is_empty() {
                flush(&mut chunks, &mut current, &current_heading, &mut current_tokens);
            }
            current.push((sentence, s_start, s_end));
            current_tokens += t;
            if current_tokens >= cfg.target_tokens {
                flush(&mut chunks, &mut current, &current_heading, &mut current_tokens);
            }
        }
    }
    if !current.is_empty() && (chunks.is_empty() || current_tokens > 0) {
        // Emit the tail only if it contains material not already covered.
        let last_end = chunks.last().map(|c| c.char_end).unwrap_or(0);
        if current.last().map(|c| c.2 > last_end).unwrap_or(true) {
            flush(&mut chunks, &mut current, &current_heading, &mut current_tokens);
        }
    }
    chunks
}

fn split_sentences(text: &str, base: u32) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        let is_end = matches!(c, '.' | '!' | '?' | '。' | '！' | '？');
        let next_is_space = i + 1 >= bytes.len() || bytes[i + 1].is_whitespace();
        if is_end && next_is_space {
            let s: String = bytes[start..=i].iter().collect();
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push((trimmed.to_string(), base + start as u32, base + i as u32 + 1));
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        let s: String = bytes[start..].iter().collect();
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            out.push((trimmed.to_string(), base + start as u32, base + bytes.len() as u32));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_blocks_and_respects_headings() {
        let para = "This is a sentence about browsers. It has some words in it. ".repeat(20);
        let blocks = vec![
            Block { heading_path: Some("Intro"), text: &para, char_start: 0 },
            Block { heading_path: Some("Intro > Details"), text: &para, char_start: para.len() as u32 },
        ];
        let chunks = chunk_text(&blocks, &ChunkerConfig::default());
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.token_count <= 400 + 40));
        assert!(chunks.iter().any(|c| c.heading_path.as_deref() == Some("Intro")));
        assert!(chunks.iter().any(|c| c.heading_path.as_deref() == Some("Intro > Details")));
        for w in chunks.windows(2) {
            assert!(w[1].ordinal == w[0].ordinal + 1);
        }
    }

    #[test]
    fn short_text_is_one_chunk() {
        let blocks = vec![Block { heading_path: None, text: "Hello world. Second sentence!", char_start: 0 }];
        let chunks = chunk_text(&blocks, &ChunkerConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].char_start, 0);
        assert_eq!(chunks[0].char_end, 29);
    }
}
