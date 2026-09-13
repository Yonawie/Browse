//! `EngineAdapter` backend for Chromium over the DevTools Protocol (ADR-001
//! backend `cdp-remote`). The same protocol surface is what CEF's
//! `SendDevToolsMessage` and WebView2's `CallDevToolsProtocolMethod` expose, so
//! this code is the reference implementation for the Chromium-based shells.
//!
//! Security-relevant choices:
//! * the page sensor is injected into a named **isolated world**; the page's
//!   scripts cannot see `BrowseSensor`, its `ref` table or the observation;
//! * every element interaction resolves a `ref` inside that world and drives
//!   the element with **trusted input events** (`Input.dispatch*`), never with
//!   selectors or `element.click()` authored by a model;
//! * agent and private profiles are separate **browser contexts** (own cookie
//!   jar, storage, cache); session import copies cookies for exactly one origin.

pub mod connection;
pub mod launcher;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use connection::{CdpEvent, Connection, Cursor};
use core_types::{Id, Observation, Origin, Sensitivity};
use engine_adapter::{
    Action, ActionResult, Capabilities, EngineAdapter, EngineError, EngineEvent, ProfileKind, Result, SessionCookie,
    WebViewOptions,
};
use serde_json::{json, Value};

/// The compiled page sensor (`sensor/dist/sensor.iife.js`, built with `npm run build`).
pub const SENSOR_JS: &str = include_str!("../../../sensor/dist/sensor.iife.js");
/// Name of the isolated world the sensor lives in.
pub const SENSOR_WORLD: &str = "browse-sensor";

#[derive(Debug, thiserror::Error)]
pub enum CdpError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("cdp error {code}: {message}{}", data.as_ref().map(|d| format!(" ({d})")).unwrap_or_default())]
    Protocol { code: i64, message: String, data: Option<String> },
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("launch: {0}")]
    Launch(String),
}

/// Per-call CDP timeout.
pub const CALL_TIMEOUT_MS: u64 = 60_000;

impl From<CdpError> for EngineError {
    fn from(e: CdpError) -> Self {
        match e {
            CdpError::Timeout(_) => EngineError::Timeout(CALL_TIMEOUT_MS),
            other => EngineError::Backend(other.to_string()),
        }
    }
}

pub type SensitivityClassifier = Arc<dyn Fn(&Origin, bool) -> Sensitivity + Send + Sync>;

struct WebViewState {
    target_id: String,
    session_id: String,
    context_id: Option<String>,
    profile: ProfileKind,
    url: String,
    title: String,
    /// requestId → method, for `EngineEvent::Request`.
    inflight: HashMap<String, String>,
}

pub struct CdpEngine {
    conn: Arc<Connection>,
    webviews: Mutex<HashMap<Id, WebViewState>>,
    contexts: Mutex<HashMap<String, String>>,
    classifier: SensitivityClassifier,
    /// Kept alive so the browser process is killed when the engine drops.
    _process: Option<launcher::ChromeProcess>,
    extra_events: Mutex<Vec<EngineEvent>>,
    capabilities: Capabilities,
}

/// Sensitivity classifier built from a policy config (ADR-007: by source, never by a model).
pub fn policy_classifier(config: policy::PolicyConfig) -> SensitivityClassifier {
    Arc::new(move |origin, has_session| {
        let signals = policy::sensitivity::PageSignals { has_session_cookie: has_session, ..Default::default() };
        policy::sensitivity::classify_page(origin, &signals, &config)
    })
}

impl CdpEngine {
    /// Launch a local Chromium and connect to it.
    pub async fn launch(opts: launcher::LaunchOptions) -> Result<Arc<CdpEngine>> {
        Self::launch_with(opts, policy_classifier(policy::PolicyConfig::default())).await
    }

    pub async fn launch_with(
        opts: launcher::LaunchOptions,
        classifier: SensitivityClassifier,
    ) -> Result<Arc<CdpEngine>> {
        let process = launcher::launch(opts).await.map_err(EngineError::from)?;
        let conn = Connection::connect(&process.ws_url).await.map_err(EngineError::from)?;
        Self::finish(conn, Some(process), classifier).await
    }

    /// Attach to an already running Chromium-based engine (e.g. a WebView2 or
    /// CEF host that exposed `--remote-debugging-port`).
    pub async fn connect(ws_url: &str, classifier: SensitivityClassifier) -> Result<Arc<CdpEngine>> {
        let conn = Connection::connect(ws_url).await.map_err(EngineError::from)?;
        Self::finish(conn, None, classifier).await
    }

