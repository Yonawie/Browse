//! Natural language omnibox intent classifier and query dispatcher (IN-1).
//!
//! Classifies omnibox user input into actionable intents:
//! - Direct URL (`https://...`, `localhost:...`)
//! - Web Search (`what is rust`, `weather tokyo`)
//! - Tab Management Command (`close shopping tabs`, `group by project`)
//! - Memory Query (`what did I read about SQLite yesterday?`)
//! - Autonomous Agent Task (`find shoes under $100 and buy them`)
//!
//! Provides deterministic zero-latency categorization and semantic routing
//! before triggering heavier background models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmniboxIntentKind {
    Url,
    Search,
    TabCommand,
    MemoryQuery,
    AgentTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmniboxClassification {
    pub kind: OmniboxIntentKind,
    pub query: String,
    pub confidence: u8,
    pub explanation: String,
}

/// Classify free-form omnibox input into an explicit intent.
pub fn classify_omnibox_input(input: &str) -> OmniboxClassification {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return OmniboxClassification {
            kind: OmniboxIntentKind::Search,
            query: String::new(),
            confidence: 100,
            explanation: "Empty input defaults to search".into(),
        };
    }

    let lower = trimmed.to_lowercase();

    // 1. Direct URL detection
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("chrome://")
        || lower.starts_with("browser://")
        || lower.starts_with("about:")
    {
        return OmniboxClassification {
            kind: OmniboxIntentKind::Url,
            query: trimmed.to_string(),
            confidence: 99,
            explanation: "Direct web protocol URL".into(),
        };
    }

    // Domain pattern: e.g. "github.com", "example.org/path", "localhost:3000"
    if !trimmed.contains(' ')
        && (trimmed.contains('.') || trimmed.contains("localhost"))
        && !trimmed.ends_with('.')
    {
        let url_formatted = if lower.starts_with("localhost") {
            format!("http://{trimmed}")
        } else {
            format!("https://{trimmed}")
        };
        return OmniboxClassification {
            kind: OmniboxIntentKind::Url,
            query: url_formatted,
            confidence: 95,
            explanation: "Standard web domain name".into(),
        };
    }

    // 2. Tab management commands
    if lower.starts_with("close tab")
        || lower.starts_with("close tabs")
        || lower.starts_with("group tabs")
        || lower.starts_with("organize tabs")
        || lower.starts_with("archive tabs")
        || lower.starts_with("mute tab")
        || lower.starts_with("pin tab")
    {
        return OmniboxClassification {
            kind: OmniboxIntentKind::TabCommand,
            query: trimmed.to_string(),
            confidence: 95,
            explanation: "Browser tab organization command".into(),
        };
    }

    // 3. Autonomous Agent task intents (action verbs)
    if lower.starts_with("buy ")
        || lower.starts_with("order ")
        || lower.starts_with("book ")
        || lower.starts_with("fill ")
        || lower.starts_with("find and ")
        || lower.starts_with("sign up ")
        || lower.starts_with("download all ")
        || lower.starts_with("crawl ")
        || lower.starts_with("extract all ")
    {
        return OmniboxClassification {
            kind: OmniboxIntentKind::AgentTask,
            query: trimmed.to_string(),
            confidence: 90,
            explanation: "Multi-step autonomous web task".into(),
        };
    }

    // 4. Memory / personal history recall questions
    if lower.starts_with("what did i read")
        || lower.starts_with("what was that")
        || lower.starts_with("where did i see")
        || lower.starts_with("find page about")
        || lower.starts_with("show my history")
        || lower.starts_with("remember ")
        || lower.contains("yesterday")
        || lower.contains("last week")
        || lower.contains("in my memory")
    {
        return OmniboxClassification {
            kind: OmniboxIntentKind::MemoryQuery,
            query: trimmed.to_string(),
            confidence: 90,
            explanation: "Personal memory & browsing history recall".into(),
        };
    }

    // 5. Default: Web Search
    OmniboxClassification {
        kind: OmniboxIntentKind::Search,
        query: trimmed.to_string(),
        confidence: 85,
        explanation: "Web search query".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_urls_correctly() {
        assert_eq!(classify_omnibox_input("https://github.com").kind, OmniboxIntentKind::Url);
        assert_eq!(classify_omnibox_input("rust-lang.org/learn").kind, OmniboxIntentKind::Url);
        assert_eq!(classify_omnibox_input("localhost:8080").kind, OmniboxIntentKind::Url);
    }

    #[test]
    fn classifies_tab_commands() {
        assert_eq!(classify_omnibox_input("close tabs about shopping").kind, OmniboxIntentKind::TabCommand);
        assert_eq!(classify_omnibox_input("group tabs by project").kind, OmniboxIntentKind::TabCommand);
    }

    #[test]
    fn classifies_agent_tasks() {
        assert_eq!(classify_omnibox_input("find and add shoes under $50 to cart").kind, OmniboxIntentKind::AgentTask);
        assert_eq!(classify_omnibox_input("book flight to Tokyo").kind, OmniboxIntentKind::AgentTask);
    }

    #[test]
    fn classifies_memory_queries() {
        assert_eq!(classify_omnibox_input("what did I read about SQLite yesterday?").kind, OmniboxIntentKind::MemoryQuery);
        assert_eq!(classify_omnibox_input("where did I see that rust tutorial").kind, OmniboxIntentKind::MemoryQuery);
    }

    #[test]
    fn classifies_general_search() {
        assert_eq!(classify_omnibox_input("weather forecast tomorrow").kind, OmniboxIntentKind::Search);
        assert_eq!(classify_omnibox_input("rust vs c++ memory safety").kind, OmniboxIntentKind::Search);
    }
}
