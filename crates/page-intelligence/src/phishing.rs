//! Phishing & Homograph Detection (Phase S8).
//!
//! Evaluates URLs and domains deterministically for:
//! 1. Homograph / mixed-script attacks (e.g. Cyrillic lookalikes in Latin domains).
//! 2. Punycode encoding (`xn--...`).
//! 3. Typosquatting / brand spoofing targeting high-value services (PayPal, Google, Apple, etc.).
//! 4. Raw IP addresses used in place of domain names.

use serde::{Deserialize, Serialize};
use url::Url;

/// Severity level of the phishing finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PhishingSeverity {
    Safe,
    Suspicious,
    Dangerous,
}

/// A structured report on URL/domain phishing heuristics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhishingReport {
    pub url: String,
    pub domain: String,
    pub severity: PhishingSeverity,
    pub reasons: Vec<String>,
}

impl PhishingReport {
    pub fn is_safe(&self) -> bool {
        self.severity == PhishingSeverity::Safe
    }
}

/// Known high-value brands frequently targeted for spoofing and credential theft.
const PROTECTED_BRANDS: &[&str] = &[
    "google",
    "paypal",
    "apple",
    "amazon",
    "microsoft",
    "github",
    "binance",
    "coinbase",
    "netflix",
    "chase",
    "wellsfargo",
    "facebook",
    "instagram",
    "twitter",
    "steam",
    "telegram",
];

/// Known Cyrillic and Greek homoglyphs commonly substituted for Latin letters.
const LOOKALIKE_HOMOGLYPHS: &[(char, char)] = &[
    ('\u{0430}', 'a'), // Cyrillic small letter a
    ('\u{0441}', 'c'), // Cyrillic small letter es (c)
    ('\u{0435}', 'e'), // Cyrillic small letter ie (e)
    ('\u{043E}', 'o'), // Cyrillic small letter o
    ('\u{0440}', 'p'), // Cyrillic small letter er (p)
    ('\u{0445}', 'x'), // Cyrillic small letter ha (x)
    ('\u{0443}', 'y'), // Cyrillic small letter u (y)
    ('\u{0456}', 'i'), // Cyrillic small letter Byelorussian-Ukrainian i
    ('\u{0458}', 'j'), // Cyrillic small letter je (j)
    ('\u{03BF}', 'o'), // Greek small letter omicron (o)
    ('\u{03B1}', 'a'), // Greek small letter alpha (a)
    ('\u{03C1}', 'p'), // Greek small letter rho (p)
];

/// Analyzes a target URL for phishing, homograph attacks, and domain deception.
pub fn inspect_url_phishing(raw_url: &str) -> PhishingReport {
    let parsed = Url::parse(raw_url).ok();
    let domain = parsed
        .as_ref()
        .and_then(|u| u.domain())
        .unwrap_or(raw_url)
        .to_lowercase();

    let mut reasons = Vec::new();
    let mut severity = PhishingSeverity::Safe;

    // 1. Raw IP address as host
    if let Some(url_obj) = &parsed {
        if let Some(host) = url_obj.host_str() {
            if host.parse::<std::net::IpAddr>().is_ok() && !is_loopback_or_local(host) {
                reasons.push(format!("Domain uses a raw IP address ({host}) instead of a named host."));
                severity = severity.max(PhishingSeverity::Suspicious);
            }
        }
    }

    // 2. Punycode check
    if domain.contains("xn--") {
        reasons.push("Domain contains internationalized Punycode ('xn--'), often used to disguise lookalike characters.".to_string());
        severity = severity.max(PhishingSeverity::Suspicious);
    }

    // 3. Mixed script / Homoglyph attack check (checked against both raw host and parsed domain)
    let raw_host = extract_raw_host(raw_url).unwrap_or_else(|| domain.clone());

    for label in raw_host.split('.').chain(domain.split('.')) {
        if check_mixed_script_homoglyphs(label) {
            reasons.push(format!(
                "Homograph attack detected in label '{label}': mixes Latin letters with visual lookalikes from other scripts."
            ));
            severity = severity.max(PhishingSeverity::Dangerous);
            break;
        }
    }

    // 4. Typosquatting / Brand spoofing check
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() >= 2 {
        let sld = labels[labels.len() - 2]; // second-level domain (e.g. "google" in "google.com")
        for &brand in PROTECTED_BRANDS {
            // Exact match on genuine domain is safe (e.g. google.com, paypal.com)
            if sld == brand {
                continue;
            }

            // A. Compound brand spoofing, e.g. "paypal-security", "verify-apple", "login-google"
            if is_compound_brand_spoof(sld, brand) {
                reasons.push(format!(
                    "Deceptive domain construction: '{sld}' combines protected brand '{brand}' with security/auth terms."
                ));
                severity = severity.max(PhishingSeverity::Dangerous);
            }

            // B. Leetspeak digit-for-letter substitution (e.g. amaz0n, paypa1, g00gle)
            let normalized_sld = normalize_digits_to_letters(sld);
            if normalized_sld == brand && sld != brand {
                reasons.push(format!(
                    "Deceptive character substitution (leetspeak spoofing): '{sld}' imitates protected brand '{brand}'."
                ));
                severity = severity.max(PhishingSeverity::Dangerous);
            } else {
                // C. Levenshtein edit distance = 1 (typosquatting like goggle, twiter)
                let dist = levenshtein_distance(sld, brand);
                if dist == 1 && sld.len() >= 4 {
                    reasons.push(format!(
                        "Potential typosquatting detected: '{sld}' is suspiciously similar (edit distance 1) to protected brand '{brand}'."
                    ));
                    severity = severity.max(PhishingSeverity::Dangerous);
                }
            }
        }
    }

    PhishingReport {
        url: raw_url.to_string(),
        domain,
        severity,
        reasons,
    }
}

