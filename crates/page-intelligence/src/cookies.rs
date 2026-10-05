use serde::{Deserialize, Serialize};

/// Semantic category of an HTTP or document cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieCategory {
    StrictlyNecessary,
    Functional,
    Analytics,
    Advertising,
    Unknown,
}

impl CookieCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StrictlyNecessary => "strictly_necessary",
            Self::Functional => "functional",
            Self::Analytics => "analytics",
            Self::Advertising => "advertising",
            Self::Unknown => "unknown",
        }
    }
}

/// Raw cookie input provided from CDP or document.cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCookieInput {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub same_site: Option<String>,
}

fn default_path() -> String {
    "/".to_string()
}

/// Processed cookie record with privacy classification and risk assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieInfo {
    pub name: String,
    pub value_preview: String,
    pub domain: String,
    pub path: String,
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    pub category: CookieCategory,
    pub is_third_party: bool,
    pub privacy_risk: String,
}

/// Aggregated cookie & storage audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageAuditSummary {
    pub target_domain: String,
    pub total_cookies: usize,
    pub strictly_necessary_count: usize,
    pub functional_count: usize,
    pub analytics_count: usize,
    pub advertising_count: usize,
    pub third_party_trackers_count: usize,
    pub insecure_count: usize,
    pub cookies: Vec<CookieInfo>,
}

/// Deterministically classifies a cookie by name and properties.
pub fn classify_cookie(
    name: &str,
    cookie_domain: &str,
    page_domain: &str,
    is_secure: bool,
    is_http_only: bool,
) -> (CookieCategory, String, bool) {
    let lower_name = name.to_lowercase();
    let clean_cookie_dom = cookie_domain.trim_start_matches('.').to_lowercase();
    let clean_page_dom = page_domain.trim_start_matches("www.").to_lowercase();
    let is_third_party = !clean_cookie_dom.is_empty()
        && !clean_page_dom.is_empty()
        && !clean_page_dom.ends_with(&clean_cookie_dom)
        && !clean_cookie_dom.ends_with(&clean_page_dom);

    // 1. Advertising & Tracking cookies
    let ad_patterns = [
        "_fbp", "_fbc", "_gcl_", "ide", "test_cookie", "fr", "uuid2", "muc_ads",
        "personalization_id", "ad_id", "conversion", "retargeting", "doubleclick",
        "rl_anonymous_id", "criteo", "outbrain", "taboola",
    ];
    for p in ad_patterns {
        if lower_name.contains(p) {
            return (CookieCategory::Advertising, "High".to_string(), is_third_party);
        }
    }

    // 2. Analytics cookies
    let analytics_patterns = [
        "_ga", "_gid", "_gat", "_pk_", "_clck", "_clsk", "amp_", "_hj", "ym_",
        "yandex", "mixpanel", "segment", "amplitude", "posthog", "stat_counter",
    ];
    for p in analytics_patterns {
        if lower_name.contains(p) {
            return (CookieCategory::Analytics, "Medium".to_string(), is_third_party);
        }
    }

    // 3. Strictly Necessary / Authentication / CSRF
    let necessary_patterns = [
        "phpsessid", "jsessionid", "aspsessionid", "connect.sid", "csrf", "xsrf",
        "auth", "session_id", "token", "jwt", "logged_in", "identity",
    ];
    for p in necessary_patterns {
        if lower_name.contains(p) {
            let risk = if !is_secure || !is_http_only {
                "Medium (Missing Secure/HttpOnly)".to_string()
            } else {
                "Low".to_string()
            };
            return (CookieCategory::StrictlyNecessary, risk, is_third_party);
        }
    }

    // 4. Functional / Preferences
    let functional_patterns = [
        "theme", "dark", "light", "lang", "locale", "tz", "timezone", "volume",
        "sidebar", "consent", "cookie_consent", "gdpr", "ccpa", "layout",
    ];
    for p in functional_patterns {
        if lower_name.contains(p) {
            return (CookieCategory::Functional, "Low".to_string(), is_third_party);
        }
    }

    // 5. Fallback heuristics
    let risk = if is_third_party {
        "Medium".to_string()
    } else {
        "Low".to_string()
    };
    (CookieCategory::Unknown, risk, is_third_party)
}

