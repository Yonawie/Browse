//! The `Observation` format (ADR-006): what a model sees of a page.
//!
//! Produced by the TypeScript sensor (`sensor/`) in an isolated world and, on
//! Chromium backends, enriched from the CDP accessibility tree. Never contains
//! raw HTML, script/style content, hidden nodes (except as injection signals),
//! masked field values or cross-origin iframes without a grant.

use crate::{Id, Origin, Sensitivity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Article,
    Product,
    Form,
    App,
    Search,
    Doc,
    Code,
    Media,
    Auth,
    Checkout,
    #[default]
    Unknown,
}

/// Stable reference to an element inside a document.
///
/// `id` is backend-specific (CDP `backendNodeId` or a sensor WeakMap id) and
/// only valid for the current document lifetime; `path` is a semantic path
/// (`role/name/index` segments) used to re-resolve after re-render and in
/// recorded scenarios.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementRef {
    pub id: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ElementState {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expanded: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Password / card-number style field. Value is never captured.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// An interactive or otherwise action-relevant element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractiveElement {
    #[serde(rename = "ref")]
    pub element_ref: ElementRef,
    /// ARIA role or inferred role (`button`, `link`, `textbox`, `combobox`, ...).
    pub role: String,
    /// Accessible name (label, aria-label, text content).
    pub name: String,
    #[serde(default)]
    pub state: ElementState,
    /// Current value for inputs; `None` when masked or empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<BoundingBox>,
    pub in_viewport: bool,
    /// Landmark the element belongs to (`main`, `navigation`, `form:checkout`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmark: Option<String>,
    /// Set when the element is a form submit inside a form with payment fields
    /// or otherwise pattern-matches a consequential action. Deterministic,
    /// computed by the sensor; PolicyEngine re-checks with its own lexicon.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub consequential_hint: bool,
}

/// A chunk of the page's main content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentChunk {
    pub obs_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    pub text: String,
    pub char_start: u32,
    pub char_end: u32,
    /// Heuristic signal that this text tries to instruct a model. Never hides
    /// the text; feeds the critic.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspect_injection: bool,
}

/// A WebMCP tool exposed by the site (`navigator.modelContext` or annotated form).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub read_only_hint: bool,
    #[serde(default)]
    pub consequential_hint: bool,
    #[serde(default = "default_true")]
    pub untrusted_content_hint: bool,
    /// `declarative` (form) or `imperative` (JS registerTool).
    pub source: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub url: String,
    pub origin: Origin,
    pub title: String,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub page_kind: PageKind,
    /// Always `true`; present so it is impossible to forget in prompts.
    pub untrusted: bool,
    pub frame_id: String,
    /// Hash of the serialized observation, used for diffs and the journal.
    pub snapshot_hash: String,
    pub sensitivity: Sensitivity,
    #[serde(default)]
    pub has_session: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub page: PageMeta,
    #[serde(default)]
    pub interactive: Vec<InteractiveElement>,
    #[serde(default)]
    pub content: Vec<ContentChunk>,
    #[serde(default)]
    pub tools: Vec<SiteTool>,
    /// Hidden or off-screen text that looked like model instructions. Kept out
    /// of `content` so the planner does not read it as data; passed to the
    /// injection classifier and the critic.
    #[serde(default)]
    pub hidden_text_signals: Vec<String>,
    /// Rough token estimate of the serialized observation.
    #[serde(default)]
    pub approx_tokens: u32,
    pub captured_at: crate::UnixMs,
}

impl Observation {
    pub fn find_ref(&self, id: u64) -> Option<&InteractiveElement> {
        self.interactive.iter().find(|e| e.element_ref.id == id)
    }

    /// Any element or chunk flagged as suspicious.
    pub fn has_injection_signals(&self) -> bool {
        !self.hidden_text_signals.is_empty() || self.content.iter().any(|c| c.suspect_injection)
    }
}
