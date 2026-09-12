//! Engine Adapter (ADR-001): the only layer that knows which rendering engine
//! is underneath. Three production/test backends share this trait:
//!
//! | backend      | platform | transport                                 |
//! |--------------|----------|-------------------------------------------|
//! | `cef`        | desktop  | CEF C API + CDP (`SendDevToolsMessage`)    |
//! | `wkwebview`  | iOS      | Swift implementation via UniFFI callbacks |
//! | `cdp-remote` | any      | WebSocket CDP to an external Chromium     |
//!
//! This crate defines the trait, the action/event vocabulary and a
//! deterministic [`mock::MockEngine`] used by agent-runtime tests. Real
//! backends live in their own crates (`engine-cef`, `engine-cdp`) so the core
//! never links against engine code.

pub mod mock;

use async_trait::async_trait;
use core_types::{ElementRef, Id, Observation, Origin};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("webview not found: {0}")]
    NoSuchWebView(Id),
    #[error("element ref not found: {0:?}")]
    NoSuchElement(ElementRef),
    #[error("navigation failed: {0}")]
    Navigation(String),
    #[error("blocked by engine policy: {0}")]
    Blocked(String),
    #[error("engine backend error: {0}")]
    Backend(String),
    #[error("timeout after {0} ms")]
    Timeout(u64),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Which storage partition a webview belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    /// The user's normal profile.
    User,
    /// Isolated agent profile (own cookies/storage/cache).
    Agent { session_id: Id },
    /// Private window; never indexed.
    Private,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebViewOptions {
    pub profile: ProfileKind,
    /// Inject the page sensor into an isolated world on every document.
    pub inject_sensor: bool,
    /// Origins whose cross-origin iframes the sensor may observe.
    pub observe_iframe_origins: Vec<String>,
    pub headless: bool,
}

impl WebViewOptions {
    pub fn agent(session_id: &str) -> Self {
        Self { profile: ProfileKind::Agent { session_id: session_id.into() }, inject_sensor: true, observe_iframe_origins: vec![], headless: false }
    }
}

/// Low-level actions the runtime can perform. All element targeting goes
/// through [`ElementRef`] (never CSS selectors authored by the model).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    Click { target: ElementRef },
    /// Replaces the field's value. Never used for masked fields.
    Type { target: ElementRef, text: String, submit: bool },
    Select { target: ElementRef, value: String },
    Check { target: ElementRef, checked: bool },
    Scroll { target: Option<ElementRef>, delta_y: i32 },
    PressKey { key: String },
    /// Read more of a content chunk beyond the observation budget.
    ReadMore { obs_id: Id },
    /// Invoke a WebMCP tool exposed by the site.
    CallSiteTool { name: String, args: serde_json::Value },
    WaitForIdle { timeout_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub ok: bool,
    #[serde(default)]
    pub message: Option<String>,
    /// URL after the action (navigation may have happened).
    pub url: String,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
}

/// Events the engine pushes to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EngineEvent {
    NavigationStarted { webview: Id, url: String },
    Loaded { webview: Id, url: String, title: String },
    /// The sensor observed a meaningful mutation (debounced, idle-time only).
    Mutated { webview: Id, snapshot_hash: String },
    /// A request was observed (Chromium backends only). Bodies are never
    /// forwarded; headers are redacted of `Authorization`/`Cookie`.
    Request { webview: Id, method: String, url: String, status: Option<u16> },
    Console { webview: Id, level: String, text: String },
    Closed { webview: Id },
    /// The engine detected a CAPTCHA / anti-bot interstitial; the runtime
    /// must hand control back to the user.
    HumanChallenge { webview: Id },
}

/// Cookie subset needed for one-origin session import (ADR-005 §1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires: Option<i64>,
}

#[async_trait]
pub trait EngineAdapter: Send + Sync {
    fn backend_name(&self) -> &'static str;

    async fn create_webview(&self, opts: WebViewOptions) -> Result<Id>;
    async fn close_webview(&self, webview: &Id) -> Result<()>;

    async fn navigate(&self, webview: &Id, url: &str) -> Result<()>;

    /// Current observation from the sensor (plus CDP enrichment where available).
    async fn observe(&self, webview: &Id) -> Result<Observation>;

    async fn act(&self, webview: &Id, action: Action) -> Result<ActionResult>;

    /// Copy the user's cookies for exactly one origin into an agent profile.
    /// The engine must refuse if `target` is not an agent profile.
    async fn import_session(&self, target: &Id, origin: &Origin, cookies: Vec<SessionCookie>) -> Result<usize>;

    /// Read the user's cookies for one origin (only used to feed
    /// `import_session` after explicit confirmation; never exposed to models).
    async fn export_session(&self, user_webview: &Id, origin: &Origin) -> Result<Vec<SessionCookie>>;

    /// Screenshot as PNG bytes (vision tier only; off by default).
    async fn screenshot(&self, webview: &Id) -> Result<Vec<u8>>;

    /// Drain pending events.
    async fn poll_events(&self) -> Result<Vec<EngineEvent>>;

    /// Backend capabilities the runtime adapts to.
    fn capabilities(&self) -> Capabilities;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub accessibility_tree: bool,
    pub network_events: bool,
    pub trusted_input_events: bool,
    pub isolated_profiles: bool,
    pub webextensions: bool,
    pub screenshots: bool,
}

impl Capabilities {
    pub const CHROMIUM: Capabilities = Capabilities { accessibility_tree: true, network_events: true, trusted_input_events: true, isolated_profiles: true, webextensions: true, screenshots: true };
    pub const WKWEBVIEW: Capabilities = Capabilities { accessibility_tree: false, network_events: false, trusted_input_events: false, isolated_profiles: true, webextensions: false, screenshots: true };
}