    async fn finish(
        conn: Arc<Connection>,
        process: Option<launcher::ChromeProcess>,
        classifier: SensitivityClassifier,
    ) -> Result<Arc<CdpEngine>> {
        conn.call(None, "Target.setDiscoverTargets", json!({ "discover": true })).await.map_err(EngineError::from)?;
        Ok(Arc::new(CdpEngine {
            conn,
            webviews: Mutex::new(HashMap::new()),
            contexts: Mutex::new(HashMap::new()),
            classifier,
            _process: process,
            extra_events: Mutex::new(vec![]),
            capabilities: Capabilities { webextensions: false, ..Capabilities::CHROMIUM },
        }))
    }

    fn session_of(&self, webview: &Id) -> Result<String> {
        self.webviews
            .lock()
            .unwrap()
            .get(webview)
            .map(|w| w.session_id.clone())
            .ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))
    }

    async fn call(&self, session: &str, method: &str, params: Value) -> Result<Value> {
        Ok(self.conn.call(Some(session), method, params).await?)
    }

    async fn browser_context_for(&self, profile: &ProfileKind) -> Result<Option<String>> {
        let key = match profile {
            ProfileKind::User => return Ok(None),
            ProfileKind::Agent { session_id } => format!("agent:{session_id}"),
            ProfileKind::Private => "private".to_string(),
        };
        if let Some(id) = self.contexts.lock().unwrap().get(&key) {
            return Ok(Some(id.clone()));
        }
        let r = self.conn.call(None, "Target.createBrowserContext", json!({ "disposeOnDetach": false })).await?;
        let id = r["browserContextId"].as_str().unwrap_or_default().to_string();
        self.contexts.lock().unwrap().insert(key, id.clone());
        Ok(Some(id))
    }

    async fn main_frame_id(&self, session: &str) -> Result<String> {
        let tree = self.call(session, "Page.getFrameTree", json!({})).await?;
        Ok(tree["frameTree"]["frame"]["id"].as_str().unwrap_or_default().to_string())
    }

    /// Execution context of the sensor's isolated world in the main frame.
    async fn sensor_context(&self, session: &str) -> Result<i64> {
        let frame_id = self.main_frame_id(session).await?;
        let r = self
            .call(
                session,
                "Page.createIsolatedWorld",
                json!({ "frameId": frame_id, "worldName": SENSOR_WORLD, "grantUniveralAccess": false }),
            )
            .await?;
        let ctx = r["executionContextId"].as_i64().ok_or_else(|| EngineError::Backend("no isolated world".into()))?;
        let present = self
            .call(
                session,
                "Runtime.evaluate",
                json!({ "expression": "typeof BrowseSensor === 'object'", "contextId": ctx, "returnByValue": true }),
            )
            .await?;
        if present["result"]["value"].as_bool() != Some(true) {
            // Document was created before the script was registered (e.g. attach to an existing tab).
            self.call(session, "Runtime.evaluate", json!({ "expression": SENSOR_JS, "contextId": ctx })).await?;
        }
        Ok(ctx)
    }

    async fn eval_sensor(&self, session: &str, expression: &str, by_value: bool) -> Result<Value> {
        let ctx = self.sensor_context(session).await?;
        let r = self
            .call(
                session,
                "Runtime.evaluate",
                json!({ "expression": expression, "contextId": ctx, "returnByValue": by_value, "awaitPromise": true }),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(EngineError::Backend(format!(
                "sensor exception: {}",
                ex["exception"]["description"].as_str().unwrap_or("unknown")
            )));
        }
        Ok(r["result"].clone())
    }

    /// Evaluate JavaScript in the page's main world (no access to the sensor).
    pub async fn evaluate(&self, webview: &Id, expression: &str) -> Result<Value> {
        let session = self.session_of(webview)?;
        let r = self
            .call(
                &session,
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(EngineError::Backend(format!(
                "evaluate exception: {}",
                ex["exception"]["description"].as_str().unwrap_or("unknown")
            )));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Serialized HTML of the current document (for Page Intelligence fallbacks).
    pub async fn html(&self, webview: &Id) -> Result<String> {
        Ok(self.evaluate(webview, "document.documentElement.outerHTML").await?.as_str().unwrap_or("").to_string())
    }

    /// Full accessibility tree (`Accessibility.getFullAXTree`).
    pub async fn ax_tree(&self, webview: &Id) -> Result<Value> {
        let session = self.session_of(webview)?;
        self.call(&session, "Accessibility.enable", json!({})).await?;
        let r = self.call(&session, "Accessibility.getFullAXTree", json!({})).await?;
        Ok(r["nodes"].clone())
    }

    pub fn current_url(&self, webview: &Id) -> Option<String> {
        self.webviews.lock().unwrap().get(webview).map(|w| w.url.clone())
    }

    async fn resolve_ref(&self, session: &str, id: u64) -> Result<String> {
        let r = self.eval_sensor(session, &format!("BrowseSensor.resolveRef({id})"), false).await?;
        match r.get("objectId").and_then(Value::as_str) {
            Some(o) => Ok(o.to_string()),
            None => Err(EngineError::NoSuchElement(core_types::ElementRef { id, path: String::new() })),
        }
    }

    async fn center_of(&self, session: &str, object_id: &str) -> Result<(f64, f64)> {
        let _ = self.call(session, "DOM.scrollIntoViewIfNeeded", json!({ "objectId": object_id })).await;
        let r = self.call(session, "DOM.getBoxModel", json!({ "objectId": object_id })).await?;
        let q = r["model"]["content"].as_array().cloned().unwrap_or_default();
        if q.len() < 8 {
            return Err(EngineError::Backend("element has no box".into()));
        }
        let xs: Vec<f64> = [0, 2, 4, 6].iter().map(|i| q[*i].as_f64().unwrap_or(0.0)).collect();
        let ys: Vec<f64> = [1, 3, 5, 7].iter().map(|i| q[*i].as_f64().unwrap_or(0.0)).collect();
        Ok((xs.iter().sum::<f64>() / 4.0, ys.iter().sum::<f64>() / 4.0))
    }

    async fn mouse(&self, session: &str, kind: &str, x: f64, y: f64) -> Result<()> {
        let mut p = json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 });
        if kind == "mouseMoved" {
            p["button"] = json!("none");
        }
        self.call(session, "Input.dispatchMouseEvent", p).await?;
        Ok(())
    }

    async fn key(&self, session: &str, key: &str) -> Result<()> {
        let (code, vk, text) = match key {
            "Enter" => ("Enter", 13, Some("\r")),
            "Tab" => ("Tab", 9, None),
            "Escape" => ("Escape", 27, None),
            "Backspace" => ("Backspace", 8, None),
            "ArrowDown" => ("ArrowDown", 40, None),
            "ArrowUp" => ("ArrowUp", 38, None),
            "PageDown" => ("PageDown", 34, None),
            "PageUp" => ("PageUp", 33, None),
            "Space" | " " => ("Space", 32, Some(" ")),
            other => return Err(EngineError::Backend(format!("unsupported key `{other}`"))),
        };
        let mut down = json!({ "type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk });
        if let Some(t) = text {
            down["text"] = json!(t);
        }
        self.call(session, "Input.dispatchKeyEvent", down).await?;
        self.call(
            session,
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk }),
        )
        .await?;
        Ok(())
    }

    /// Wait for a navigation that an action issued after `since` may have
    /// triggered, then for the network to go quiet. Returns quickly when
    /// nothing happened.
    async fn settle(&self, session: &str, since: Cursor) {
        let started = self
            .conn
            .wait_for(Some(session), since, Duration::from_millis(600), |e| {
                e.method == "Page.frameStartedNavigating" || e.method == "Page.frameScheduledNavigation"
            })
            .await
            .is_some();
        if started {
            let done = self.wait_loaded(session, since, None, Duration::from_secs(15)).await;
            if done.as_ref().map(|e| e.method == "Page.frameNavigated").unwrap_or(false) {
                return; // restored from cache: no network activity to wait for
            }
        }
        self.wait_idle(session, since, Duration::from_millis(800)).await;
    }

    /// Wait for the document to finish loading. A history navigation served
    /// from the back-forward cache fires no `load`; it is complete at
    /// `frameNavigated{type: BackForwardCacheRestore}`.
    async fn wait_loaded(
        &self,
        session: &str,
        since: Cursor,
        loader_id: Option<&str>,
        timeout: Duration,
    ) -> Option<CdpEvent> {
        let loader = loader_id.map(str::to_string);
        self.conn
            .wait_for(Some(session), since, timeout, |e| {
                (e.method == "Page.lifecycleEvent"
                    && e.params["name"] == "load"
                    && loader.as_ref().map(|l| e.params["loaderId"] == *l).unwrap_or(true))
                    || (e.method == "Page.frameNavigated"
                        && e.params["type"] == "BackForwardCacheRestore"
                        && e.params["frame"].get("parentId").is_none())
            })
            .await
    }

    async fn wait_idle(&self, session: &str, since: Cursor, timeout: Duration) {
        let _ = self
            .conn
            .wait_for(Some(session), since, timeout, |e| {
                e.method == "Page.lifecycleEvent"
                    && (e.params["name"] == "networkIdle" || e.params["name"] == "networkAlmostIdle")
            })
            .await;
    }

    fn map_events(&self, webview: &Id, state: &mut WebViewState, events: Vec<CdpEvent>) -> Vec<EngineEvent> {
        let mut out = vec![];
        for ev in events {
            match ev.method.as_str() {
                "Page.frameNavigated" if ev.params["frame"].get("parentId").is_none() => {
                    let url = ev.params["frame"]["url"].as_str().unwrap_or("").to_string();
                    state.url = url.clone();
                    out.push(EngineEvent::NavigationStarted { webview: webview.clone(), url });
                }
                "Page.loadEventFired" => out.push(EngineEvent::Loaded {
                    webview: webview.clone(),
                    url: state.url.clone(),
                    title: state.title.clone(),
                }),
                "Network.requestWillBeSent" => {
                    if let (Some(id), Some(m)) =
                        (ev.params["requestId"].as_str(), ev.params["request"]["method"].as_str())
                    {
                        state.inflight.insert(id.to_string(), m.to_string());
                    }
                }
                "Network.responseReceived" => {
                    let id = ev.params["requestId"].as_str().unwrap_or("");
                    let method = state.inflight.remove(id).unwrap_or_else(|| "GET".into());
                    out.push(EngineEvent::Request {
                        webview: webview.clone(),
                        method,
                        url: ev.params["response"]["url"].as_str().unwrap_or("").to_string(),
                        status: ev.params["response"]["status"].as_u64().map(|s| s as u16),
                    });
                }
                "Runtime.consoleAPICalled" => {
                    let text = ev.params["args"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|v| {
                                    v["value"]
                                        .as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| v["description"].to_string())
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    out.push(EngineEvent::Console {
                        webview: webview.clone(),
                        level: ev.params["type"].as_str().unwrap_or("log").to_string(),
                        text,
                    });
                }
                _ => {}
            }
        }
        out
    }
}

