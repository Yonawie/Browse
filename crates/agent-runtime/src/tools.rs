//! Built-in tool registry and the ToolCall ⇄ engine Action bridge.
//!
//! Two principles from ADR-005 §3 are enforced here rather than in prompts:
//!
//! * **Tools take references, not free text.** Element targets are `ref` ids
//!   from the current observation; text arguments are *references* to a source
//!   (`$user`, `$obs:<id>`, `$mem:<id>`) that the runtime resolves and labels
//!   with the source's provenance. A literal string from the model is labelled
//!   `Provenance::Model` and PolicyEngine denies it unless the manifest says
//!   `free_text_ok`.
//! * **Deterministic target snapshot.** The `TargetInfo` attached to the call
//!   (role, name, submit-ness, payment fields) comes from the observation, so
//!   consequential classification cannot be talked out of by the model.

use std::collections::BTreeMap;

use core_types::{
    ArgAnnotation, ArgKind, ConsequentialPredicate, ElementRef, InteractiveElement, LabeledValue, Observation,
    Provenance, ProvenanceClass, Sensitivity, TargetInfo, ToolCall, ToolManifest,
};
use engine_adapter::Action;
use model_gateway::ToolSpec;
use serde_json::{json, Value};

use crate::AgentError;

fn ann(kind: ArgKind) -> ArgAnnotation {
    ArgAnnotation { kind, ..Default::default() }
}

/// Non-text arguments the model is allowed to author directly.
fn model_ok(kind: ArgKind) -> ArgAnnotation {
    ArgAnnotation {
        kind,
        free_text_ok: false,
        allowed_provenance: vec![
            ProvenanceClass::User,
            ProvenanceClass::Memory,
            ProvenanceClass::OriginSame,
            ProvenanceClass::Model,
        ],
        secret_forbidden: true,
    }
}

fn free_text_ok(kind: ArgKind) -> ArgAnnotation {
    ArgAnnotation { kind, free_text_ok: true, ..model_ok(kind) }
}

/// Built-in tool manifests. `extract` and `read_more`, `search_memory` are
/// read-only and therefore usable in the `user_readonly` scope profile.
pub fn builtin_manifests() -> Vec<ToolManifest> {
    vec![
        ToolManifest::builtin(
            "navigate",
            "Open a URL in the agent tab. Only https origins inside the task scope.",
            false,
        )
        .with_arg("url", model_ok(ArgKind::Url))
        .with_consequential_when(vec![ConsequentialPredicate::NavigatesToCheckoutLikeUrl]),
        ToolManifest::builtin("click", "Click an interactive element by its `ref` from the observation.", false)
            .with_arg("ref", model_ok(ArgKind::Ref))
            .with_consequential_when(vec![
                ConsequentialPredicate::TargetIsSubmit,
                ConsequentialPredicate::TargetTextMatchesConsequentialLexicon,
                ConsequentialPredicate::FormHasPaymentFields,
            ]),
        ToolManifest::builtin(
            "type",
            "Replace the value of a text field. `text` must reference a source: `$user`, `$obs:<id>` or `$mem:<id>`.",
            false,
        )
        .with_arg("ref", model_ok(ArgKind::Ref))
        .with_arg("text", ann(ArgKind::FreeText))
        .with_arg("submit", model_ok(ArgKind::Enum))
        .with_consequential_when(vec![
            ConsequentialPredicate::TargetIsSubmit,
            ConsequentialPredicate::FormHasPaymentFields,
        ]),
        ToolManifest::builtin("select", "Choose an option in a <select> by visible label.", false)
            .with_arg("ref", model_ok(ArgKind::Ref))
            .with_arg("value", free_text_ok(ArgKind::Enum)),
        ToolManifest::builtin("scroll", "Scroll the page or an element.", true)
            .with_arg("ref", model_ok(ArgKind::Ref))
            .with_arg("delta_y", model_ok(ArgKind::Number)),
        ToolManifest::builtin(
            "read_more",
            "Read the full text of a content chunk beyond the observation budget.",
            true,
        )
        .with_arg("obs_id", model_ok(ArgKind::ObsId)),
        ToolManifest::builtin("extract", "Return the text of the given content chunks as the task result.", true)
            .with_arg("obs_ids", model_ok(ArgKind::ObsId)),
        ToolManifest::builtin("search_memory", "Hybrid search over the user's browsing memory (local only).", true)
            .with_arg("query", free_text_ok(ArgKind::FreeText)),
        ToolManifest::builtin("call_site_tool", "Invoke a WebMCP tool declared by the current page.", false)
            .with_arg("name", model_ok(ArgKind::Enum))
            .with_arg("args", ann(ArgKind::FreeText))
            .with_consequential_when(vec![ConsequentialPredicate::DeletesOrPublishes]),
    ]
}

