//! Model Gateway (ADR-003, ADR-007).
//!
//! The gateway is the *only* path from the rest of the browser to any model.
//! It owns three responsibilities:
//!
//! 1. A uniform [`ModelProvider`] interface over local runtimes (`llama-server`,
//!    Apple Foundation Models, ONNX embedders) and cloud adapters.
//! 2. [`router::Router`]: deterministic routing by sensitivity class, offline
//!    mode and tier availability. `Private` and `Secret` requests can never be
//!    routed to a cloud provider; `Secret` is refused entirely.
//! 3. [`manager::ModelManager`]: VRAM budget accounting for the local runtime
//!    (fast + embed resident, smart/vision LRU-swapped, compositor reserve).
//!
//! No HTTP client lives in this crate. The `llama-server` adapter speaks the
//! OpenAI-compatible protocol through a [`llama_server::Transport`] trait so
//! that the request/response shaping is unit-testable and the desktop app can
//! plug in whichever client it prefers.

pub mod llama_server;
pub mod manager;
pub mod router;

use core_types::{Locality, ModelTier, Sensitivity};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("no provider can serve this request: {0}")]
    NoRoute(String),
    #[error("request refused by policy: {0}")]
    Refused(String),
    #[error("provider {provider} failed: {message}")]
    Provider { provider: String, message: String },
    #[error("malformed provider response: {0}")]
    Protocol(String),
    #[error("model does not fit into the VRAM budget: need {need_mb} MiB, have {available_mb} MiB")]
    OutOfBudget { need_mb: u32, available_mb: u32 },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// For `Role::Tool` messages: the id of the call this message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_call_id: None }
    }
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: content.into(), tool_call_id: Some(call_id.into()) }
    }
}

/// A tool exposed to the model (function-calling). `parameters` is a JSON Schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A request to the gateway. `sensitivity` is computed by the caller from the
/// provenance of every part (observation, memory hits, user text) — never by the
/// model itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub tier: ModelTier,
    pub sensitivity: Sensitivity,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// If set, the provider must return JSON conforming to this schema
    /// (grammar-constrained on llama.cpp, `response_format` on OpenAI-compatible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Explicit per-request user opt-in to use a cloud provider for `Personal`
    /// data (the toggle in the UI). Ignored for `Private`/`Secret`.
    #[serde(default)]
    pub cloud_opt_in: bool,
}

impl ModelRequest {
    pub fn new(tier: ModelTier, sensitivity: Sensitivity, messages: Vec<Message>) -> Self {
        Self {
            tier,
            sensitivity,
            messages,
            tools: vec![],
            json_schema: None,
            max_tokens: None,
            temperature: None,
            cloud_opt_in: false,
        }
    }

    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.json_schema = Some(schema);
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallOut {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments as produced by the model. Provenance `Model` until the
    /// agent runtime resolves refs.
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallOut>,
    #[serde(default)]
    pub usage: Usage,
    pub provider: String,
    pub model: String,
    pub locality: Locality,
}

impl ModelResponse {
    /// Parse `content` as JSON (for structured-output requests).
    pub fn json(&self) -> Result<serde_json::Value, ModelError> {
        Ok(serde_json::from_str(self.content.trim())?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub locality: Locality,
    pub tiers: Vec<ModelTier>,
}

impl ProviderInfo {
    pub fn supports(&self, tier: ModelTier) -> bool {
        self.tiers.contains(&tier)
    }
}

#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    fn info(&self) -> ProviderInfo;

    async fn chat(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError>;

    /// Embed texts with the `Embed` tier model. Returns one vector per input.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError>;
}
