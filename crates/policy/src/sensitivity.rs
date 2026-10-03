//! Deterministic sensitivity classification by *source* (ADR-007).

use crate::PolicyConfig;
use core_types::{Origin, Sensitivity};

/// Facts about a page that the engine adapter can observe without a model.
#[derive(Debug, Clone, Default)]
pub struct PageSignals {
    /// The profile holds cookies for this origin (any cookie with `HttpOnly`
    /// or a name matching common session patterns).
    pub has_session_cookie: bool,
    /// Response carried `Cache-Control: private` or `no-store`.
    pub cache_private: bool,
    /// The page is being viewed in the user's (not the agent's) profile.
    pub in_user_profile: bool,
    /// The sensor found masked inputs (password / card).
    pub has_masked_inputs: bool,
}

pub fn classify_page(origin: &Origin, signals: &PageSignals, config: &PolicyConfig) -> Sensitivity {
    let domain = origin.registrable_domain();
    let host = origin.host();
    if config.private_domains.iter().any(|d| d == host || d == &domain || host.ends_with(&format!(".{d}"))) {
        return Sensitivity::Private;
    }
    if config.public_override_domains.iter().any(|d| d == host || d == &domain) {
        return Sensitivity::Public;
    }
    if signals.has_session_cookie || signals.cache_private {
        return Sensitivity::Private;
    }
    if signals.has_masked_inputs {
        // Login/checkout pages without a session yet: user is about to type secrets.
        return Sensitivity::Personal;
    }
    Sensitivity::Public
}

/// Very small PII detector for user-authored text. Deterministic; errs toward
/// `Personal`. A model is never used for this.
pub fn classify_user_text(text: &str) -> Sensitivity {
    let t = text.to_lowercase();
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    let looks_like_card = digits.len() >= 13
        && digits.len() <= 19
        && t.split(|c: char| !c.is_ascii_digit()).filter(|s| s.len() == 4).count() >= 3;
    let looks_like_phone = digits.len() >= 10 && (t.contains('+') || t.contains('(') || t.matches('-').count() >= 2);
    let has_email = t.contains('@') && t.contains('.') && t.split('@').nth(1).map(|d| d.contains('.')).unwrap_or(false);
    let iban = t.split_whitespace().any(|w| {
        w.len() >= 15
            && w.chars().take(2).all(|c| c.is_ascii_alphabetic())
            && w.chars().skip(2).take(2).all(|c| c.is_ascii_digit())
    });
    if looks_like_card || iban {
        return Sensitivity::Private;
    }
    if looks_like_phone || has_email {
        return Sensitivity::Personal;
    }
    Sensitivity::Public
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_domains_win() {
        let cfg = PolicyConfig::default();
        let o = Origin::parse("https://mail.google.com/mail/u/0").unwrap();
        assert_eq!(classify_page(&o, &PageSignals::default(), &cfg), Sensitivity::Private);
    }

    #[test]
    fn session_makes_private_unless_overridden() {
        let mut cfg = PolicyConfig::default();
        let o = Origin::parse("https://wiki.example.org/page").unwrap();
        let s = PageSignals { has_session_cookie: true, ..Default::default() };
        assert_eq!(classify_page(&o, &s, &cfg), Sensitivity::Private);
        cfg.public_override_domains.push("example.org".into());
        assert_eq!(classify_page(&o, &s, &cfg), Sensitivity::Public);
    }

    #[test]
    fn user_text_pii() {
        assert_eq!(classify_user_text("book me a table for two"), Sensitivity::Public);
        assert_eq!(classify_user_text("email bert@example.com about it"), Sensitivity::Personal);
        assert_eq!(classify_user_text("call +1 (555) 010-2233"), Sensitivity::Personal);
        assert_eq!(classify_user_text("card 4111 1111 1111 1111"), Sensitivity::Private);
        assert_eq!(classify_user_text("iban DE89370400440532013000"), Sensitivity::Private);
    }
}