/// Audits a set of cookies against the current browsing origin.
pub fn audit_cookies(page_domain: &str, raw_cookies: &[RawCookieInput]) -> StorageAuditSummary {
    let mut total_cookies = 0;
    let mut strictly_necessary_count = 0;
    let mut functional_count = 0;
    let mut analytics_count = 0;
    let mut advertising_count = 0;
    let mut third_party_trackers_count = 0;
    let mut insecure_count = 0;
    let mut cookies = Vec::with_capacity(raw_cookies.len());

    for rc in raw_cookies {
        total_cookies += 1;
        let (category, privacy_risk, is_third_party) = classify_cookie(
            &rc.name,
            &rc.domain,
            page_domain,
            rc.secure,
            rc.http_only,
        );

        match category {
            CookieCategory::StrictlyNecessary => strictly_necessary_count += 1,
            CookieCategory::Functional => functional_count += 1,
            CookieCategory::Analytics => analytics_count += 1,
            CookieCategory::Advertising => advertising_count += 1,
            CookieCategory::Unknown => {}
        }

        if is_third_party {
            third_party_trackers_count += 1;
        }

        if !rc.secure || !rc.http_only {
            insecure_count += 1;
        }

        // Mask cookie value to prevent leaking sensitive session secrets
        let value_preview = if rc.value.len() > 16 {
            format!("{}...{}", &rc.value[..6], &rc.value[rc.value.len() - 4..])
        } else if !rc.value.is_empty() {
            "********".to_string()
        } else {
            String::new()
        };

        cookies.push(CookieInfo {
            name: rc.name.clone(),
            value_preview,
            domain: rc.domain.clone(),
            path: rc.path.clone(),
            expires: rc.expires,
            secure: rc.secure,
            http_only: rc.http_only,
            same_site: rc.same_site.clone().unwrap_or_else(|| "Lax".to_string()),
            category,
            is_third_party,
            privacy_risk,
        });
    }

    StorageAuditSummary {
        target_domain: page_domain.to_string(),
        total_cookies,
        strictly_necessary_count,
        functional_count,
        analytics_count,
        advertising_count,
        third_party_trackers_count,
        insecure_count,
        cookies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_various_cookie_types_accurately() {
        let (cat, risk, third) = classify_cookie("_ga", "example.com", "example.com", true, false);
        assert_eq!(cat, CookieCategory::Analytics);
        assert_eq!(risk, "Medium");
        assert!(!third);

        let (cat, risk, third) = classify_cookie("_fbp", "facebook.com", "myblog.org", true, false);
        assert_eq!(cat, CookieCategory::Advertising);
        assert_eq!(risk, "High");
        assert!(third);

        let (cat, risk, _) = classify_cookie("session_id", "example.com", "example.com", true, true);
        assert_eq!(cat, CookieCategory::StrictlyNecessary);
        assert_eq!(risk, "Low");

        let (cat, risk, _) = classify_cookie("theme", "example.com", "example.com", false, false);
        assert_eq!(cat, CookieCategory::Functional);
        assert_eq!(risk, "Low");
    }

    #[test]
    fn audits_storage_cookies_and_masks_values() {
        let raw = vec![
            RawCookieInput {
                name: "PHPSESSID".into(),
                value: "abcdef1234567890secretkey".into(),
                domain: "shop.example.com".into(),
                path: "/".into(),
                expires: None,
                secure: true,
                http_only: true,
                same_site: Some("Strict".into()),
            },
            RawCookieInput {
                name: "_ga".into(),
                value: "GA1.2.987654321".into(),
                domain: ".example.com".into(),
                path: "/".into(),
                expires: Some(1800000000),
                secure: true,
                http_only: false,
                same_site: None,
            },
            RawCookieInput {
                name: "_fbp".into(),
                value: "fb.1.123456".into(),
                domain: ".doubleclick.net".into(),
                path: "/".into(),
                expires: Some(1800000000),
                secure: true,
                http_only: false,
                same_site: Some("None".into()),
            },
        ];

        let audit = audit_cookies("example.com", &raw);
        assert_eq!(audit.total_cookies, 3);
        assert_eq!(audit.strictly_necessary_count, 1);
        assert_eq!(audit.analytics_count, 1);
        assert_eq!(audit.advertising_count, 1);
        assert_eq!(audit.third_party_trackers_count, 1);
        // Sensitive session ID should be masked
        assert!(audit.cookies[0].value_preview.contains("..."));
        assert!(!audit.cookies[0].value_preview.contains("secretkey"));
    }
}
