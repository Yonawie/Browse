use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Category of blocked web traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdBlockCategory {
    Advertising,
    Analytics,
    Tracking,
    Cryptominer,
    Malware,
}

impl AdBlockCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Advertising => "advertising",
            Self::Analytics => "analytics",
            Self::Tracking => "tracking",
            Self::Cryptominer => "cryptominer",
            Self::Malware => "malware",
        }
    }
}

/// A single block or exception rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdBlockRule {
    pub pattern: String,
    pub category: AdBlockCategory,
    pub is_exception: bool,
}

/// Result of evaluating an URL against the AdBlock engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AdBlockDecision {
    Allow,
    Block {
        category: AdBlockCategory,
        rule: String,
        reason: String,
    },
}

/// Statistics of blocked requests.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AdBlockStats {
    pub total_evaluated: u64,
    pub total_blocked: u64,
    pub total_allowed: u64,
    pub by_category: HashMap<String, u64>,
}

/// AdBlock & Tracker Protection Engine (ADR-007, Brave Shield / EasyList style).
#[derive(Debug, Clone)]
pub struct AdBlockEngine {
    rules: Vec<AdBlockRule>,
    enabled: bool,
}

impl Default for AdBlockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AdBlockEngine {
    /// Creates a new AdBlock engine preloaded with high-impact ad, tracking, and telemetry rules.
    pub fn new() -> Self {
        let mut engine = Self {
            rules: Vec::new(),
            enabled: true,
        };
        engine.load_builtin_rules();
        engine
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Load default high-frequency ad and tracker signatures.
    pub fn load_builtin_rules(&mut self) {
        let ads = [
            "doubleclick.net",
            "googleadservices.com",
            "googlesyndication.com",
            "adnxs.com",
            "criteo.com",
            "criteo.net",
            "outbrain.com",
            "taboola.com",
            "adroll.com",
            "amazon-adsystem.com",
            "popads.net",
            "adservice.google.",
            "pagead2.googlesyndication.",
            "an.yandex.ru",
            "ads.pubmatic.com",
            "rubiconproject.com",
            "advertising.com",
        ];
        for pattern in ads {
            self.rules.push(AdBlockRule {
                pattern: pattern.to_string(),
                category: AdBlockCategory::Advertising,
                is_exception: false,
            });
        }

        let trackers = [
            "google-analytics.com",
            "googletagmanager.com/gtm.js",
            "googletagmanager.com/gtag/js",
            "facebook.net/en_US/fbevents.js",
            "connect.facebook.net",
            "mc.yandex.ru/metrika",
            "hotjar.com",
            "static.hotjar.com",
            "clarity.ms",
            "analytics.tiktok.com",
            "browser.sentry-cdn.com",
            "segment.io",
            "cdn.segment.com",
            "api.mixpanel.com",
            "cdn.amplitude.com",
            "fpjs.sh",
            "fingerprintjs.com",
        ];
        for pattern in trackers {
            self.rules.push(AdBlockRule {
                pattern: pattern.to_string(),
                category: AdBlockCategory::Tracking,
                is_exception: false,
            });
        }

        let analytics = [
            "telemetry.",
            "stats.wp.com",
            "ping.chartbeat.net",
            "scorecardresearch.com",
            "quantserve.com",
        ];
        for pattern in analytics {
            self.rules.push(AdBlockRule {
                pattern: pattern.to_string(),
                category: AdBlockCategory::Analytics,
                is_exception: false,
            });
        }

        let miners = [
            "coinhive.com",
            "coin-hive.com",
            "crypto-loot.com",
            "monerominer.rocks",
        ];
        for pattern in miners {
            self.rules.push(AdBlockRule {
                pattern: pattern.to_string(),
                category: AdBlockCategory::Cryptominer,
                is_exception: false,
            });
        }
    }

    /// Add a custom user rule. Prefix with `@@` to create an exception rule.
    pub fn add_rule(&mut self, rule: &str, category: AdBlockCategory) {
        let trimmed = rule.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(pattern) = trimmed.strip_prefix("@@") {
            self.rules.push(AdBlockRule {
                pattern: pattern.trim().to_string(),
                category,
                is_exception: true,
            });
        } else {
            self.rules.push(AdBlockRule {
                pattern: trimmed.to_string(),
                category,
                is_exception: false,
            });
        }
    }

    /// Evaluates an URL to decide whether it should be blocked.
    pub fn evaluate(&self, url: &str) -> AdBlockDecision {
        if !self.enabled || url.is_empty() {
            return AdBlockDecision::Allow;
        }

        let url_lower = url.to_lowercase();

        // 1. Check exception rules first
        for rule in &self.rules {
            if rule.is_exception && Self::matches_pattern(&url_lower, &rule.pattern) {
                return AdBlockDecision::Allow;
            }
        }

        // 2. Check blocking rules
        for rule in &self.rules {
            if !rule.is_exception && Self::matches_pattern(&url_lower, &rule.pattern) {
                return AdBlockDecision::Block {
                    category: rule.category,
                    rule: rule.pattern.clone(),
                    reason: format!("Matched {} filter rule '{}'", rule.category.as_str(), rule.pattern),
                };
            }
        }

        AdBlockDecision::Allow
    }

    fn matches_pattern(url: &str, pattern: &str) -> bool {
        let p = pattern.to_lowercase();
        // Domain anchor: ||example.com
        if let Some(domain) = p.strip_prefix("||") {
            if let Some(pos) = url.find("://") {
                let after_proto = &url[pos + 3..];
                let host = after_proto.split(&['/', '?', '#', ':'][..]).next().unwrap_or("");
                return host == domain || host.ends_with(&format!(".{domain}"));
            }
            return url.contains(domain);
        }

        // Substring / glob match
        if p.contains('*') {
            let segments: Vec<&str> = p.split('*').filter(|s| !s.is_empty()).collect();
            if segments.is_empty() {
                return true;
            }
            let mut curr = url;
            for seg in segments {
                if let Some(found_idx) = curr.find(seg) {
                    curr = &curr[found_idx + seg.len()..];
                } else {
                    return false;
                }
            }
            return true;
        }

        url.contains(&p)
    }

    /// Converts active blocking rules into wildcard patterns for CDP `Network.setBlockedURLs`.
    pub fn cdp_blocked_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();
        for rule in &self.rules {
            if !rule.is_exception {
                if rule.pattern.starts_with('*') && rule.pattern.ends_with('*') {
                    patterns.push(rule.pattern.clone());
                } else if rule.pattern.starts_with("||") {
                    patterns.push(format!("*{}*", &rule.pattern[2..]));
                } else {
                    patterns.push(format!("*{}*", rule.pattern));
                }
            }
        }
        patterns
    }

