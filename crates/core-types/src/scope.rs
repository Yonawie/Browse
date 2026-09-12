//! `TaskScope` — declared before an agent's first action (ADR-005 §4).
//! Mirrors `schemas/task-scope.schema.json`.

use crate::{ModelTier, Origin};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScopeProfile {
    /// Separate agent profile with its own cookies/storage (default).
    #[default]
    Isolated,
    /// The user's own profile, read-only tools only.
    UserReadonly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossOriginFlow {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeLimits {
    pub max_steps: u32,
    pub max_duration_ms: u64,
    /// 0 => the agent may never reach a payment step.
    pub money_limit_minor_units: u64,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default = "default_max_consequential")]
    pub max_consequential_actions: u32,
}

fn default_max_consequential() -> u32 {
    3
}

impl Default for ScopeLimits {
    fn default() -> Self {
        Self {
            max_steps: 40,
            max_duration_ms: 600_000,
            money_limit_minor_units: 0,
            currency: None,
            max_consequential_actions: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeModelPolicy {
    #[serde(default)]
    pub allow_cloud_planning: bool,
    #[serde(default = "default_critic_tier")]
    pub min_critic_tier: ModelTier,
}

fn default_critic_tier() -> ModelTier {
    ModelTier::Fast
}

impl Default for ScopeModelPolicy {
    fn default() -> Self {
        Self { allow_cloud_planning: false, min_critic_tier: ModelTier::Fast }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskScope {
    /// Origin patterns (`https://host[:port]` or `https://*.host`).
    pub origins: Vec<String>,
    /// Allow-listed tool names.
    pub tools: Vec<String>,
    #[serde(default)]
    pub profile: ScopeProfile,
    #[serde(default)]
    pub session_imports: Vec<String>,
    #[serde(default)]
    pub cross_origin_flows: Vec<CrossOriginFlow>,
    #[serde(default)]
    pub observe_cross_origin_iframes: Vec<String>,
    #[serde(default)]
    pub limits: ScopeLimits,
    #[serde(default)]
    pub model_policy: ScopeModelPolicy,
}

impl TaskScope {
    pub fn new(
        origins: impl IntoIterator<Item = impl Into<String>>,
        tools: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            origins: origins.into_iter().map(Into::into).collect(),
            tools: tools.into_iter().map(Into::into).collect(),
            profile: ScopeProfile::Isolated,
            session_imports: vec![],
            cross_origin_flows: vec![],
            observe_cross_origin_iframes: vec![],
            limits: ScopeLimits::default(),
            model_policy: ScopeModelPolicy::default(),
        }
    }

    pub fn allows_origin(&self, origin: &Origin) -> bool {
        self.origins.iter().any(|p| origin.matches_pattern(p))
    }

    pub fn allows_tool(&self, tool: &str) -> bool {
        self.tools.iter().any(|t| t == tool)
    }

    pub fn flow_preapproved(&self, from: &Origin, to: &Origin) -> bool {
        self.cross_origin_flows.iter().any(|f| from.matches_pattern(&f.from) && to.matches_pattern(&f.to))
    }
}
