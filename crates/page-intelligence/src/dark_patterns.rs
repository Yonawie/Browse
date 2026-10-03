//! Dark Pattern Detection (Phase S8).
//!
//! Analyzes extracted page text and interactive DOM elements to identify
//! deceptive design patterns (FTC / EU DSA compliance), such as:
//! - Fake urgency / countdown timers
//! - Fake scarcity pressure
//! - Concealed recurring subscription renewals
//! - Confirmshaming opt-outs
//! - Sneak into basket / forced continuity

use serde::{Deserialize, Serialize};

/// Categories of detected dark patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DarkPatternCategory {
    FakeUrgency,
    FakeScarcity,
    HiddenSubscription,
    Confirmshaming,
    ForcedContinuity,
}

/// A specific dark pattern match with source snippet and explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DarkPatternFinding {
    pub category: DarkPatternCategory,
    pub title: &'static str,
    pub snippet: String,
    pub explanation: &'static str,
}

/// Pattern definition with multilingual regex-like keywords.
struct PatternRule {
    category: DarkPatternCategory,
    title: &'static str,
    explanation: &'static str,
    keywords: &'static [&'static str],
}

const RULES: &[PatternRule] = &[
    PatternRule {
        category: DarkPatternCategory::FakeUrgency,
        title: "Artificial Urgency",
        explanation: "Uses artificial time pressure to rush the user into a transaction before considering alternatives.",
        keywords: &[
            "sale ends in",
            "deal expires in",
            "limited time offer",
            "only today",
            "act fast before time runs out",
            "до конца акции осталось",
            "время действия скидки истекает",
            "предложение ограничено по времени",
            "только сегодня",
        ],
    },
    PatternRule {
        category: DarkPatternCategory::FakeScarcity,
        title: "Artificial Scarcity",
        explanation: "Pressures the user with claims of critically low inventory or competing buyers.",
        keywords: &[
            "only 1 left in stock",
            "only 2 left in stock",
            "only 3 left in stock",
            "only 1 item left in stock",
            "only 2 items left in stock",
            "only 3 items left in stock",
            "left in stock",
            "almost sold out",
            "selling fast",
            "people are viewing this item right now",
            "high demand - reserve now",
            "осталось всего 1 шт",
            "осталось всего 2 шт",
            "заканчивается на складе",
            "высокий спрос",
            "сейчас просматривают",
        ],
    },
    PatternRule {
        category: DarkPatternCategory::HiddenSubscription,
        title: "Concealed Recurring Charge",
        explanation: "Obscures automatic renewal or recurring credit card billing behind a trial or one-time action.",
        keywords: &[
            "renews automatically",
            "auto-renews at",
            "billed annually after trial",
            "billed monthly after trial",
            "after your free trial ends you will be charged",
            "recurring billing every",
            "автоматическое продление",
            "автоматически списывается",
            "после пробного периода будет списано",
            "ежемесячная подписка",
        ],
    },
    PatternRule {
        category: DarkPatternCategory::Confirmshaming,
        title: "Confirmshaming Opt-Out",
        explanation: "Uses guilt-inducing or derogatory language in decline options to manipulate user consent.",
        keywords: &[
            "no thanks, i hate saving money",
            "no, i prefer paying full price",
            "i don't want discounts",
            "no thanks, i don't like free gifts",
            "i don't care about security",
            "нет, я люблю переплачивать",
            "нет, мне не нужны скидки",
            "я откажусь от выгоды",
        ],
    },
    PatternRule {
        category: DarkPatternCategory::ForcedContinuity,
        title: "Forced Continuity / Hard Cancellation",
        explanation: "Creates artificial friction or penalties when attempting to cancel a service or subscription.",
        keywords: &[
            "call customer support to cancel",
            "cancellation fee applies",
            "must call between 9am and 5pm to cancel",
            "early termination fee",
            "для отмены необходимо позвонить",
            "комиссия за досрочное расторжение",
        ],
    },
];

/// Safely extract a snippet around byte position without violating UTF-8 char boundaries.
fn safe_snippet(text: &str, byte_pos: usize, match_len: usize) -> String {
    let mut start = byte_pos.saturating_sub(20);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (byte_pos + match_len + 30).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    text[start..end].trim().to_string()
}

/// Scans extracted page text or snippets for deceptive dark patterns.
pub fn detect_dark_patterns(text: &str) -> Vec<DarkPatternFinding> {
    let lower = text.to_lowercase();
    let mut findings = Vec::new();

    for rule in RULES {
        for &kw in rule.keywords {
            if let Some(pos) = lower.find(kw) {
                let snippet = safe_snippet(text, pos, kw.len());

                findings.push(DarkPatternFinding {
                    category: rule.category,
                    title: rule.title,
                    snippet,
                    explanation: rule.explanation,
                });
                break; // One match per category rule is sufficient
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_page_has_no_dark_patterns() {
        let text = "Welcome to Rust programming language documentation. Read chapters online.";
        let findings = detect_dark_patterns(text);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_fake_urgency_and_scarcity() {
        let text = "Special hotel booking: only 2 left in stock! Hurry, sale ends in 05:22!";
        let findings = detect_dark_patterns(text);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|f| f.category == DarkPatternCategory::FakeScarcity));
        assert!(findings.iter().any(|f| f.category == DarkPatternCategory::FakeUrgency));
    }

    #[test]
    fn detects_hidden_subscription() {
        let text = "Try our software free! Renews automatically at $49.99/mo after 7-day trial.";
        let findings = detect_dark_patterns(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, DarkPatternCategory::HiddenSubscription);
    }

    #[test]
    fn detects_confirmshaming() {
        let text = "Get 20% off your order! [Accept Deal] or [No thanks, I hate saving money]";
        let findings = detect_dark_patterns(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, DarkPatternCategory::Confirmshaming);
    }

    #[test]
    fn detects_russian_patterns() {
        let text = "Оформи карту прямо сейчас. После пробного периода будет списано 590 рублей ежемесячно.";
        let findings = detect_dark_patterns(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, DarkPatternCategory::HiddenSubscription);
    }
}
