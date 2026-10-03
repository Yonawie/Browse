//! Scriptable in-memory engine for tests and for running the agent runtime
//! without a real browser.

use crate::*;
use core_types::{ContentChunk, ElementRef, ElementState, InteractiveElement, PageKind, PageMeta, Sensitivity};
use std::collections::HashMap;
use std::sync::Mutex;

struct WebViewState {
    profile: ProfileKind,
    url: String,
    imported_origins: Vec<Origin>,
}

/// A page the mock can serve.
#[derive(Debug, Clone)]
pub struct MockPage {
    pub url: String,
    pub title: String,
    pub interactive: Vec<InteractiveElement>,
    pub content: Vec<ContentChunk>,
    pub hidden_text_signals: Vec<String>,
    pub sensitivity: Sensitivity,
}

impl MockPage {
    pub fn simple(url: &str, title: &str, text: &str) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            interactive: vec![],
            content: vec![ContentChunk {
                obs_id: "c0".into(),
                heading_path: None,
                text: text.into(),
                char_start: 0,
                char_end: text.chars().count() as u32,
                suspect_injection: false,
            }],
            hidden_text_signals: vec![],
            sensitivity: Sensitivity::Public,
        }
    }

    pub fn with_element(mut self, id: u64, role: &str, name: &str) -> Self {
        self.interactive.push(InteractiveElement {
            element_ref: ElementRef { id, path: format!("{role}/{name}/0") },
            role: role.into(),
            name: name.into(),
            state: ElementState::default(),
            value: None,
            href: None,
            input_type: None,
            bbox: None,
            in_viewport: true,
            landmark: Some("main".into()),
            consequential_hint: false,
        });
        self
    }

    pub fn with_hidden_injection(mut self, text: &str) -> Self {
        self.hidden_text_signals.push(text.into());
        self
    }
}

pub struct MockEngine {
    pages: Mutex<HashMap<String, MockPage>>,
    webviews: Mutex<HashMap<Id, WebViewState>>,
    pub actions: Mutex<Vec<(Id, Action)>>,
    events: Mutex<Vec<EngineEvent>>,
    counter: Mutex<u64>,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            pages: Mutex::new(HashMap::new()),
            webviews: Mutex::new(HashMap::new()),
            actions: Mutex::new(vec![]),
            events: Mutex::new(vec![]),
            counter: Mutex::new(0),
        }
    }

    pub fn add_page(&self, page: MockPage) {
        self.pages.lock().unwrap().insert(page.url.clone(), page);
    }

    fn page_for(&self, url: &str) -> Option<MockPage> {
        self.pages.lock().unwrap().get(url).cloned()
    }
}