fn extract_raw_host(raw_url: &str) -> Option<String> {
    let without_scheme = if let Some(pos) = raw_url.find("://") {
        &raw_url[pos + 3..]
    } else {
        raw_url
    };
    let host_part = without_scheme.split(&['/', '?', '#', ':'][..]).next()?;
    if host_part.is_empty() {
        None
    } else {
        Some(host_part.to_lowercase())
    }
}

fn is_loopback_or_local(host: &str) -> bool {
    host == "127.0.0.1" || host == "localhost" || host == "::1" || host.ends_with(".localhost")
}

fn check_mixed_script_homoglyphs(label: &str) -> bool {
    let mut has_ascii_letters = false;
    let mut has_homoglyph = false;

    for ch in label.chars() {
        if ch.is_ascii_alphabetic() {
            has_ascii_letters = true;
        } else if LOOKALIKE_HOMOGLYPHS.iter().any(|(h, _)| *h == ch) {
            has_homoglyph = true;
        }
    }

    has_ascii_letters && has_homoglyph
}

fn is_compound_brand_spoof(sld: &str, brand: &str) -> bool {
    if !sld.contains(brand) {
        return false;
    }

    let suspicious_tokens = [
        "login", "signin", "verify", "security", "update", "support",
        "account", "billing", "service", "auth", "portal", "secure",
    ];

    for tok in suspicious_tokens {
        if sld.contains(tok) {
            return true;
        }
    }

    // Hyphenated compound: brand-something or something-brand
    if sld.starts_with(&format!("{brand}-")) || sld.ends_with(&format!("-{brand}")) {
        return true;
    }

    false
}

/// Compute Levenshtein distance between two strings.
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let v1: Vec<char> = s1.chars().collect();
    let v2: Vec<char> = s2.chars().collect();

    let len1 = v1.len();
    let len2 = v2.len();

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if v1[i - 1] == v2[j - 1] { 0 } else { 1 };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[len1][len2]
}

fn normalize_digits_to_letters(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => 'l',
            '3' => 'e',
            '5' => 's',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genuine_brands_are_safe() {
        let rep = inspect_url_phishing("https://google.com/search?q=rust");
        assert_eq!(rep.severity, PhishingSeverity::Safe);
        assert!(rep.reasons.is_empty());

        let rep_paypal = inspect_url_phishing("https://paypal.com/signin");
        assert_eq!(rep_paypal.severity, PhishingSeverity::Safe);
    }

    #[test]
    fn homoglyph_attack_is_flagged_dangerous() {
        // Cyrillic 'о' (\u{043e}) inserted into "google.com"
        let spoofed = format!("https://g{}ogle.com", '\u{043E}');
        let rep = inspect_url_phishing(&spoofed);
        assert_eq!(rep.severity, PhishingSeverity::Dangerous);
        assert!(rep.reasons.iter().any(|r| r.contains("Homograph attack detected")));
    }

    #[test]
    fn compound_brand_spoof_is_flagged_dangerous() {
        let rep = inspect_url_phishing("https://paypal-security-update.com/login");
        assert_eq!(rep.severity, PhishingSeverity::Dangerous);
        assert!(rep.reasons.iter().any(|r| r.contains("Deceptive domain construction")));
    }

    #[test]
    fn typosquatting_and_leetspeak_are_flagged() {
        // Leetspeak spoofing (0 for o)
        let rep_leet = inspect_url_phishing("https://amaz0n.com/cart");
        assert_eq!(rep_leet.severity, PhishingSeverity::Dangerous);
        assert!(rep_leet.reasons.iter().any(|r| r.contains("leetspeak spoofing")));

        // Distance 1 typosquatting
        let rep_typo = inspect_url_phishing("https://goggle.com/search");
        assert_eq!(rep_typo.severity, PhishingSeverity::Dangerous);
        assert!(rep_typo.reasons.iter().any(|r| r.contains("typosquatting")));
    }

    #[test]
    fn punycode_domain_is_flagged_suspicious() {
        let rep = inspect_url_phishing("https://xn--e1afmkfd.xn--p1ai");
        assert!(rep.severity >= PhishingSeverity::Suspicious);
        assert!(rep.reasons.iter().any(|r| r.contains("Punycode")));
    }
}
