//! Policy & Security layer (ADR-005, ADR-007).
//!
//! Everything in this crate is deterministic: no model is consulted. The
//! agent runtime calls [`PolicyEngine::check`] before the critic and before
//! any confirmation UI. A `Deny` here cannot be overridden by the model or the
//! critic; a `Confirm` can only be resolved by the user.

pub mod config;
pub mod consequential;
pub mod grants;
pub mod sensitivity;

pub use config::PolicyConfig;
pub use grants::{Grant, GrantKind, GrantStore};

use core_types::{
    ArgKind, Origin, Provenance, ProvenanceClass, ScopeProfile, Sensitivity, TaskScope, ToolCall, ToolManifest, UnixMs,
    Verdict,
};
use serde::{Deserialize, Serialize};

/// A policy decision with the rule that produced it, for the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub verdict: Verdict,
    /// Machine-readable rule id, e.g. `scope.tool_not_allowed`.
    pub rule: String,
    #[serde(default)]
    pub consequential: bool,
    #[serde(default)]
    pub details: Vec<String>,
}

impl Decision {
    fn allow(rule: &str) -> Self {
        Self { verdict: Verdict::Allow, rule: rule.into(), consequential: false, details: vec![] }
    }
    fn confirm(rule: &str, reason: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Confirm { reason: reason.into() },
            rule: rule.into(),
            consequential: false,
            details: vec![],
        }
    }
    fn deny(rule: &str, reason: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Deny { reason: reason.into() },
            rule: rule.into(),
            consequential: false,
            details: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

/// Runtime context PolicyEngine needs besides the call itself.
pub struct CheckContext<'a> {
    pub scope: &'a TaskScope,
    pub manifest: &'a ToolManifest,
    pub grants: &'a GrantStore,
    pub now: UnixMs,
    /// Consequential actions already executed in this session.
    pub consequential_so_far: u32,
    /// Steps already executed in this session.
    pub steps_so_far: u32,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    /// Validate a scope before the session starts. Rejects scopes that would
    /// be unsafe by construction.
    pub fn validate_scope(&self, scope: &TaskScope, manifests: &[&ToolManifest]) -> Result<(), String> {
        if scope.origins.is_empty() {
            return Err("scope must declare at least one origin".into());
        }
        for o in &scope.origins {
            if !o.starts_with("https://") {
                return Err(format!("scope origin must be https: {o}"));
            }
        }
        for tool in &scope.tools {
            let m =
                manifests.iter().find(|m| m.name == *tool).ok_or_else(|| format!("unknown tool in scope: {tool}"))?;
            if scope.profile == ScopeProfile::UserReadonly && !m.safety.read_only {
                return Err(format!("tool `{tool}` is not read-only but scope profile is user_readonly"));
            }
            if m.safety.requires_dev_mode && !self.config.dev_mode_enabled {
                return Err(format!("tool `{tool}` requires dev mode"));
            }
        }
        if scope.limits.money_limit_minor_units > 0 && !self.config.allow_money {
            return Err("money limit > 0 but payments are disabled in policy".into());
        }
        Ok(())
    }

    /// The main entry point. Rules are evaluated in order of severity; the
    /// first `Deny` wins, otherwise the strictest of the remaining verdicts.
    pub fn check(&self, call: &ToolCall, ctx: &CheckContext<'_>) -> Decision {
        // 1. Hard denies.
        if let Some(d) = self.hard_denies(call, ctx) {
            return d;
        }

        let mut decision = Decision::allow("allow");
        let mut details = vec![];

        // 2. Scope: tool allow-list and origin.
        if !ctx.scope.allows_tool(&call.tool) {
            return Decision::deny("scope.tool_not_allowed", format!("tool `{}` is not in the task scope", call.tool));
        }
        if let Some(target) = &call.target_origin {
            if !ctx.scope.allows_origin(target) && !ctx.grants.has(GrantKind::OriginScope, target.as_str(), ctx.now) {
                decision = decision_stricter(
                    decision,
                    Decision::confirm("scope.origin_out_of_scope", format!("`{target}` is outside the declared scope")),
                );
            }
        }
        // Navigation to a new origin is checked against the scope too.
        if let Some(url_arg) = call.args.get("url") {
            if let Some(url) = url_arg.value.as_str() {
                if has_blocked_scheme(url) {
                    return Decision::deny("url.blocked", format!("navigation to `{url}` is blocked by policy"));
                }
                match Origin::parse(url) {
                    Ok(dest) => {
                        if self.is_blocked_url(url, &dest) {
                            return Decision::deny(
                                "url.blocked",
                                format!("navigation to `{url}` is blocked by policy"),
                            );
                        }
                        if !ctx.scope.allows_origin(&dest)
                            && !ctx.grants.has(GrantKind::OriginScope, dest.as_str(), ctx.now)
                        {
                            decision = decision_stricter(
                                decision,
                                Decision::confirm(
                                    "scope.navigation_out_of_scope",
                                    format!("navigating to `{dest}` outside scope"),
                                ),
                            );
                        }
                    }
                    Err(_) => return Decision::deny("url.invalid", format!("invalid url `{url}`")),
                }
            }
        }

        // 3. Argument provenance: restore SOP for the agent.
        for (name, value) in &call.args {
            let ann = ctx.manifest.args.get(name).cloned().unwrap_or_default();
            match &value.provenance {
                Provenance::Model => {
                    if !ann.free_text_ok && ann.kind == ArgKind::FreeText {
                        return Decision::deny(
                            "args.free_text_not_allowed",
                            format!("argument `{name}` does not accept model-authored free text"),
                        );
                    }
                    if !ann.allowed_provenance.contains(&ProvenanceClass::Model) && !ann.free_text_ok {
                        return Decision::deny(
                            "args.provenance.model",
                            format!("argument `{name}` cannot come from the model"),
                        );
                    }
                }
                Provenance::Origin { origin } => {
                    let same = call.target_origin.as_ref().map(|t| t == origin).unwrap_or(false);
                    if !same {
                        let to = call.target_origin.clone();
                        let preapproved = to.as_ref().map(|t| ctx.scope.flow_preapproved(origin, t)).unwrap_or(false)
                            || to
                                .as_ref()
                                .map(|t| ctx.grants.has(GrantKind::CrossOriginFlow, &format!("{origin}->{t}"), ctx.now))
                                .unwrap_or(false);
                        if !preapproved {
                            details.push(format!("`{name}` was read on {origin}"));
                            decision = decision_stricter(
                                decision,
                                Decision::confirm(
                                    "flow.cross_origin",
                                    format!(
                                        "argument `{name}` carries data read on `{origin}` into `{}`",
                                        to.map(|t| t.to_string()).unwrap_or_else(|| "another origin".into())
                                    ),
                                ),
                            );
                        }
                    } else if !ann.allowed_provenance.contains(&ProvenanceClass::OriginSame) {
                        decision = decision_stricter(
                            decision,
                            Decision::confirm(
                                "args.provenance.origin_same",
                                format!("argument `{name}` is page-derived"),
                            ),
                        );
                    }
                }
                Provenance::User | Provenance::Memory { .. } | Provenance::Tool { .. } => {}
            }
        }

        // 4. Consequential classification.
        let consequential = consequential::is_consequential(call, ctx.manifest);
        if consequential {
            if ctx.scope.limits.money_limit_minor_units == 0 && consequential::looks_like_payment(call) {
                return Decision::deny("money.not_allowed", "payment step reached but the task has no money limit");
            }
            if ctx.consequential_so_far >= ctx.scope.limits.max_consequential_actions {
                return Decision::deny("limits.max_consequential", "maximum number of consequential actions reached");
            }
            decision = decision_stricter(
                decision,
                Decision::confirm(
                    "action.consequential",
                    format!("`{}` is an irreversible action and needs your confirmation", describe_target(call)),
                ),
            );
            decision.consequential = true;
        }

        // 5. Step budget.
        if ctx.steps_so_far >= ctx.scope.limits.max_steps {
            return Decision::deny("limits.max_steps", "step budget exhausted");
        }

        decision.details.extend(details);
        decision
    }

    fn hard_denies(&self, call: &ToolCall, ctx: &CheckContext<'_>) -> Option<Decision> {
        if call.tool != ctx.manifest.name {
            return Some(Decision::deny("manifest.mismatch", "tool call does not match manifest"));
        }
        if ctx.manifest.safety.requires_dev_mode && !self.config.dev_mode_enabled {
            return Some(Decision::deny("tool.dev_mode_required", format!("`{}` requires dev mode", call.tool)));
        }
        if ctx.scope.profile == ScopeProfile::UserReadonly && !ctx.manifest.safety.read_only {
            return Some(Decision::deny("profile.readonly", "only read-only tools may run in the user's profile"));
        }
        for (name, v) in &call.args {
            if v.sensitivity == Sensitivity::Secret {
                return Some(Decision::deny("args.secret", format!("argument `{name}` carries secret data")));
            }
        }
        if let Some(t) = &call.target_origin {
            if self.is_blocked_origin(t) {
                return Some(Decision::deny("origin.blocked", format!("`{t}` is blocked by policy")));
            }
        }
        None
    }

    fn is_blocked_origin(&self, origin: &Origin) -> bool {
        if !origin.is_https() {
            return true;
        }
        if origin.is_loopback() && !self.config.dev_mode_enabled {
            return true;
        }
        if origin.is_loopback() {
            return !self.config.dev_localhost_origins.iter().any(|o| o == origin.as_str());
        }
        let host = origin.host();
        host.starts_with("10.") || host.starts_with("192.168.") || host.starts_with("169.254.")
    }

    fn is_blocked_url(&self, url: &str, origin: &Origin) -> bool {
        has_blocked_scheme(url) || self.is_blocked_origin(origin)
    }
}

