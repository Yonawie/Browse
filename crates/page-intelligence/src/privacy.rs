use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerCategory {
    Analytics,
    Advertising,
    SessionRecording,
    Fingerprinting,
    SocialWidget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerFinding {
    pub name: String,
    pub category: TrackerCategory,
    pub domain_pattern: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintSignal {
    pub api_name: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyAuditReport {
    pub grade: String,
    pub score: u32, // 0 to 100
    pub trackers: Vec<TrackerFinding>,
    pub fingerprinting: Vec<FingerprintSignal>,
    pub third_party_hosts: Vec<String>,
    pub summary: String,
}

struct KnownTrackerRule {
    name: &'static str,
    category: TrackerCategory,
    pattern: &'static str,
    description: &'static str,
}

const KNOWN_TRACKERS: &[KnownTrackerRule] = &[
    KnownTrackerRule {
        name: "Google Analytics / Tag Manager",
        category: TrackerCategory::Analytics,
        pattern: "google-analytics.com",
        description: "Google Analytics behavioral and pageview telemetry",
    },
    KnownTrackerRule {
        name: "Google Tag Manager",
        category: TrackerCategory::Analytics,
        pattern: "googletagmanager.com",
        description: "Dynamic container script loading arbitrary third-party marketing tags",
    },
    KnownTrackerRule {
        name: "Meta / Facebook Pixel",
        category: TrackerCategory::Advertising,
        pattern: "connect.facebook.net",
        description: "Cross-site social tracking and conversion attribution",
    },
    KnownTrackerRule {
        name: "Meta / Facebook Pixel (fbevents)",
        category: TrackerCategory::Advertising,
        pattern: "fbevents.js",
        description: "Facebook cross-site event telemetry",
    },
    KnownTrackerRule {
        name: "Google DoubleClick / AdSense",
        category: TrackerCategory::Advertising,
        pattern: "doubleclick.net",
        description: "Targeted advertising and cross-site bidding network",
    },
    KnownTrackerRule {
        name: "Criteo Retargeting",
        category: TrackerCategory::Advertising,
        pattern: "criteo.net",
        description: "Cross-site behavioral ad retargeting",
    },
    KnownTrackerRule {
        name: "Yandex Metrika",
        category: TrackerCategory::Analytics,
        pattern: "mc.yandex.ru",
        description: "Behavioral analytics and click heatmaps",
    },
    KnownTrackerRule {
        name: "Hotjar Session Recorder",
        category: TrackerCategory::SessionRecording,
        pattern: "static.hotjar.com",
        description: "Session replay and keystroke/mouse movement tracking",
    },
    KnownTrackerRule {
        name: "Microsoft Clarity",
        category: TrackerCategory::SessionRecording,
        pattern: "clarity.ms",
        description: "Session replay and behavioral recording",
    },
    KnownTrackerRule {
        name: "TikTok Pixel",
        category: TrackerCategory::Advertising,
        pattern: "analytics.tiktok.com",
        description: "TikTok cross-app marketing telemetry",
    },
    KnownTrackerRule {
        name: "FingerprintJS",
        category: TrackerCategory::Fingerprinting,
        pattern: "fingerprintjs",
        description: "Browser fingerprinting library creating cross-browser visitor IDs",
    },
];

const FINGERPRINTING_PATTERNS: &[(&str, &str)] = &[
    ("toDataURL", "Canvas fingerprinting via getImageData/toDataURL"),
    ("WEBGL_debug_renderer_info", "WebGL GPU renderer and hardware profiling"),
    ("createOscillator", "AudioContext hardware and frequency response fingerprinting"),
    ("hardwareConcurrency", "Hardware core count and CPU profiling"),
    ("getBattery", "Battery API status tracking"),
];

/// Audits a page's content, scripts and origin for trackers, third-party analytics and fingerprinting (D-5).
pub fn inspect_page_privacy(
    page_url: &str,
    page_content: &str,
    script_sources: &[&str],
) -> PrivacyAuditReport {
    let lower_content = page_content.to_lowercase();
    let mut trackers = Vec::new();
    let mut third_party_hosts = Vec::new();

    let page_origin = url::Url::parse(page_url).ok();
    let page_host = page_origin.as_ref().and_then(|u| u.host_str()).unwrap_or("");

    // Check script sources and content for known tracking patterns
    for rule in KNOWN_TRACKERS {
        let in_scripts = script_sources.iter().any(|s| s.to_lowercase().contains(rule.pattern));
        let in_content = lower_content.contains(rule.pattern);

        if in_scripts || in_content {
            trackers.push(TrackerFinding {
                name: rule.name.to_string(),
                category: rule.category,
                domain_pattern: rule.pattern.to_string(),
                description: rule.description.to_string(),
            });
            if !third_party_hosts.contains(&rule.pattern.to_string()) {
                third_party_hosts.push(rule.pattern.to_string());
            }
        }
    }

    // Inspect script sources for external domains
    for &src in script_sources {
        if let Ok(parsed) = url::Url::parse(src) {
            if let Some(host) = parsed.host_str() {
                if host != page_host && !page_host.ends_with(host) && !third_party_hosts.contains(&host.to_string()) {
                    third_party_hosts.push(host.to_string());
                }
            }
        }
    }

    // Detect browser fingerprinting signals
    let mut fingerprinting = Vec::new();
    for &(needle, explanation) in FINGERPRINTING_PATTERNS {
        if lower_content.contains(&needle.to_lowercase()) {
            fingerprinting.push(FingerprintSignal {
                api_name: needle.to_string(),
                explanation: explanation.to_string(),
            });
        }
    }

    // Calculate privacy score (100 is cleanest)
    let mut score = 100u32;

    for t in &trackers {
        match t.category {
            TrackerCategory::Fingerprinting => score = score.saturating_sub(35),
            TrackerCategory::SessionRecording => score = score.saturating_sub(25),
            TrackerCategory::Advertising => score = score.saturating_sub(15),
            TrackerCategory::Analytics => score = score.saturating_sub(8),
            TrackerCategory::SocialWidget => score = score.saturating_sub(10),
        }
    }

    for _ in &fingerprinting {
        score = score.saturating_sub(12);
    }

    let grade = match score {
        90..=100 => "A",
        75..=89 => "B",
        55..=74 => "C",
        35..=54 => "D",
        _ => "F",
    }
    .to_string();

    let summary = if trackers.is_empty() && fingerprinting.is_empty() {
        "Grade A: Zero third-party trackers or fingerprinting detected. Clean page.".to_string()
    } else {
        format!(
            "Grade {}: Detected {} tracker(s) and {} fingerprinting signal(s).",
            grade,
            trackers.len(),
            fingerprinting.len()
        )
    };

    PrivacyAuditReport {
        grade,
        score,
        trackers,
        fingerprinting,
        third_party_hosts,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_page_receives_grade_a() {
        let report = inspect_page_privacy(
            "https://example.com/docs",
            "Welcome to documentation. No third party scripts here.",
            &[],
        );
        assert_eq!(report.grade, "A");
        assert_eq!(report.score, 100);
        assert!(report.trackers.is_empty());
        assert!(report.fingerprinting.is_empty());
    }

    #[test]
    fn detects_analytics_and_facebook_pixel() {
        let scripts = vec![
            "https://www.googletagmanager.com/gtag/js?id=UA-12345",
            "https://connect.facebook.net/en_US/fbevents.js",
        ];
        let report = inspect_page_privacy("https://shop.example.com", "Online Shop", &scripts);
        assert!(report.score < 80);
        assert!(report.trackers.iter().any(|t| t.name.contains("Google Tag Manager")));
        assert!(report.trackers.iter().any(|t| t.name.contains("Meta / Facebook Pixel")));
    }

    #[test]
    fn detects_fingerprinting_signals() {
        let content = "function getCanvasId() { var c = document.createElement('canvas'); return c.toDataURL(); }";
        let report = inspect_page_privacy("https://tracker.example.com", content, &[]);
        assert!(!report.fingerprinting.is_empty());
        assert_eq!(report.fingerprinting[0].api_name, "toDataURL");
    }
}
