//! Tool manifests and tool calls. Mirrors `schemas/tool-manifest.schema.json`.

use crate::{Id, LabeledValue, Origin, Sensitivity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuntime {
    Builtin,
    Wasm,
    McpStdio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgKind {
    Ref,
    ObsId,
    MemId,
    Url,
    FreeText,
    Enum,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceClass {
    User,
    Memory,
    Model,
    OriginSame,
    OriginOther,
}

/// Per-argument annotation (`x-browse` in the JSON schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgAnnotation {
    pub kind: ArgKind,
    #[serde(default)]
    pub free_text_ok: bool,
    #[serde(default = "default_allowed_provenance")]
    pub allowed_provenance: Vec<ProvenanceClass>,
    #[serde(default = "default_true")]
    pub secret_forbidden: bool,
}

fn default_true() -> bool {
    true
}

fn default_allowed_provenance() -> Vec<ProvenanceClass> {
    vec![ProvenanceClass::User, ProvenanceClass::Memory, ProvenanceClass::OriginSame]
}

impl Default for ArgAnnotation {
    fn default() -> Self {
        Self {
            kind: ArgKind::FreeText,
            free_text_ok: false,
            allowed_provenance: default_allowed_provenance(),
            secret_forbidden: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsequentialPredicate {
    TargetIsSubmit,
    TargetTextMatchesConsequentialLexicon,
    FormHasPaymentFields,
    FormHasFileUpload,
    NavigatesToCheckoutLikeUrl,
    DeletesOrPublishes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolSafety {
    pub read_only: bool,
    pub consequential: bool,
    #[serde(default)]
    pub consequential_when: Vec<ConsequentialPredicate>,
    #[serde(default)]
    pub requires_dev_mode: bool,
    #[serde(default)]
    pub max_calls_per_session: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub runtime: ToolRuntime,
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Argument annotations by argument name (extracted from `input_schema`).
    #[serde(default)]
    pub args: BTreeMap<String, ArgAnnotation>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub safety: ToolSafety,
    #[serde(default)]
    pub output_sensitivity: Option<Sensitivity>,
}

impl ToolManifest {
    /// Convenience constructor for built-in tools.
    pub fn builtin(name: &str, description: &str, read_only: bool) -> Self {
        Self {
            name: name.to_string(),
            version: "0.1.0".into(),
            description: description.to_string(),
            runtime: ToolRuntime::Builtin,
            entrypoint: None,
            args: BTreeMap::new(),
            capabilities: vec![],
            safety: ToolSafety { read_only, consequential: false, ..Default::default() },
            output_sensitivity: None,
        }
    }

    pub fn with_arg(mut self, name: &str, ann: ArgAnnotation) -> Self {
        self.args.insert(name.to_string(), ann);
        self
    }

    pub fn with_consequential_when(mut self, preds: Vec<ConsequentialPredicate>) -> Self {
        self.safety.consequential_when = preds;
        self
    }
}

/// A tool invocation proposed by the planner. Arguments carry labels so
/// PolicyEngine can reason about data flow without trusting the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Id,
    pub tool: String,
    pub args: BTreeMap<String, LabeledValue>,
    /// Origin of the page the action targets (current tab), if any.
    #[serde(default)]
    pub target_origin: Option<Origin>,
    /// Snapshot of the target element (name/role/type) for deterministic
    /// consequential classification and confirmation previews.
    #[serde(default)]
    pub target: Option<TargetInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TargetInfo {
    pub role: String,
    pub name: String,
    #[serde(default)]
    pub input_type: Option<String>,
    #[serde(default)]
    pub is_submit: bool,
    #[serde(default)]
    pub form_has_payment_fields: bool,
    #[serde(default)]
    pub form_has_file_upload: bool,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub site_tool_consequential_hint: bool,
}

impl ToolCall {
    pub fn new(id: impl Into<Id>, tool: &str) -> Self {
        Self { id: id.into(), tool: tool.to_string(), args: BTreeMap::new(), target_origin: None, target: None }
    }

    pub fn arg(mut self, name: &str, value: LabeledValue) -> Self {
        self.args.insert(name.to_string(), value);
        self
    }

    pub fn on(mut self, origin: Origin) -> Self {
        self.target_origin = Some(origin);
        self
    }

    pub fn target(mut self, info: TargetInfo) -> Self {
        self.target = Some(info);
        self
    }

    pub fn max_sensitivity(&self) -> Sensitivity {
        Sensitivity::max_of(self.args.values().map(|v| v.sensitivity))
    }
}

/// Result of executing a tool. Output re-enters the model as untrusted data
/// labelled with the tool's origin/name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: Id,
    pub ok: bool,
    pub output: LabeledValue,
    #[serde(default)]
    pub error: Option<String>,
}