    /// Total number of loaded rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_google_analytics_and_ads() {
        let engine = AdBlockEngine::new();

        let ga_url = "https://www.google-analytics.com/analytics.js";
        match engine.evaluate(ga_url) {
            AdBlockDecision::Block { category, .. } => {
                assert_eq!(category, AdBlockCategory::Tracking);
            }
            AdBlockDecision::Allow => panic!("Expected google-analytics to be blocked"),
        }

        let ad_url = "https://securepubads.g.doubleclick.net/gampad/ads?env=vp";
        match engine.evaluate(ad_url) {
            AdBlockDecision::Block { category, .. } => {
                assert_eq!(category, AdBlockCategory::Advertising);
            }
            AdBlockDecision::Allow => panic!("Expected doubleclick to be blocked"),
        }
    }

    #[test]
    fn allows_regular_content() {
        let engine = AdBlockEngine::new();
        assert_eq!(
            engine.evaluate("https://en.wikipedia.org/wiki/Rust_(programming_language)"),
            AdBlockDecision::Allow
        );
        assert_eq!(
            engine.evaluate("https://github.com/rust-lang/rust"),
            AdBlockDecision::Allow
        );
    }

    #[test]
    fn exception_rule_overrides_blocking() {
        let mut engine = AdBlockEngine::new();
        // Exception for internal analytics
        engine.add_rule("@@analytics.mycompany.google-analytics.com", AdBlockCategory::Analytics);

        assert_eq!(
            engine.evaluate("https://analytics.mycompany.google-analytics.com/app"),
            AdBlockDecision::Allow
        );
        // Regular ga is still blocked
        assert!(matches!(
            engine.evaluate("https://www.google-analytics.com/collect"),
            AdBlockDecision::Block { .. }
        ));
    }

    #[test]
    fn generates_cdp_blocked_patterns() {
        let engine = AdBlockEngine::new();
        let patterns = engine.cdp_blocked_patterns();
        assert!(!patterns.is_empty());
        assert!(patterns.iter().any(|p| p.contains("doubleclick.net")));
        assert!(patterns.iter().any(|p| p.contains("google-analytics.com")));
    }
}