fn has_blocked_scheme(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    ["file:", "browser:", "chrome:", "chrome-extension:", "javascript:", "data:", "view-source:", "about:"]
        .iter()
        .any(|s| lower.starts_with(s))
}

fn decision_stricter(a: Decision, b: Decision) -> Decision {
    // Keep the rule of whichever verdict is stricter.
    match (&a.verdict, &b.verdict) {
        (Verdict::Deny { .. }, _) => a,
        (_, Verdict::Deny { .. }) => b,
        (Verdict::Confirm { .. }, _) => a,
        (_, Verdict::Confirm { .. }) => b,
        _ => a,
    }
}

fn describe_target(call: &ToolCall) -> String {
    match &call.target {
        Some(t) if !t.name.is_empty() => format!("{} \"{}\"", t.role, t.name),
        _ => call.tool.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{ArgAnnotation, ArgKind, ConsequentialPredicate, LabeledValue, ProvenanceClass, TargetInfo};

    fn o(s: &str) -> Origin {
        Origin::parse(s).unwrap()
    }

    fn click_manifest() -> ToolManifest {
        ToolManifest::builtin("click", "Click an element by ref", false)
            .with_arg("ref", ArgAnnotation { kind: ArgKind::Ref, free_text_ok: false, ..Default::default() })
            .with_consequential_when(vec![
                ConsequentialPredicate::TargetIsSubmit,
                ConsequentialPredicate::TargetTextMatchesConsequentialLexicon,
                ConsequentialPredicate::FormHasPaymentFields,
            ])
    }

    fn type_manifest() -> ToolManifest {
        ToolManifest::builtin("type", "Type text into a field", false)
            .with_arg("ref", ArgAnnotation { kind: ArgKind::Ref, ..Default::default() })
            .with_arg(
                "text",
                ArgAnnotation {
                    kind: ArgKind::FreeText,
                    free_text_ok: true,
                    allowed_provenance: vec![
                        ProvenanceClass::User,
                        ProvenanceClass::Memory,
                        ProvenanceClass::Model,
                        ProvenanceClass::OriginSame,
                    ],
                    secret_forbidden: true,
                },
            )
    }

    fn navigate_manifest() -> ToolManifest {
        ToolManifest::builtin("navigate", "Open a URL", false).with_arg(
            "url",
            ArgAnnotation {
                kind: ArgKind::Url,
                free_text_ok: true,
                allowed_provenance: vec![ProvenanceClass::User, ProvenanceClass::Model],
                secret_forbidden: true,
            },
        )
    }

    fn ctx<'a>(scope: &'a TaskScope, manifest: &'a ToolManifest, grants: &'a GrantStore) -> CheckContext<'a> {
        CheckContext { scope, manifest, grants, now: 1_000, consequential_so_far: 0, steps_so_far: 0 }
    }

    #[test]
    fn cross_origin_data_flow_requires_confirmation() {
        // Data read on shop-a is typed into a form on shop-b: the classic exfiltration path.
        let scope = TaskScope::new(["https://shop-a.example", "https://shop-b.example"], ["type"]);
        let m = type_manifest();
        let grants = GrantStore::default();
        let call = ToolCall::new("c1", "type")
            .on(o("https://shop-b.example"))
            .arg("ref", LabeledValue::from_origin(12, o("https://shop-b.example"), Sensitivity::Public))
            .arg("text", LabeledValue::from_origin("order #4411", o("https://shop-a.example"), Sensitivity::Private));
        let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &m, &grants));
        assert_eq!(d.rule, "flow.cross_origin");
        assert!(matches!(d.verdict, Verdict::Confirm { .. }));
    }

    #[test]
    fn preapproved_flow_is_allowed() {
        let mut scope = TaskScope::new(["https://shop-a.example", "https://sheet.example"], ["type"]);
        scope.cross_origin_flows.push(core_types::CrossOriginFlow {
            from: "https://shop-a.example".into(),
            to: "https://sheet.example".into(),
            fields: vec![],
        });
        let m = type_manifest();
        let grants = GrantStore::default();
        let call = ToolCall::new("c1", "type")
            .on(o("https://sheet.example"))
            .arg("ref", LabeledValue::from_origin(1, o("https://sheet.example"), Sensitivity::Public))
            .arg("text", LabeledValue::from_origin("19.99", o("https://shop-a.example"), Sensitivity::Public));
        let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &m, &grants));
        assert_eq!(d.verdict, Verdict::Allow, "{d:?}");
    }

    #[test]
    fn secrets_are_denied_before_anything_else() {
        let scope = TaskScope::new(["https://a.example"], ["type"]);
        let m = type_manifest();
        let grants = GrantStore::default();
        let call = ToolCall::new("c1", "type").on(o("https://a.example")).arg(
            "text",
            LabeledValue { value: "hunter2".into(), provenance: Provenance::User, sensitivity: Sensitivity::Secret },
        );
        let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &m, &grants));
        assert_eq!(d.rule, "args.secret");
        assert!(matches!(d.verdict, Verdict::Deny { .. }));
    }

    #[test]
    fn injected_instruction_cannot_widen_scope() {
        // Even if the model was tricked into navigating to attacker.example, it is outside scope → confirm,
        // and the tool `send_message` is not in the allow-list → deny.
        let scope = TaskScope::new(["https://news.example"], ["navigate"]);
        let nav = navigate_manifest();
        let grants = GrantStore::default();
        let call = ToolCall::new("c1", "navigate")
            .on(o("https://news.example"))
            .arg("url", LabeledValue::model("https://attacker.example/steal"));
        let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &nav, &grants));
        assert_eq!(d.rule, "scope.navigation_out_of_scope");

        let send = ToolManifest::builtin("send_message", "send", false);
        let call2 = ToolCall::new("c2", "send_message").on(o("https://news.example"));
        let d2 = PolicyEngine::new(PolicyConfig::default()).check(&call2, &ctx(&scope, &send, &grants));
        assert_eq!(d2.rule, "scope.tool_not_allowed");
    }

    #[test]
    fn blocked_urls_are_denied() {
        let scope = TaskScope::new(["https://a.example"], ["navigate"]);
        let nav = navigate_manifest();
        let grants = GrantStore::default();
        for url in [
            "file:///etc/passwd",
            "chrome://settings",
            "http://a.example/",
            "https://localhost:8080/",
            "https://192.168.0.1/",
        ] {
            let call = ToolCall::new("c", "navigate").on(o("https://a.example")).arg("url", LabeledValue::user(url));
            let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &nav, &grants));
            assert!(matches!(d.verdict, Verdict::Deny { .. }), "{url} → {d:?}");
        }
    }

    #[test]
    fn dev_mode_can_allow_specific_localhost_origin() {
        let scope = TaskScope::new(["https://localhost:3000"], ["navigate"]);
        let nav = navigate_manifest();
        let grants = GrantStore::default();
        let cfg = PolicyConfig {
            dev_mode_enabled: true,
            dev_localhost_origins: vec!["https://localhost:3000".into()],
            ..Default::default()
        };
        let call = ToolCall::new("c", "navigate")
            .on(o("https://localhost:3000"))
            .arg("url", LabeledValue::user("https://localhost:3000/app"));
        let d = PolicyEngine::new(cfg).check(&call, &ctx(&scope, &nav, &grants));
        assert_eq!(d.verdict, Verdict::Allow, "{d:?}");
    }

    #[test]
    fn consequential_click_requires_confirmation_and_payment_is_denied_without_money_limit() {
        let scope = TaskScope::new(["https://shop.example"], ["click"]);
        let m = click_manifest();
        let grants = GrantStore::default();
        let pay = ToolCall::new("c", "click")
            .on(o("https://shop.example"))
            .arg("ref", LabeledValue::from_origin(7, o("https://shop.example"), Sensitivity::Public))
            .target(TargetInfo {
                role: "button".into(),
                name: "Оплатить заказ".into(),
                is_submit: true,
                form_has_payment_fields: true,
                ..Default::default()
            });
        let d = PolicyEngine::new(PolicyConfig::default()).check(&pay, &ctx(&scope, &m, &grants));
        assert_eq!(d.rule, "money.not_allowed");

        let publish = ToolCall::new("c", "click")
            .on(o("https://shop.example"))
            .arg("ref", LabeledValue::from_origin(8, o("https://shop.example"), Sensitivity::Public))
            .target(TargetInfo { role: "button".into(), name: "Publish".into(), ..Default::default() });
        let d = PolicyEngine::new(PolicyConfig::default()).check(&publish, &ctx(&scope, &m, &grants));
        assert_eq!(d.rule, "action.consequential");
        assert!(d.consequential);

        let harmless = ToolCall::new("c", "click")
            .on(o("https://shop.example"))
            .arg("ref", LabeledValue::from_origin(9, o("https://shop.example"), Sensitivity::Public))
            .target(TargetInfo { role: "link".into(), name: "Next page".into(), ..Default::default() });
        let d = PolicyEngine::new(PolicyConfig::default()).check(&harmless, &ctx(&scope, &m, &grants));
        assert_eq!(d.verdict, Verdict::Allow);
    }

    #[test]
    fn model_free_text_only_where_declared() {
        let scope = TaskScope::new(["https://a.example"], ["click"]);
        let m = click_manifest();
        let grants = GrantStore::default();
        let call = ToolCall::new("c", "click").on(o("https://a.example")).arg("ref", LabeledValue::model("button.buy"));
        let d = PolicyEngine::new(PolicyConfig::default()).check(&call, &ctx(&scope, &m, &grants));
        assert_eq!(d.rule, "args.provenance.model");
    }

    #[test]
    fn readonly_profile_rejects_acting_tools() {
        let mut scope = TaskScope::new(["https://a.example"], ["click"]);
        scope.profile = ScopeProfile::UserReadonly;
        let m = click_manifest();
        let engine = PolicyEngine::new(PolicyConfig::default());
        assert!(engine.validate_scope(&scope, &[&m]).is_err());
        let grants = GrantStore::default();
        let call = ToolCall::new("c", "click").on(o("https://a.example"));
        assert_eq!(engine.check(&call, &ctx(&scope, &m, &grants)).rule, "profile.readonly");
    }

    #[test]
    fn grants_expire() {
        let scope = TaskScope::new(["https://a.example"], ["navigate"]);
        let nav = navigate_manifest();
        let mut grants = GrantStore::default();
        grants.add(Grant::new(GrantKind::OriginScope, "agent", "https://b.example", 0, 500));
        let call = ToolCall::new("c", "navigate")
            .on(o("https://a.example"))
            .arg("url", LabeledValue::user("https://b.example/x"));
        let engine = PolicyEngine::new(PolicyConfig::default());
        let mut c = ctx(&scope, &nav, &grants);
        c.now = 100;
        assert_eq!(engine.check(&call, &c).verdict, Verdict::Allow);
        c.now = 1_000;
        assert_eq!(engine.check(&call, &c).rule, "scope.navigation_out_of_scope");
    }
}