#[async_trait]
impl EngineAdapter for MockEngine {
    fn backend_name(&self) -> &'static str {
        "mock"
    }

    async fn create_webview(&self, opts: WebViewOptions) -> Result<Id> {
        let mut c = self.counter.lock().unwrap();
        *c += 1;
        let id = format!("wv-{}", *c);
        self.webviews.lock().unwrap().insert(
            id.clone(),
            WebViewState { profile: opts.profile, url: "about:blank".into(), imported_origins: vec![] },
        );
        Ok(id)
    }

    async fn close_webview(&self, webview: &Id) -> Result<()> {
        self.webviews.lock().unwrap().remove(webview).ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))?;
        self.events.lock().unwrap().push(EngineEvent::Closed { webview: webview.clone() });
        Ok(())
    }

    async fn navigate(&self, webview: &Id, url: &str) -> Result<()> {
        let mut wvs = self.webviews.lock().unwrap();
        let wv = wvs.get_mut(webview).ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))?;
        if self.page_for(url).is_none() {
            return Err(EngineError::Navigation(format!("mock has no page for {url}")));
        }
        wv.url = url.to_string();
        let title = self.page_for(url).map(|p| p.title).unwrap_or_default();
        self.events.lock().unwrap().push(EngineEvent::Loaded { webview: webview.clone(), url: url.into(), title });
        Ok(())
    }

    async fn observe(&self, webview: &Id) -> Result<Observation> {
        let url = self
            .webviews
            .lock()
            .unwrap()
            .get(webview)
            .ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))?
            .url
            .clone();
        let page = self.page_for(&url).ok_or_else(|| EngineError::Navigation(format!("no page at {url}")))?;
        let origin = Origin::parse(&url).map_err(|e| EngineError::Backend(e.to_string()))?;
        let serialized =
            format!("{}|{}|{}", url, page.title, page.content.iter().map(|c| c.text.as_str()).collect::<String>());
        Ok(Observation {
            page: PageMeta {
                url: url.clone(),
                origin,
                title: page.title.clone(),
                lang: Some("en".into()),
                page_kind: PageKind::Unknown,
                untrusted: true,
                frame_id: "main".into(),
                snapshot_hash: format!("{:016x}", fnv1a(&serialized)),
                sensitivity: page.sensitivity,
                has_session: false,
            },
            interactive: page.interactive.clone(),
            content: page.content.clone(),
            tools: vec![],
            hidden_text_signals: page.hidden_text_signals.clone(),
            approx_tokens: (serialized.len() / 4) as u32,
            captured_at: 0,
        })
    }

    async fn act(&self, webview: &Id, action: Action) -> Result<ActionResult> {
        let url = self
            .webviews
            .lock()
            .unwrap()
            .get(webview)
            .ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))?
            .url
            .clone();
        if let Action::Click { target }
        | Action::Type { target, .. }
        | Action::Select { target, .. }
        | Action::Check { target, .. } = &action
        {
            let page = self.page_for(&url).ok_or_else(|| EngineError::Navigation(url.clone()))?;
            if !page.interactive.iter().any(|e| e.element_ref.id == target.id) {
                return Err(EngineError::NoSuchElement(target.clone()));
            }
        }
        if let Action::Navigate { url: to } = &action {
            self.navigate(webview, to).await?;
        }
        self.actions.lock().unwrap().push((webview.clone(), action));
        let url = self.webviews.lock().unwrap().get(webview).map(|w| w.url.clone()).unwrap_or_default();
        Ok(ActionResult { ok: true, message: None, url, output: None })
    }

    async fn import_session(&self, target: &Id, origin: &Origin, cookies: Vec<SessionCookie>) -> Result<usize> {
        let mut wvs = self.webviews.lock().unwrap();
        let wv = wvs.get_mut(target).ok_or_else(|| EngineError::NoSuchWebView(target.clone()))?;
        if !matches!(wv.profile, ProfileKind::Agent { .. }) {
            return Err(EngineError::Blocked("session import is only allowed into agent profiles".into()));
        }
        wv.imported_origins.push(origin.clone());
        Ok(cookies.len())
    }

    async fn export_session(&self, _user_webview: &Id, origin: &Origin) -> Result<Vec<SessionCookie>> {
        Ok(vec![SessionCookie {
            name: "sid".into(),
            value: "mock".into(),
            domain: origin.host().into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            expires: None,
        }])
    }

    async fn screenshot(&self, _webview: &Id) -> Result<Vec<u8>> {
        Ok(vec![])
    }

    async fn poll_events(&self) -> Result<Vec<EngineEvent>> {
        Ok(std::mem::take(&mut *self.events.lock().unwrap()))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::CHROMIUM
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_round_trip_and_profile_guard() {
        let engine = MockEngine::new();
        engine.add_page(MockPage::simple("https://a.example/", "A", "Hello.").with_element(1, "button", "Go"));
        let user = engine
            .create_webview(WebViewOptions {
                profile: ProfileKind::User,
                inject_sensor: true,
                observe_iframe_origins: vec![],
                headless: false,
            })
            .await
            .unwrap();
        let agent = engine.create_webview(WebViewOptions::agent("s1")).await.unwrap();
        engine.navigate(&agent, "https://a.example/").await.unwrap();
        let obs = engine.observe(&agent).await.unwrap();
        assert!(obs.page.untrusted);
        assert_eq!(obs.interactive.len(), 1);
        let r = engine
            .act(&agent, Action::Click { target: ElementRef { id: 1, path: "button/Go/0".into() } })
            .await
            .unwrap();
        assert!(r.ok);
        assert!(matches!(
            engine.act(&agent, Action::Click { target: ElementRef { id: 99, path: String::new() } }).await,
            Err(EngineError::NoSuchElement(_))
        ));
        let origin = Origin::parse("https://a.example").unwrap();
        let cookies = engine.export_session(&user, &origin).await.unwrap();
        assert!(matches!(engine.import_session(&user, &origin, cookies.clone()).await, Err(EngineError::Blocked(_))));
        assert_eq!(engine.import_session(&agent, &origin, cookies).await.unwrap(), 1);
        assert!(!engine.poll_events().await.unwrap().is_empty());
    }
}
