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
#[serde(rename_all = "snake_case")]
pub enum TabCommandAction {
    CloseByQuery(String),
    CloseStale,
    GroupByDomainOrCategory,
    ArchiveStale,
    Unknown(String),
}

pub fn parse_tab_command(input: &str) -> TabCommandAction {
    let lower = input.trim().to_lowercase();
    if lower.contains("stale") || lower.contains("inactive") || lower.contains("old") {
        if lower.contains("archive") {
            TabCommandAction::ArchiveStale
        } else {
            TabCommandAction::CloseStale
        }
    } else if lower.starts_with("group") || lower.starts_with("organize") {
        TabCommandAction::GroupByDomainOrCategory
    } else if let Some(rest) = lower.strip_prefix("close tabs about ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if let Some(rest) = lower.strip_prefix("close tab about ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if let Some(rest) = lower.strip_prefix("close tabs with ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if let Some(rest) = lower.strip_prefix("close tab with ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if let Some(rest) = lower.strip_prefix("close tabs ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if let Some(rest) = lower.strip_prefix("close tab ") {
        TabCommandAction::CloseByQuery(rest.trim().to_string())
    } else if lower.starts_with("archive") || lower.starts_with("prune") {
        TabCommandAction::ArchiveStale
    } else {
        TabCommandAction::Unknown(input.trim().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmniboxClassification {
    pub kind: OmniboxIntentKind,
    pub query: String,
    pub confidence: u8,
    pub explanation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_action: Option<TabCommandAction>,
}

/// Parse quick search engine bangs (e.g. `!gh`, `!yt`, `!w`, `!ddg`, `!g`, `!c`, `!d`, `!r`).
pub fn parse_search_bang(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('!') {
        return None;
    }

    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
    let bang = parts[0].to_lowercase();
    let query = if parts.len() > 1 { parts[1].trim() } else { "" };
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();

    let target = match bang.as_str() {
        "!gh" | "!github" => format!("https://github.com/search?q={encoded}"),
        "!yt" | "!youtube" => format!("https://www.youtube.com/results?search_query={encoded}"),
        "!w" | "!wiki" | "!wikipedia" => format!("https://en.wikipedia.org/wiki/Special:Search?search={encoded}"),
        "!ddg" | "!duckduckgo" => format!("https://duckduckgo.com/?q={encoded}"),
        "!g" | "!google" => format!("https://www.google.com/search?q={encoded}"),
        "!b" | "!brave" => format!("https://search.brave.com/search?q={encoded}"),
        "!c" | "!crates" => format!("https://crates.io/search?q={encoded}"),
        "!d" | "!docs" => format!("https://docs.rs/releases/search?query={encoded}"),
        "!r" | "!reddit" => format!("https://www.reddit.com/search/?q={encoded}"),
        _ => return None,
    };
    Some(target)
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
            tab_action: None,
        };
    }

    // 0. Search engine quick bangs
    if let Some(bang_url) = parse_search_bang(trimmed) {
        return OmniboxClassification {
            kind: OmniboxIntentKind::Url,
            query: bang_url,
            confidence: 99,
            explanation: "Search engine bang navigation".into(),
            tab_action: None,
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
            tab_action: None,
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
            tab_action: None,
        };
    }

    // 2. Tab management commands
    let is_tab_cmd = (lower.starts_with("close ")
        || lower.starts_with("group ")
        || lower.starts_with("organize ")
        || lower.starts_with("archive ")
        || lower.starts_with("prune ")
        || lower.starts_with("mute ")
        || lower.starts_with("pin "))
        && (lower.contains("tab") || lower.contains("tabs"))
        || lower == "group tabs"
        || lower == "organize tabs"
        || lower == "prune tabs"
        || lower == "archive tabs";

    if is_tab_cmd {
        return OmniboxClassification {
            kind: OmniboxIntentKind::TabCommand,
            query: trimmed.to_string(),
            confidence: 95,
            explanation: "Browser tab organization command".into(),
            tab_action: Some(parse_tab_command(trimmed)),
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
            tab_action: None,
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
            tab_action: None,
        };
    }

    // 5. Default: Web Search
    OmniboxClassification {
        kind: OmniboxIntentKind::Search,
        query: trimmed.to_string(),
        confidence: 85,
        explanation: "Web search query".into(),
        tab_action: None,
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
        let c1 = classify_omnibox_input("close tabs about shopping");
        assert_eq!(c1.kind, OmniboxIntentKind::TabCommand);
        assert_eq!(c1.tab_action, Some(TabCommandAction::CloseByQuery("shopping".into())));

        let c2 = classify_omnibox_input("group tabs by project");
        assert_eq!(c2.kind, OmniboxIntentKind::TabCommand);
        assert_eq!(c2.tab_action, Some(TabCommandAction::GroupByDomainOrCategory));

        let c3 = classify_omnibox_input("close stale tabs");
        assert_eq!(c3.kind, OmniboxIntentKind::TabCommand);
        assert_eq!(c3.tab_action, Some(TabCommandAction::CloseStale));

        let c4 = classify_omnibox_input("prune tabs");
        assert_eq!(c4.kind, OmniboxIntentKind::TabCommand);
        assert_eq!(c4.tab_action, Some(TabCommandAction::ArchiveStale));
    }

    #[test]
    fn parses_tab_command_actions() {
        assert_eq!(parse_tab_command("close tabs about amazon"), TabCommandAction::CloseByQuery("amazon".into()));
        assert_eq!(parse_tab_command("close inactive tabs"), TabCommandAction::CloseStale);
        assert_eq!(parse_tab_command("archive stale tabs"), TabCommandAction::ArchiveStale);
        assert_eq!(parse_tab_command("organize tabs"), TabCommandAction::GroupByDomainOrCategory);
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

    #[test]
    fn parses_and_classifies_search_engine_bangs() {
        assert_eq!(
            parse_search_bang("!gh tokio rs"),
            Some("https://github.com/search?q=tokio+rs".to_string())
        );
        assert_eq!(
            parse_search_bang("!yt lofi beats"),
            Some("https://www.youtube.com/results?search_query=lofi+beats".to_string())
        );
        assert_eq!(
            parse_search_bang("!w rust programming"),
            Some("https://en.wikipedia.org/wiki/Special:Search?search=rust+programming".to_string())
        );
        assert_eq!(
            parse_search_bang("!ddg zero telemetry"),
            Some("https://duckduckgo.com/?q=zero+telemetry".to_string())
        );
        assert_eq!(
            parse_search_bang("!c serde"),
            Some("https://crates.io/search?q=serde".to_string())
        );
        assert_eq!(
            parse_search_bang("!d axum"),
            Some("https://docs.rs/releases/search?query=axum".to_string())
        );
        assert_eq!(parse_search_bang("regular search without bang"), None);

        let classified = classify_omnibox_input("!gh Yonawie/Browse");
        assert_eq!(classified.kind, OmniboxIntentKind::Url);
        assert_eq!(classified.query, "https://github.com/search?q=Yonawie%2FBrowse");
    }
}
