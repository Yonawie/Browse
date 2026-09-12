//! Cheap page-kind detection from an observation (no model).

use core_types::{Observation, PageKind};

pub fn detect_page_kind(obs: &Observation) -> PageKind {
    let url = obs.page.url.to_lowercase();
    let title = obs.page.title.to_lowercase();
    let roles = |r: &str| obs.interactive.iter().filter(|e| e.role == r).count();
    let masked = obs.interactive.iter().filter(|e| e.state.masked).count();
    let text_len: usize = obs.content.iter().map(|c| c.text.chars().count()).sum();
    let names: Vec<String> = obs.interactive.iter().map(|e| e.name.to_lowercase()).collect();
    let any_name = |words: &[&str]| names.iter().any(|n| words.iter().any(|w| n.contains(w)));

    if ["/checkout", "/payment", "/cart"].iter().any(|m| url.contains(m))
        || obs.interactive.iter().any(|e| e.consequential_hint && e.state.masked)
    {
        return PageKind::Checkout;
    }
    if masked >= 1
        && roles("textbox") <= 3
        && (url.contains("login")
            || url.contains("signin")
            || url.contains("auth")
            || any_name(&["sign in", "log in", "войти"]))
    {
        return PageKind::Auth;
    }
    if url.contains("/search")
        || url.contains("?q=")
        || url.contains("&q=")
        || title.contains(" - search")
        || title.contains("поиск")
    {
        return PageKind::Search;
    }
    if ["github.com", "gitlab.com", "/blob/", "/pull/", "/commit/"].iter().any(|m| url.contains(m)) {
        return PageKind::Code;
    }
    if ["/docs", "docs.", "/documentation", "/reference", "/api/"].iter().any(|m| url.contains(m)) {
        return PageKind::Doc;
    }
    if any_name(&["add to cart", "buy now", "в корзину", "купить", "in den warenkorb", "ajouter au panier"])
    {
        return PageKind::Product;
    }
    if ["youtube.com/watch", "vimeo.com", "/video/", "/watch"].iter().any(|m| url.contains(m)) {
        return PageKind::Media;
    }
    let inputs = roles("textbox") + roles("combobox") + roles("checkbox") + roles("radio");
    if inputs >= 4 && text_len < 4000 {
        return PageKind::Form;
    }
    if text_len >= 1500 && roles("button") <= 15 {
        return PageKind::Article;
    }
    if roles("button") + roles("menuitem") + roles("tab") >= 20 {
        return PageKind::App;
    }
    PageKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::*;

    fn obs(url: &str, interactive: Vec<InteractiveElement>, text: &str) -> Observation {
        Observation {
            page: PageMeta {
                url: url.into(),
                origin: Origin::parse(url).unwrap(),
                title: "t".into(),
                lang: None,
                page_kind: PageKind::Unknown,
                untrusted: true,
                frame_id: "main".into(),
                snapshot_hash: String::new(),
                sensitivity: Sensitivity::Public,
                has_session: false,
            },
            interactive,
            content: vec![ContentChunk {
                obs_id: "c0".into(),
                heading_path: None,
                text: text.into(),
                char_start: 0,
                char_end: text.len() as u32,
                suspect_injection: false,
            }],
            tools: vec![],
            hidden_text_signals: vec![],
            approx_tokens: 0,
            captured_at: 0,
        }
    }

    fn el(role: &str, name: &str, masked: bool) -> InteractiveElement {
        InteractiveElement {
            element_ref: ElementRef { id: 1, path: format!("{role}/{name}") },
            role: role.into(),
            name: name.into(),
            state: ElementState { masked, ..Default::default() },
            value: None,
            href: None,
            input_type: None,
            bbox: None,
            in_viewport: true,
            landmark: None,
            consequential_hint: false,
        }
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(
            detect_page_kind(&obs("https://shop.example/p/1", vec![el("button", "Add to cart", false)], "")),
            PageKind::Product
        );
        assert_eq!(
            detect_page_kind(&obs(
                "https://a.example/login",
                vec![el("textbox", "Email", false), el("textbox", "Password", true), el("button", "Sign in", false)],
                ""
            )),
            PageKind::Auth
        );
        assert_eq!(detect_page_kind(&obs("https://github.com/x/y/pull/1", vec![], "")), PageKind::Code);
        assert_eq!(
            detect_page_kind(&obs("https://blog.example/post", vec![], &"word ".repeat(400))),
            PageKind::Article
        );
        assert_eq!(detect_page_kind(&obs("https://s.example/search?q=x", vec![], "")), PageKind::Search);
    }
}
