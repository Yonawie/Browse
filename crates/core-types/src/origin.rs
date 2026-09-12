use serde::{Deserialize, Serialize};
use std::fmt;

/// A web origin: `scheme://host[:port]`, lowercased, without path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Origin(String);

#[derive(Debug, thiserror::Error)]
pub enum OriginError {
    #[error("invalid url: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("url has no host: {0}")]
    NoHost(String),
}

impl Origin {
    pub fn parse(url: &str) -> Result<Origin, OriginError> {
        let parsed = url::Url::parse(url)?;
        Self::from_url(&parsed)
    }

    pub fn from_url(url: &url::Url) -> Result<Origin, OriginError> {
        let host = url.host_str().ok_or_else(|| OriginError::NoHost(url.to_string()))?;
        let mut s = format!("{}://{}", url.scheme(), host.to_ascii_lowercase());
        if let Some(port) = url.port() {
            s.push(':');
            s.push_str(&port.to_string());
        }
        Ok(Origin(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn scheme(&self) -> &str {
        self.0.split("://").next().unwrap_or("")
    }

    pub fn host(&self) -> &str {
        let rest = self.0.split("://").nth(1).unwrap_or("");
        rest.split(':').next().unwrap_or("")
    }

    pub fn is_https(&self) -> bool {
        self.scheme() == "https"
    }

    pub fn is_loopback(&self) -> bool {
        matches!(self.host(), "localhost" | "127.0.0.1" | "[::1]") || self.host().ends_with(".localhost")
    }

    /// Naive registrable domain (eTLD+1). Good enough for grouping and
    /// `never_remember` lists; a PSL-backed implementation can replace it.
    pub fn registrable_domain(&self) -> String {
        let host = self.host();
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() <= 2 {
            return host.to_string();
        }
        let second_level_tlds = ["co", "com", "org", "net", "gov", "edu", "ac"];
        let n = parts.len();
        if parts[n - 1].len() == 2 && second_level_tlds.contains(&parts[n - 2]) {
            parts[n - 3..].join(".")
        } else {
            parts[n - 2..].join(".")
        }
    }

    /// Does this origin match a scope pattern such as `https://github.com` or
    /// `https://*.github.com`? Scheme and port must match exactly; a leading
    /// `*.` matches any subdomain (but not the apex).
    pub fn matches_pattern(&self, pattern: &str) -> bool {
        if let Some(rest) = pattern.strip_prefix("https://*.") {
            let (pat_host, pat_port) = split_host_port(rest);
            let (host, port) = split_host_port(self.0.trim_start_matches("https://"));
            return self.is_https() && port == pat_port && host.ends_with(&format!(".{pat_host}"));
        }
        self.0 == pattern
    }
}

fn split_host_port(s: &str) -> (&str, Option<&str>) {
    match s.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !h.contains(']') => (h, Some(p)),
        _ => (s, None),
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes() {
        let o = Origin::parse("HTTPS://GitHub.com/bert/browse?x=1").unwrap();
        assert_eq!(o.as_str(), "https://github.com");
        assert_eq!(o.host(), "github.com");
        assert!(o.is_https());
        let p = Origin::parse("http://localhost:3000/").unwrap();
        assert_eq!(p.as_str(), "http://localhost:3000");
        assert!(p.is_loopback());
    }

    #[test]
    fn registrable_domain_handles_common_cases() {
        assert_eq!(Origin::parse("https://docs.github.com").unwrap().registrable_domain(), "github.com");
        assert_eq!(Origin::parse("https://www.bbc.co.uk").unwrap().registrable_domain(), "bbc.co.uk");
        assert_eq!(Origin::parse("https://example.org").unwrap().registrable_domain(), "example.org");
    }

    #[test]
    fn pattern_matching() {
        let o = Origin::parse("https://api.github.com").unwrap();
        assert!(o.matches_pattern("https://*.github.com"));
        assert!(!o.matches_pattern("https://github.com"));
        assert!(Origin::parse("https://github.com").unwrap().matches_pattern("https://github.com"));
        assert!(!Origin::parse("https://github.com").unwrap().matches_pattern("https://*.github.com"));
        assert!(!Origin::parse("http://api.github.com").unwrap().matches_pattern("https://*.github.com"));
    }
}