fn looks_like_human_challenge(obs: &Observation) -> bool {
    let t = format!("{} {}", obs.page.title, obs.page.url).to_lowercase();
    let body = obs.content.iter().take(3).map(|c| c.text.to_lowercase()).collect::<Vec<_>>().join(" ");
    let markers =
        ["captcha", "verify you are human", "are you a robot", "checking your browser", "cf-chl", "just a moment"];
    markers.iter().any(|m| t.contains(m)) || markers.iter().any(|m| body.contains(m))
}

#[async_trait]
impl EngineAdapter for CdpEngine {
    fn backend_name(&self) -> &'static str {
        "cdp"
    }

    async fn create_webview(&self, opts: WebViewOptions) -> Result<Id> {
        let context_id = self.browser_context_for(&opts.profile).await?;
        // A fresh browser context has no window yet; Chromium refuses to open a
        // tab there unless a new window is requested.
        let mut params = json!({ "url": "about:blank", "newWindow": context_id.is_some(), "background": false });
        if let Some(ctx) = &context_id {
            params["browserContextId"] = json!(ctx);
        }
        let r = self.conn.call(None, "Target.createTarget", params).await?;
        let target_id = r["targetId"].as_str().unwrap_or_default().to_string();
        let r =
            self.conn.call(None, "Target.attachToTarget", json!({ "targetId": target_id, "flatten": true })).await?;
        let session_id = r["sessionId"].as_str().unwrap_or_default().to_string();

        for (method, params) in [
            ("Page.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("DOM.enable", json!({})),
            ("Network.enable", json!({})),
            ("Page.setLifecycleEventsEnabled", json!({ "enabled": true })),
            ("Emulation.setFocusEmulationEnabled", json!({ "enabled": true })),
        ] {
            self.call(&session_id, method, params).await?;
        }
        if opts.inject_sensor {
            self.call(
                &session_id,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": SENSOR_JS, "worldName": SENSOR_WORLD, "runImmediately": true }),
            )
            .await?;
        }
        let id = format!("wv-{}", &target_id[..8.min(target_id.len())]);
        self.webviews.lock().unwrap().insert(
            id.clone(),
            WebViewState {
                target_id,
                session_id,
                context_id,
                profile: opts.profile,
                url: "about:blank".into(),
                title: String::new(),
                inflight: HashMap::new(),
            },
        );
        Ok(id)
    }

    async fn close_webview(&self, webview: &Id) -> Result<()> {
        let state =
            self.webviews.lock().unwrap().remove(webview).ok_or_else(|| EngineError::NoSuchWebView(webview.clone()))?;
        let _ = self.conn.call(None, "Target.closeTarget", json!({ "targetId": state.target_id })).await;
        self.extra_events.lock().unwrap().push(EngineEvent::Closed { webview: webview.clone() });
        Ok(())
    }

    async fn navigate(&self, webview: &Id, url: &str) -> Result<()> {
        let session = self.session_of(webview)?;
        let since = self.conn.cursor();
        let r = self.call(&session, "Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = r.get("errorText").and_then(Value::as_str) {
            return Err(EngineError::Navigation(format!("{url}: {err}")));
        }
        let loader = r["loaderId"].as_str().map(str::to_string);
        self.wait_loaded(&session, since, loader.as_deref(), Duration::from_secs(20)).await;
        self.wait_idle(&session, since, Duration::from_millis(800)).await;
        if let Some(w) = self.webviews.lock().unwrap().get_mut(webview) {
            w.url = url.to_string();
        }
        Ok(())
    }

    async fn observe(&self, webview: &Id) -> Result<Observation> {
        let session = self.session_of(webview)?;
        let r = self.eval_sensor(&session, "JSON.stringify(BrowseSensor.snapshot())", true).await?;
        let json_text =
            r["value"].as_str().ok_or_else(|| EngineError::Backend("sensor returned no snapshot".into()))?;
        let mut obs: Observation =
            serde_json::from_str(json_text).map_err(|e| EngineError::Backend(format!("bad observation: {e}")))?;
        obs.page.origin = Origin::parse(&obs.page.url).map_err(|e| EngineError::Backend(e.to_string()))?;
        obs.page.untrusted = true;

        // Session detection via the cookie jar, not page heuristics alone.
        let has_cookies = {
            let ctx = self.webviews.lock().unwrap().get(webview).and_then(|w| w.context_id.clone());
            let mut p = json!({});
            if let Some(c) = &ctx {
                p["browserContextId"] = json!(c);
            }
            match self.conn.call(None, "Storage.getCookies", p).await {
                Ok(v) => v["cookies"]
                    .as_array()
                    .map(|cs| cs.iter().any(|c| cookie_matches(c, obs.page.origin.host()) && c["httpOnly"] == true))
                    .unwrap_or(false),
                Err(_) => false,
            }
        };
        obs.page.has_session = obs.page.has_session || has_cookies;
        obs.page.sensitivity = (self.classifier)(&obs.page.origin, obs.page.has_session);
        if let Some(w) = self.webviews.lock().unwrap().get_mut(webview) {
            w.url = obs.page.url.clone();
            w.title = obs.page.title.clone();
        }
        if looks_like_human_challenge(&obs) {
            self.extra_events.lock().unwrap().push(EngineEvent::HumanChallenge { webview: webview.clone() });
        }
        Ok(obs)
    }

    async fn act(&self, webview: &Id, action: Action) -> Result<ActionResult> {
        let session = self.session_of(webview)?;
        let since = self.conn.cursor();
        let mut output = None;
        match &action {
            Action::Navigate { url } => self.navigate(webview, url).await?,
            Action::Back | Action::Forward => {
                let h = self.call(&session, "Page.getNavigationHistory", json!({})).await?;
                let idx = h["currentIndex"].as_i64().unwrap_or(0);
                let entries = h["entries"].as_array().cloned().unwrap_or_default();
                let target = if matches!(action, Action::Back) { idx - 1 } else { idx + 1 };
                let Some(entry) = entries.get(target.max(0) as usize).filter(|_| target >= 0) else {
                    return Ok(ActionResult {
                        ok: false,
                        message: Some("no history entry".into()),
                        url: self.current_url(webview).unwrap_or_default(),
                        output: None,
                    });
                };
                self.call(&session, "Page.navigateToHistoryEntry", json!({ "entryId": entry["id"] })).await?;
                self.settle(&session, since).await;
            }
            Action::Reload => {
                self.call(&session, "Page.reload", json!({})).await?;
                self.wait_loaded(&session, since, None, Duration::from_secs(20)).await;
            }
            Action::Click { target } => {
                let obj = self.resolve_ref(&session, target.id).await?;
                let (x, y) = self.center_of(&session, &obj).await?;
                self.mouse(&session, "mouseMoved", x, y).await?;
                self.mouse(&session, "mousePressed", x, y).await?;
                self.mouse(&session, "mouseReleased", x, y).await?;
                self.settle(&session, since).await;
            }
            Action::Type { target, text, submit } => {
                let obj = self.resolve_ref(&session, target.id).await?;
                let masked = self
                    .call(
                        &session,
                        "Runtime.callFunctionOn",
                        json!({
                            "objectId": obj,
                            "functionDeclaration": "function(){ return this.type === 'password' || /^(cc-|one-time-code|current-password|new-password)/.test(this.autocomplete || '') }",
                            "returnByValue": true
                        }),
                    )
                    .await?;
                if masked["result"]["value"].as_bool() == Some(true) {
                    return Err(EngineError::Blocked("refusing to type into a masked field".into()));
                }
                self.call(&session, "DOM.focus", json!({ "objectId": obj })).await?;
                self.call(
                    &session,
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": obj,
                        "functionDeclaration": "function(){ if (typeof this.select === 'function') { this.select(); } else if (this.isContentEditable) { const r = document.createRange(); r.selectNodeContents(this); const s = getSelection(); s.removeAllRanges(); s.addRange(r); } }"
                    }),
                )
                .await?;
                if text.is_empty() {
                    self.key(&session, "Backspace").await?;
                } else {
                    self.call(&session, "Input.insertText", json!({ "text": text })).await?;
                }
                if *submit {
                    self.key(&session, "Enter").await?;
                    self.settle(&session, since).await;
                }
            }
            Action::Select { target, value } => {
                let obj = self.resolve_ref(&session, target.id).await?;
                let r = self
                    .call(
                        &session,
                        "Runtime.callFunctionOn",
                        json!({
                            "objectId": obj,
                            "functionDeclaration": "function(v){ const opts = Array.from(this.options || []); const o = opts.find(o => o.label === v || o.value === v || o.textContent.trim() === v) || opts.find(o => o.label.toLowerCase().includes(String(v).toLowerCase())); if (!o) return false; this.value = o.value; this.dispatchEvent(new Event('input', {bubbles:true})); this.dispatchEvent(new Event('change', {bubbles:true})); return true }",
                            "arguments": [{ "value": value }],
                            "returnByValue": true
                        }),
                    )
                    .await?;
                if r["result"]["value"].as_bool() != Some(true) {
                    return Ok(ActionResult {
                        ok: false,
                        message: Some(format!("no option matching `{value}`")),
                        url: self.current_url(webview).unwrap_or_default(),
                        output: None,
                    });
                }
            }
            Action::Check { target, checked } => {
                let obj = self.resolve_ref(&session, target.id).await?;
                let cur = self
                    .call(
                        &session,
                        "Runtime.callFunctionOn",
                        json!({ "objectId": obj, "functionDeclaration": "function(){ return !!this.checked }", "returnByValue": true }),
                    )
                    .await?;
                if cur["result"]["value"].as_bool() != Some(*checked) {
                    let (x, y) = self.center_of(&session, &obj).await?;
                    self.mouse(&session, "mouseMoved", x, y).await?;
                    self.mouse(&session, "mousePressed", x, y).await?;
                    self.mouse(&session, "mouseReleased", x, y).await?;
                }
            }
            Action::Scroll { target, delta_y } => {
                let (x, y) = match target {
                    Some(t) => {
                        let obj = self.resolve_ref(&session, t.id).await?;
                        self.center_of(&session, &obj).await?
                    }
                    None => (400.0, 300.0),
                };
                self.call(
                    &session,
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": delta_y }),
                )
                .await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Action::PressKey { key } => {
                self.key(&session, key).await?;
                self.settle(&session, since).await;
            }
            Action::ReadMore { obs_id } => {
                let r = self.eval_sensor(&session, &format!("BrowseSensor.readMore({})", json!(obs_id)), true).await?;
                output = Some(r["value"].clone());
            }
            Action::CallSiteTool { .. } => {
                return Err(EngineError::Backend("WebMCP site tools are not supported by the CDP backend".into()));
            }
            Action::WaitForIdle { timeout_ms } => {
                self.wait_idle(&session, since, Duration::from_millis(*timeout_ms)).await;
            }
        }
        let url = self
            .evaluate(webview, "location.href")
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| self.current_url(webview).unwrap_or_default());
        if let Some(w) = self.webviews.lock().unwrap().get_mut(webview) {
            w.url = url.clone();
        }
        Ok(ActionResult { ok: true, message: None, url, output })
    }

    async fn import_session(&self, target: &Id, origin: &Origin, cookies: Vec<SessionCookie>) -> Result<usize> {
        let (profile, ctx) = {
            let wvs = self.webviews.lock().unwrap();
            let w = wvs.get(target).ok_or_else(|| EngineError::NoSuchWebView(target.clone()))?;
            (w.profile.clone(), w.context_id.clone())
        };
        if !matches!(profile, ProfileKind::Agent { .. }) {
            return Err(EngineError::Blocked("session import is only allowed into agent profiles".into()));
        }
        let Some(ctx) = ctx else { return Err(EngineError::Blocked("agent webview has no browser context".into())) };
        let host = origin.host();
        let cookies: Vec<Value> = cookies
            .into_iter()
            .filter(|c| host == c.domain.trim_start_matches('.') || host.ends_with(c.domain.trim_start_matches('.')))
            .map(|c| {
                let mut v = json!({
                    "name": c.name, "value": c.value, "domain": c.domain, "path": c.path,
                    "secure": c.secure, "httpOnly": c.http_only,
                });
                if let Some(e) = c.expires {
                    v["expires"] = json!(e);
                }
                v
            })
            .collect();
        let n = cookies.len();
        if n > 0 {
            self.conn.call(None, "Storage.setCookies", json!({ "cookies": cookies, "browserContextId": ctx })).await?;
        }
        Ok(n)
    }

    async fn export_session(&self, user_webview: &Id, origin: &Origin) -> Result<Vec<SessionCookie>> {
        let ctx = {
            let wvs = self.webviews.lock().unwrap();
            let w = wvs.get(user_webview).ok_or_else(|| EngineError::NoSuchWebView(user_webview.clone()))?;
            w.context_id.clone()
        };
        let mut p = json!({});
        if let Some(c) = ctx {
            p["browserContextId"] = json!(c);
        }
        let r = self.conn.call(None, "Storage.getCookies", p).await?;
        let host = origin.host();
        Ok(r["cookies"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|c| cookie_matches(c, host))
            .map(|c| SessionCookie {
                name: c["name"].as_str().unwrap_or("").into(),
                value: c["value"].as_str().unwrap_or("").into(),
                domain: c["domain"].as_str().unwrap_or("").into(),
                path: c["path"].as_str().unwrap_or("/").into(),
                secure: c["secure"].as_bool().unwrap_or(false),
                http_only: c["httpOnly"].as_bool().unwrap_or(false),
                expires: c["expires"].as_f64().filter(|e| *e > 0.0).map(|e| e as i64),
            })
            .collect())
    }

    async fn screenshot(&self, webview: &Id) -> Result<Vec<u8>> {
        let session = self.session_of(webview)?;
        let r = self.call(&session, "Page.captureScreenshot", json!({ "format": "png" })).await?;
        let data = r["data"].as_str().unwrap_or("");
        base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| EngineError::Backend(e.to_string()))
    }

    async fn poll_events(&self) -> Result<Vec<EngineEvent>> {
        let mut out: Vec<EngineEvent> = std::mem::take(&mut *self.extra_events.lock().unwrap());
        let mut wvs = self.webviews.lock().unwrap();
        for (id, state) in wvs.iter_mut() {
            let events = self.conn.drain_events(Some(&state.session_id));
            out.extend(self.map_events(id, state, events));
        }
        // Browser-level events are consumed so the queue does not grow unbounded.
        let _ = self.conn.drain_events(None);
        Ok(out)
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
}

fn cookie_matches(c: &Value, host: &str) -> bool {
    let d = c["domain"].as_str().unwrap_or("");
    let d = d.trim_start_matches('.');
    !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
}