pub struct ToolRegistry {
    manifests: BTreeMap<String, ToolManifest>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self { manifests: BTreeMap::new() }
    }

    pub fn builtin() -> Self {
        let mut r = Self::empty();
        for m in builtin_manifests() {
            r.register(m);
        }
        r
    }

    pub fn register(&mut self, manifest: ToolManifest) {
        self.manifests.insert(manifest.name.clone(), manifest);
    }

    pub fn get(&self, name: &str) -> Option<&ToolManifest> {
        self.manifests.get(name)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &ToolManifest> {
        self.manifests.values()
    }

    /// Function-calling specs for the tools allowed by the scope.
    pub fn specs_for(&self, allowed: &[String]) -> Vec<ToolSpec> {
        self.manifests
            .values()
            .filter(|m| allowed.iter().any(|a| a == &m.name))
            .map(|m| ToolSpec { name: m.name.clone(), description: m.description.clone(), parameters: schema_for(m) })
            .collect()
    }
}

fn schema_for(m: &ToolManifest) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = vec![];
    for (name, a) in &m.args {
        let prop = match a.kind {
            ArgKind::Ref => json!({ "type": "integer", "description": "`ref` of an element in the observation" }),
            ArgKind::ObsId => json!({ "type": ["string", "array"], "description": "obs_id(s) of content chunks" }),
            ArgKind::MemId => json!({ "type": "string" }),
            ArgKind::Url => json!({ "type": "string", "format": "uri" }),
            ArgKind::Number => json!({ "type": "number" }),
            ArgKind::Enum => json!({ "type": ["string", "boolean"] }),
            ArgKind::FreeText if a.free_text_ok => json!({ "type": "string" }),
            ArgKind::FreeText => json!({
                "type": "string",
                "description": "A source reference: `$user` (the user's request), `$obs:<obs_id>` (page text), `$mem:<id>` (a memory). Literal text is rejected."
            }),
        };
        props.insert(name.clone(), prop);
        if name == "ref" || name == "url" || name == "text" || name == "obs_id" || name == "query" || name == "name" {
            required.push(Value::String(name.clone()));
        }
    }
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

/// Sources a `$…` reference can resolve to.
pub struct ResolveContext<'a> {
    pub observation: &'a Observation,
    pub user_request: &'a str,
    pub user_request_sensitivity: Sensitivity,
    /// Memory lookups (`$mem:<id>` → text). Provided by the runtime.
    pub memory_lookup: &'a dyn Fn(&str) -> Option<(String, Sensitivity)>,
    /// Outputs of earlier tool calls in this session (`$result:<call_id>`),
    /// still carrying the provenance of the page they were read from.
    pub results: &'a BTreeMap<String, LabeledValue>,
}

