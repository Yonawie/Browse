use serde::{Deserialize, Serialize};

/// Clean article representation for distraction-free reading mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderArticle {
    pub url: String,
    pub title: String,
    pub byline: Option<String>,
    pub published_time: Option<String>,
    pub reading_time_minutes: u32,
    pub word_count: usize,
    pub clean_text: String,
    pub clean_html: String,
    pub excerpt: String,
}

/// Heuristics to extract clean article content from raw web page text or HTML.
pub fn extract_reader_article(url: &str, title: &str, raw_text: &str) -> ReaderArticle {
    let clean_title = if title.trim().is_empty() {
        // Fallback: derive title from URL
        let without_proto = url.split("://").nth(1).unwrap_or(url);
        let path = without_proto.split('/').next_back().unwrap_or("Article");
        path.replace(['-', '_'], " ")
    } else {
        // Remove trailing brand suffixes like " | The Verge" or " - New York Times"
        let parts: Vec<&str> = title.split(&['|', '—', '-'][..]).collect();
        if parts.len() > 1 && parts[0].trim().len() > 10 {
            parts[0].trim().to_string()
        } else {
            title.trim().to_string()
        }
    };

    let mut clean_paragraphs = Vec::new();
    let mut byline: Option<String> = None;
    let mut published_time: Option<String> = None;

    let boilerplate_keywords = [
        "all rights reserved",
        "subscribe to our newsletter",
        "sign in to read",
        "cookie preferences",
        "privacy policy",
        "terms of service",
        "share on twitter",
        "share on facebook",
        "advertisement",
        "sponsored content",
        "leave a comment",
    ];

    for raw_para in raw_text.split('\n') {
        let p = raw_para.trim();
        if p.is_empty() {
            continue;
        }

        let p_lower = p.to_lowercase();

        // Detect author byline
        if byline.is_none()
            && (p_lower.starts_with("by ") || p_lower.starts_with("author:") || p_lower.starts_with("written by "))
            && p.len() < 80
        {
            byline = Some(p.to_string());
            continue;
        }

        // Detect publication date
        if published_time.is_none()
            && (p_lower.starts_with("published:") || p_lower.starts_with("date:") || p_lower.starts_with("posted on"))
            && p.len() < 80
        {
            published_time = Some(p.to_string());
            continue;
        }

        // Skip boilerplate paragraphs
        if boilerplate_keywords.iter().any(|b| p_lower.contains(b)) {
            continue;
        }

        // Keep meaningful paragraphs (at least 20 chars or heading)
        if p.len() >= 20 || p.starts_with('#') {
            clean_paragraphs.push(p);
        }
    }

    let clean_text = clean_paragraphs.join("\n\n");
    let word_count = clean_text.split_whitespace().count();
    let reading_time_minutes = ((word_count as f32 / 200.0).ceil() as u32).max(1);

    // Generate excerpt from the first paragraph
    let excerpt = clean_paragraphs
        .first()
        .map(|p| {
            if p.chars().count() > 200 {
                format!("{}...", p.chars().take(200).collect::<String>())
            } else {
                p.to_string()
            }
        })
        .unwrap_or_default();

    // Render clean HTML
    let mut clean_html = String::new();
    clean_html.push_str(&format!("<article class=\"reader-content\">\n<h1>{}</h1>\n", html_escape(&clean_title)));

    if let Some(ref auth) = byline {
        clean_html.push_str(&format!("<div class=\"reader-byline\">{}</div>\n", html_escape(auth)));
    }
    if let Some(ref date) = published_time {
        clean_html.push_str(&format!("<div class=\"reader-date\">{}</div>\n", html_escape(date)));
    }
    clean_html.push_str(&format!(
        "<div class=\"reader-meta\">{} min read &bull; {} words</div>\n<hr/>\n",
        reading_time_minutes, word_count
    ));

    for p in &clean_paragraphs {
        if let Some(h2) = p.strip_prefix("## ") {
            clean_html.push_str(&format!("<h2>{}</h2>\n", html_escape(h2)));
        } else if let Some(h1) = p.strip_prefix("# ") {
            clean_html.push_str(&format!("<h2>{}</h2>\n", html_escape(h1)));
        } else {
            clean_html.push_str(&format!("<p>{}</p>\n", html_escape(p)));
        }
    }
    clean_html.push_str("</article>");

    ReaderArticle {
        url: url.to_string(),
        title: clean_title,
        byline,
        published_time,
        reading_time_minutes,
        word_count,
        clean_text,
        clean_html,
        excerpt,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_clean_reader_article_and_strips_boilerplate() {
        let raw = r#"
By Jane Doe
Published: October 5, 2026

Rust 2026 introduces major performance improvements and ergonomics for asynchronous programming across all supported platforms.

Subscribe to our newsletter for more updates!

The new type system features allow compiler developers to catch lifetime ambiguities earlier during compile-time checks without runtime penalties.

All rights reserved. Cookie preferences.
"#;

        let article = extract_reader_article("https://news.example.com/rust-2026", "Rust 2026 Release - Tech News", raw);

        assert_eq!(article.title, "Rust 2026 Release");
        assert_eq!(article.byline, Some("By Jane Doe".into()));
        assert_eq!(article.published_time, Some("Published: October 5, 2026".into()));
        assert_eq!(article.reading_time_minutes, 1);
        assert!(article.clean_text.contains("Rust 2026 introduces major performance"));
        assert!(article.clean_text.contains("The new type system features"));
        assert!(!article.clean_text.contains("Subscribe to our newsletter"));
        assert!(!article.clean_text.contains("All rights reserved"));
        assert!(article.clean_html.contains("<h1>Rust 2026 Release</h1>"));
    }
}
