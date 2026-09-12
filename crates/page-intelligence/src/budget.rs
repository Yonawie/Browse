//! Observation budgeting (ADR-006): keep the snapshot within a token budget by
//! dropping the least useful interactive elements and content chunks first.

use crate::approx_tokens;
use core_types::Observation;

#[derive(Debug, Clone, Copy)]
pub struct ObservationBudget {
    pub max_tokens: u32,
    pub max_interactive: usize,
    pub max_content_chunks: usize,
}

impl ObservationBudget {
    /// Budget for local 4B–8B models.
    pub const LOCAL: ObservationBudget = ObservationBudget { max_tokens: 4_000, max_interactive: 150, max_content_chunks: 12 };
    /// Budget for cloud planning (only `public` observations).
    pub const CLOUD: ObservationBudget = ObservationBudget { max_tokens: 12_000, max_interactive: 400, max_content_chunks: 40 };
}

pub fn estimate_tokens(obs: &Observation) -> u32 {
    serde_json::to_string(obs).map(|s| approx_tokens(&s)).unwrap_or(u32::MAX)
}

/// Trim in place. Priority order:
/// 1. interactive elements outside the viewport (furthest first),
/// 2. content chunks beyond `max_content_chunks` (later chunks first),
/// 3. remaining interactive elements beyond `max_interactive`.
///
/// Hidden-text signals and site tools are never trimmed (they are small and
/// security-relevant).
pub fn trim_observation(obs: &mut Observation, budget: ObservationBudget) {
    obs.interactive.sort_by_key(|e| {
        let vp = if e.in_viewport { 0 } else { 1 };
        let y = e.bbox.as_ref().map(|b| b.y as i64).unwrap_or(i64::MAX / 2);
        (vp, y)
    });
    if obs.interactive.len() > budget.max_interactive {
        obs.interactive.truncate(budget.max_interactive);
    }
    if obs.content.len() > budget.max_content_chunks {
        obs.content.truncate(budget.max_content_chunks);
    }
    let mut tokens = estimate_tokens(obs);
    while tokens > budget.max_tokens {
        // Drop the furthest off-viewport element, else the last content chunk, else the last element.
        if let Some(pos) = obs.interactive.iter().rposition(|e| !e.in_viewport) {
            obs.interactive.remove(pos);
        } else if obs.content.len() > 1 {
            obs.content.pop();
        } else if obs.interactive.len() > 1 {
            obs.interactive.pop();
        } else {
            break;
        }
        tokens = estimate_tokens(obs);
    }
    obs.approx_tokens = tokens;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::*;

    #[test]
    fn trims_offscreen_first() {
        let mut interactive = Vec::new();
        for i in 0..300u64 {
            interactive.push(InteractiveElement {
                element_ref: ElementRef { id: i, path: format!("button/b{i}") },
                role: "button".into(),
                name: format!("Button number {i} with a fairly long accessible name"),
                state: ElementState::default(),
                value: None,
                href: None,
                input_type: None,
                bbox: Some(BoundingBox { x: 0.0, y: i as f32 * 40.0, w: 10.0, h: 10.0 }),
                in_viewport: i < 20,
                landmark: None,
                consequential_hint: false,
            });
        }
        let mut obs = Observation {
            page: PageMeta {
                url: "https://a.example/".into(),
                origin: Origin::parse("https://a.example").unwrap(),
                title: "t".into(),
                lang: None,
                page_kind: PageKind::Unknown,
                untrusted: true,
                frame_id: "main".into(),
                snapshot_hash: String::new(),
                sensitivity: Sensitivity::Public,
                has_session: false,
            },
            interactive,
            content: (0..30).map(|i| ContentChunk { obs_id: format!("c{i}"), heading_path: None, text: "lorem ipsum ".repeat(60), char_start: 0, char_end: 0, suspect_injection: false }).collect(),
            tools: vec![],
            hidden_text_signals: vec!["ignore previous".into()],
            approx_tokens: 0,
            captured_at: 0,
        };
        trim_observation(&mut obs, ObservationBudget::LOCAL);
        assert!(obs.approx_tokens <= ObservationBudget::LOCAL.max_tokens, "{}", obs.approx_tokens);
        assert!(obs.interactive.iter().take(20).all(|e| e.in_viewport));
        assert_eq!(obs.hidden_text_signals.len(), 1);
    }
}