/// Turn raw model arguments into a labelled [`ToolCall`].
pub fn label_call(
    call_id: &str,
    tool: &str,
    raw_args: &Value,
    manifest: &ToolManifest,
    ctx: &ResolveContext<'_>,
) -> Result<ToolCall, AgentError> {
    let obj = raw_args.as_object().ok_or_else(|| AgentError::BadToolArgs("arguments must be an object".into()))?;
    let origin = ctx.observation.page.origin.clone();
    let mut call = ToolCall::new(call_id, tool).on(origin);

    for (name, raw) in obj {
        let a = manifest.args.get(name).cloned().unwrap_or_default();
        let labeled = match (&a.kind, raw) {
            (ArgKind::FreeText, Value::String(s)) if s.starts_with('$') => resolve_reference(s, ctx)?,
            _ => LabeledValue::model(raw.clone()),
        };
        call = call.arg(name, labeled);
    }

    // Attach a deterministic target snapshot for element-targeting tools.
    if let Some(Value::Number(n)) = obj.get("ref") {
        let id = n.as_u64().ok_or_else(|| AgentError::BadToolArgs("`ref` must be a positive integer".into()))?;
        let el = ctx.observation.find_ref(id).ok_or(AgentError::UnknownRef(id))?;
        if el.state.masked {
            return Err(AgentError::MaskedField(id));
        }
        call = call.target(target_info(el, ctx.observation));
    }
    if let Some(Value::String(name)) = obj.get("name") {
        if tool == "call_site_tool" {
            if let Some(t) = ctx.observation.tools.iter().find(|t| &t.name == name) {
                call = call.target(TargetInfo {
                    role: "site_tool".into(),
                    name: t.name.clone(),
                    site_tool_consequential_hint: t.consequential_hint,
                    ..Default::default()
                });
            }
        }
    }
    Ok(call)
}

fn resolve_reference(reference: &str, ctx: &ResolveContext<'_>) -> Result<LabeledValue, AgentError> {
    if reference == "$user" {
        return Ok(LabeledValue {
            value: json!(ctx.user_request),
            provenance: Provenance::User,
            sensitivity: ctx.user_request_sensitivity,
        });
    }
    if let Some(obs_id) = reference.strip_prefix("$obs:") {
        let chunk = ctx
            .observation
            .content
            .iter()
            .find(|c| c.obs_id == obs_id)
            .ok_or_else(|| AgentError::BadToolArgs(format!("unknown obs_id `{obs_id}`")))?;
        return Ok(LabeledValue::from_origin(
            chunk.text.clone(),
            ctx.observation.page.origin.clone(),
            ctx.observation.page.sensitivity,
        ));
    }
    if let Some(id) = reference.strip_prefix("$mem:") {
        let (text, sens) =
            (ctx.memory_lookup)(id).ok_or_else(|| AgentError::BadToolArgs(format!("unknown memory `{id}`")))?;
        return Ok(LabeledValue {
            value: json!(text),
            provenance: Provenance::Memory { id: id.to_string() },
            sensitivity: sens,
        });
    }
    if let Some(call_id) = reference.strip_prefix("$result:") {
        let prior =
            ctx.results.get(call_id).ok_or_else(|| AgentError::BadToolArgs(format!("unknown result `{call_id}`")))?;
        return Ok(LabeledValue {
            value: json!(flatten_text(&prior.value)),
            provenance: prior.provenance.clone(),
            sensitivity: prior.sensitivity,
        });
    }
    Err(AgentError::BadToolArgs(format!("unknown reference `{reference}`")))
}

/// Text view of a tool output (`extract` returns `[{obs_id, text}]`).
fn flatten_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(flatten_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => o.get("text").map(flatten_text).unwrap_or_default(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn target_info(el: &InteractiveElement, obs: &Observation) -> TargetInfo {
    let input_type = el.input_type.clone();
    let is_submit = el.role == "button"
        && (input_type.as_deref() == Some("submit") || el.name.eq_ignore_ascii_case("submit") || el.consequential_hint);
    let form_has_payment_fields = obs.interactive.iter().any(|e| {
        let n = e.name.to_ascii_lowercase();
        e.input_type.as_deref() == Some("cc-number")
            || n.contains("card number")
            || n.contains("cvv")
            || n.contains("cvc")
            || n.contains("номер карты")
    });
    let form_has_file_upload = obs.interactive.iter().any(|e| e.input_type.as_deref() == Some("file"));
    TargetInfo {
        role: el.role.clone(),
        name: el.name.clone(),
        input_type,
        is_submit,
        form_has_payment_fields,
        form_has_file_upload,
        href: el.href.clone(),
        site_tool_consequential_hint: el.consequential_hint,
    }
}

fn element_ref(call: &ToolCall, obs: &Observation) -> Result<ElementRef, AgentError> {
    let id = call
        .args
        .get("ref")
        .and_then(|v| v.value.as_u64())
        .ok_or_else(|| AgentError::BadToolArgs("missing `ref`".into()))?;
    obs.find_ref(id).map(|e| e.element_ref.clone()).ok_or(AgentError::UnknownRef(id))
}

fn string_arg(call: &ToolCall, name: &str) -> Result<String, AgentError> {
    call.args
        .get(name)
        .and_then(|v| v.value.as_str().map(str::to_string))
        .ok_or_else(|| AgentError::BadToolArgs(format!("missing `{name}`")))
}

/// Which engine action a labelled call maps to. Tools without an engine action
/// (`extract`, `search_memory`) return `None` and are handled by the runtime.
pub fn to_action(call: &ToolCall, obs: &Observation) -> Result<Option<Action>, AgentError> {
    let action = match call.tool.as_str() {
        "navigate" => Action::Navigate { url: string_arg(call, "url")? },
        "click" => Action::Click { target: element_ref(call, obs)? },
        "type" => Action::Type {
            target: element_ref(call, obs)?,
            text: string_arg(call, "text")?,
            submit: call.args.get("submit").and_then(|v| v.value.as_bool()).unwrap_or(false),
        },
        "select" => Action::Select { target: element_ref(call, obs)?, value: string_arg(call, "value")? },
        "scroll" => Action::Scroll {
            target: call.args.get("ref").and_then(|_| element_ref(call, obs).ok()),
            delta_y: call.args.get("delta_y").and_then(|v| v.value.as_i64()).unwrap_or(600) as i32,
        },
        "read_more" => Action::ReadMore { obs_id: string_arg(call, "obs_id")? },
        "call_site_tool" => Action::CallSiteTool {
            name: string_arg(call, "name")?,
            args: call.args.get("args").map(|v| v.value.clone()).unwrap_or(Value::Null),
        },
        "extract" | "search_memory" => return Ok(None),
        other => return Err(AgentError::UnknownTool(other.to_string())),
    };
    Ok(Some(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{ContentChunk, ElementState, PageKind, PageMeta};

    fn obs() -> Observation {
        Observation {
            page: PageMeta {
                url: "https://shop.example/cart".into(),
                origin: core_types::Origin::parse("https://shop.example").unwrap(),
                title: "Cart".into(),
                lang: None,
                page_kind: PageKind::Unknown,
                untrusted: true,
                frame_id: "main".into(),
                snapshot_hash: "h".into(),
                sensitivity: Sensitivity::Public,
                has_session: false,
            },
            interactive: vec![
                InteractiveElement {
                    element_ref: ElementRef { id: 7, path: "textbox/Promo/0".into() },
                    role: "textbox".into(),
                    name: "Promo code".into(),
                    state: ElementState::default(),
                    value: None,
                    href: None,
                    input_type: Some("text".into()),
                    bbox: None,
                    in_viewport: true,
                    landmark: None,
                    consequential_hint: false,
                },
                InteractiveElement {
                    element_ref: ElementRef { id: 8, path: "textbox/Password/0".into() },
                    role: "textbox".into(),
                    name: "Password".into(),
                    state: ElementState { masked: true, ..Default::default() },
                    value: None,
                    href: None,
                    input_type: Some("password".into()),
                    bbox: None,
                    in_viewport: true,
                    landmark: None,
                    consequential_hint: false,
                },
            ],
            content: vec![ContentChunk {
                obs_id: "c1".into(),
                heading_path: None,
                text: "SAVE10".into(),
                char_start: 0,
                char_end: 6,
                suspect_injection: false,
            }],
            tools: vec![],
            hidden_text_signals: vec![],
            approx_tokens: 10,
            selected_text: None,
            captured_at: 0,
        }
    }

    static NO_RESULTS: std::sync::OnceLock<BTreeMap<String, LabeledValue>> = std::sync::OnceLock::new();

    fn ctx<'a>(o: &'a Observation, lookup: &'a dyn Fn(&str) -> Option<(String, Sensitivity)>) -> ResolveContext<'a> {
        ResolveContext {
            observation: o,
            user_request: "apply promo SAVE10",
            user_request_sensitivity: Sensitivity::Personal,
            memory_lookup: lookup,
            results: NO_RESULTS.get_or_init(BTreeMap::new),
        }
    }

    #[test]
    fn text_references_resolve_with_correct_provenance() {
        let o = obs();
        let lookup = |id: &str| (id == "m1").then(|| ("Bert".to_string(), Sensitivity::Personal));
        let c = ctx(&o, &lookup);
        let reg = ToolRegistry::builtin();
        let m = reg.get("type").unwrap();

        let call = label_call("1", "type", &json!({ "ref": 7, "text": "$obs:c1" }), m, &c).unwrap();
        assert!(matches!(call.args["text"].provenance, Provenance::Origin { .. }));
        assert_eq!(call.args["text"].value, "SAVE10");
        assert_eq!(call.target.as_ref().unwrap().name, "Promo code");

        let call = label_call("2", "type", &json!({ "ref": 7, "text": "$user" }), m, &c).unwrap();
        assert_eq!(call.args["text"].provenance, Provenance::User);

        let call = label_call("3", "type", &json!({ "ref": 7, "text": "$mem:m1" }), m, &c).unwrap();
        assert!(matches!(call.args["text"].provenance, Provenance::Memory { .. }));

        let call = label_call("4", "type", &json!({ "ref": 7, "text": "literal from model" }), m, &c).unwrap();
        assert_eq!(call.args["text"].provenance, Provenance::Model);

        assert!(matches!(
            label_call("5", "type", &json!({ "ref": 7, "text": "$obs:nope" }), m, &c),
            Err(AgentError::BadToolArgs(_))
        ));
    }

    #[test]
    fn masked_and_unknown_refs_are_rejected_before_policy() {
        let o = obs();
        let lookup = |_: &str| None;
        let c = ctx(&o, &lookup);
        let reg = ToolRegistry::builtin();
        let m = reg.get("type").unwrap();
        assert!(matches!(
            label_call("1", "type", &json!({ "ref": 8, "text": "$user" }), m, &c),
            Err(AgentError::MaskedField(8))
        ));
        assert!(matches!(
            label_call("2", "click", &json!({ "ref": 99 }), reg.get("click").unwrap(), &c),
            Err(AgentError::UnknownRef(99))
        ));
    }

    #[test]
    fn calls_map_to_engine_actions() {
        let o = obs();
        let lookup = |_: &str| None;
        let c = ctx(&o, &lookup);
        let reg = ToolRegistry::builtin();
        let call = label_call(
            "1",
            "type",
            &json!({ "ref": 7, "text": "$user", "submit": true }),
            reg.get("type").unwrap(),
            &c,
        )
        .unwrap();
        match to_action(&call, &o).unwrap().unwrap() {
            Action::Type { target, text, submit } => {
                assert_eq!(target.id, 7);
                assert_eq!(text, "apply promo SAVE10");
                assert!(submit);
            }
            other => panic!("unexpected {other:?}"),
        }
        let call = label_call("2", "extract", &json!({ "obs_ids": ["c1"] }), reg.get("extract").unwrap(), &c).unwrap();
        assert!(to_action(&call, &o).unwrap().is_none());
    }

    #[test]
    fn specs_only_include_scoped_tools_and_free_text_is_documented_as_reference() {
        let reg = ToolRegistry::builtin();
        let specs = reg.specs_for(&["click".into(), "type".into()]);
        assert_eq!(specs.len(), 2);
        let ty = specs.iter().find(|s| s.name == "type").unwrap();
        let desc = ty.parameters["properties"]["text"]["description"].as_str().unwrap();
        assert!(desc.contains("$obs:"));
    }
}
