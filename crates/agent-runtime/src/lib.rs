//! Agent Runtime (ADR-005).
//!
//! One step of the loop:
//!
//! ```text
//! observe ──► plan ──► label args ──► PolicyEngine.check ──► blind critic
//!    ▲                                      │ deny              │ deny
//!    │                                      ▼                   ▼
//!    │                              feedback to planner   feedback to planner
//!    │                                      │ allow/confirm
//!    │                                      ▼
//!    │                       confirm? ──► user (or stop in dry-run)
//!    │                                      │ approved
//!    │                                      ▼
//!    └──────────── journal ◄──── execute via EngineAdapter
//! ```
//!
//! Every branch writes to the journal. A `Deny` from PolicyEngine is final;
//! neither the planner nor the critic can override it.

pub mod io;
pub mod mcp;
pub mod planner;
pub mod tools;

pub use io::*;
pub use mcp::*;
pub use planner::*;
pub use tools::*;

use std::collections::BTreeMap;
use std::sync::Arc;

use core_types::{LabeledValue, Observation, Provenance, Sensitivity, TaskScope, ToolCall, ToolResult, Verdict};
use engine_adapter::{EngineAdapter, EngineEvent, WebViewOptions};
use model_gateway::ToolSpec;
use page_intelligence::budget::{trim_observation, ObservationBudget};
use policy::{CheckContext, Decision, Grant, GrantKind, GrantStore, PolicyEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Engine(#[from] engine_adapter::EngineError),
    #[error(transparent)]
    Model(#[from] model_gateway::ModelError),
    #[error("invalid scope: {0}")]
    InvalidScope(String),
    #[error("bad tool arguments: {0}")]
    BadToolArgs(String),
    #[error("unknown tool `{0}`")]
    UnknownTool(String),
    #[error("element ref {0} is not in the current observation")]
    UnknownRef(u64),
    #[error("element ref {0} is a masked field; the agent never types into masked fields")]
    MaskedField(u64),
    #[error("agent actions are disabled in policy")]
    AgentDisabled,
    #[error("session {0} is closed")]
    SessionClosed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Live,
    /// Execute reversible steps; stop at the first action that would need
    /// confirmation and report what *would* happen.
    DryRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StepOutcome {
    Acted {
        call: ToolCall,
        result: ToolResult,
    },
    Denied {
        call: ToolCall,
        reason: String,
        rule: String,
    },
    Rejected {
        call: ToolCall,
    },
    /// Dry run reached an action that needs confirmation.
    DryRunStopped {
        call: ToolCall,
        reason: String,
    },
    Finished {
        summary: String,
    },
    NeedsUser {
        question: String,
    },
    HumanChallenge,
    LimitReached {
        what: String,
    },
}

impl StepOutcome {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            StepOutcome::Finished { .. }
                | StepOutcome::NeedsUser { .. }
                | StepOutcome::HumanChallenge
                | StepOutcome::LimitReached { .. }
                | StepOutcome::DryRunStopped { .. }
        )
    }
}

pub struct Session {
    pub id: String,
    pub webview: String,
    pub request: String,
    pub request_sensitivity: Sensitivity,
    pub scope: TaskScope,
    pub mode: ExecutionMode,
    pub grants: GrantStore,
    pub history: Vec<StepSummary>,
    /// Labelled outputs of executed calls, addressable as `$result:<call_id>`.
    pub results: BTreeMap<String, LabeledValue>,
    pub steps: u32,
    pub consequential: u32,
    pub consecutive_denies: u32,
    pub open: bool,
    pub started_at: i64,
}

/// Resolves `$mem:<id>` references to `(text, sensitivity)`.
pub type MemoryLookup = Arc<dyn Fn(&str) -> Option<(String, Sensitivity)> + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct AgentRunner {
    engine: Arc<dyn EngineAdapter>,
    policy: PolicyEngine,
    tools: ToolRegistry,
    planner: Arc<dyn Planner>,
    critic: Arc<dyn Critic>,
    confirmer: Arc<dyn ConfirmationHandler>,
    journal: Arc<dyn Journal>,
    memory_lookup: MemoryLookup,
    now: Clock,
    /// Planner is denied more than this many times in a row → the session stops.
    pub max_consecutive_denies: u32,
}

impl AgentRunner {
    pub fn new(
        engine: Arc<dyn EngineAdapter>,
        policy: PolicyEngine,
        tools: ToolRegistry,
        planner: Arc<dyn Planner>,
        critic: Arc<dyn Critic>,
        confirmer: Arc<dyn ConfirmationHandler>,
        journal: Arc<dyn Journal>,
    ) -> Self {
        Self {
            engine,
            policy,
            tools,
            planner,
            critic,
            confirmer,
            journal,
            memory_lookup: Arc::new(|_| None),
            now: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            }),
            max_consecutive_denies: 3,
        }
    }

    pub fn with_memory_lookup(mut self, f: MemoryLookup) -> Self {
        self.memory_lookup = f;
        self
    }

    pub fn with_clock(mut self, f: Clock) -> Self {
        self.now = f;
        self
    }

    /// Validate the scope, open an isolated agent webview and navigate to the
    /// first origin. `request_sensitivity` comes from `policy::sensitivity::classify_user_text`.
    pub async fn start(
        &self,
        request: &str,
        request_sensitivity: Sensitivity,
        scope: TaskScope,
        start_url: &str,
        mode: ExecutionMode,
    ) -> Result<Session, AgentError> {
        if !self.policy.config().agent_enabled {
            return Err(AgentError::AgentDisabled);
        }
        let manifests: Vec<&core_types::ToolManifest> = self.tools.manifests().collect();
        self.policy.validate_scope(&scope, &manifests).map_err(AgentError::InvalidScope)?;
        let start_origin = core_types::Origin::parse(start_url).map_err(|e| AgentError::InvalidScope(e.to_string()))?;
        if !scope.allows_origin(&start_origin) {
            return Err(AgentError::InvalidScope(format!("start url `{start_url}` is outside the scope")));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let mut opts = WebViewOptions::agent(&id);
        opts.observe_iframe_origins = scope.observe_cross_origin_iframes.clone();
        let webview = self.engine.create_webview(opts).await?;
        self.engine.navigate(&webview, start_url).await?;

        self.journal.record(JournalEvent::SessionStarted {
            session_id: id.clone(),
            request: request.to_string(),
            scope: scope.clone(),
            dry_run: mode == ExecutionMode::DryRun,
        });
        Ok(Session {
            id,
            webview,
            request: request.to_string(),
            request_sensitivity,
            scope,
            mode,
            grants: GrantStore::default(),
            history: vec![],
            results: BTreeMap::new(),
            steps: 0,
            consequential: 0,
            consecutive_denies: 0,
            open: true,
            started_at: (self.now)(),
        })
    }

    pub async fn close(&self, session: &mut Session, status: &str, outcome: Option<String>) -> Result<(), AgentError> {
        if session.open {
            session.open = false;
            let _ = self.engine.close_webview(&session.webview).await;
            self.journal.record(JournalEvent::SessionEnded {
                session_id: session.id.clone(),
                status: status.into(),
                outcome,
            });
        }
        Ok(())
    }

    /// Run until a terminal outcome or the step budget.
    pub async fn run(&self, session: &mut Session) -> Result<StepOutcome, AgentError> {
        loop {
            let outcome = self.step(session).await?;
            if outcome.is_terminal() {
                let status = match &outcome {
                    StepOutcome::Finished { .. } => "finished",
                    StepOutcome::DryRunStopped { .. } => "dry_run_stopped",
                    StepOutcome::NeedsUser { .. } => "needs_user",
                    StepOutcome::HumanChallenge => "human_challenge",
                    _ => "stopped",
                };
                let summary = serde_json::to_string(&outcome).ok();
                self.close(session, status, summary).await?;
                return Ok(outcome);
            }
        }
    }

    pub async fn step(&self, session: &mut Session) -> Result<StepOutcome, AgentError> {
        if !session.open {
            return Err(AgentError::SessionClosed(session.id.clone()));
        }
        let now = (self.now)();
        if session.steps >= session.scope.limits.max_steps {
            return Ok(StepOutcome::LimitReached { what: "max_steps".into() });
        }
        if (now - session.started_at) as u64 >= session.scope.limits.max_duration_ms {
            return Ok(StepOutcome::LimitReached { what: "max_duration".into() });
        }
        if session.consecutive_denies >= self.max_consecutive_denies {
            return Ok(StepOutcome::LimitReached { what: "consecutive_denies".into() });
        }

        for ev in self.engine.poll_events().await? {
            if matches!(ev, EngineEvent::HumanChallenge { ref webview } if *webview == session.webview) {
                return Ok(StepOutcome::HumanChallenge);
            }
        }

        // 1. Observe (trimmed to the local budget unless cloud planning is allowed).
        let mut observation = self.engine.observe(&session.webview).await?;
        let budget = if session.scope.model_policy.allow_cloud_planning {
            ObservationBudget::CLOUD
        } else {
            ObservationBudget::LOCAL
        };
        trim_observation(&mut observation, budget);
        let before_hash = observation.page.snapshot_hash.clone();

        // 2. Plan.
        let specs: Vec<ToolSpec> = self.tools.specs_for(&session.scope.tools);
        let sensitivity = Sensitivity::max_of([session.request_sensitivity, observation.page.sensitivity]);
        let step = self
            .planner
            .plan(PlanContext {
                user_request: &session.request,
                observation: &observation,
                history: &session.history,
                tools: &specs,
                sensitivity,
                allow_cloud: session.scope.model_policy.allow_cloud_planning,
            })
            .await?;
        session.steps += 1;
        let ordinal = session.steps;

        let (call_id, tool, args, thought) = match step {
            PlanStep::Finish { summary } => return Ok(StepOutcome::Finished { summary }),
            PlanStep::AskUser { question } => return Ok(StepOutcome::NeedsUser { question }),
            PlanStep::Call { call_id, tool, args, thought } => (call_id, tool, args, thought),
        };
        self.journal.record(JournalEvent::Step {
            session_id: session.id.clone(),
            ordinal,
            observation_hash: before_hash.clone(),
            observation_tokens: observation.approx_tokens,
            thought,
        });

        // 3. Label arguments (refs → provenance) and check policy.
        let Some(manifest) = self.tools.get(&tool).cloned() else {
            return Ok(self.deny_feedback(
                session,
                ToolCall::new(call_id, &tool),
                "tool.unknown",
                format!("unknown tool `{tool}`"),
                &before_hash,
            ));
        };
        let lookup = self.memory_lookup.clone();
        let call = match label_call(
            &call_id,
            &tool,
            &args,
            &manifest,
            &ResolveContext {
                observation: &observation,
                user_request: &session.request,
                user_request_sensitivity: session.request_sensitivity,
                memory_lookup: &*lookup,
                results: &session.results,
            },
        ) {
            Ok(c) => c,
            Err(e) => {
                return Ok(self.deny_feedback(
                    session,
                    ToolCall::new(call_id, &tool),
                    "args.invalid",
                    e.to_string(),
                    &before_hash,
                ))
            }
        };

        let decision = self.policy.check(
            &call,
            &CheckContext {
                scope: &session.scope,
                manifest: &manifest,
                grants: &session.grants,
                now,
                consequential_so_far: session.consequential,
                steps_so_far: session.steps - 1,
            },
        );
        if let Verdict::Deny { reason } = &decision.verdict {
            let rule = decision.rule.clone();
            return Ok(self.deny_feedback(session, call, &rule, reason.clone(), &before_hash));
        }

        // 4. Blind critic. Its verdict can only make things stricter.
        let critic_verdict =
            self.critic.review(&CriticInput::from_call(&session.request, &call, &decision, session.steps)).await?;
        let verdict = decision.verdict.clone().stricter(critic_verdict.clone());
        if let Verdict::Deny { reason } = &verdict {
            self.record_action(
                session,
                ordinal,
                &call,
                &decision,
                Some(critic_verdict),
                None,
                false,
                None,
                Some(reason.clone()),
                &before_hash,
                None,
            );
            session.consecutive_denies += 1;
            session.history.push(StepSummary {
                ordinal,
                tool: call.tool.clone(),
                target: target_label(&call),
                outcome: format!("denied by critic: {reason}"),
            });
            return Ok(StepOutcome::Denied { call, reason: reason.clone(), rule: "critic.deny".into() });
        }

        // 5. Confirmation (or dry-run stop).
        let mut approval = None;
        if let Verdict::Confirm { reason } = &verdict {
            if session.mode == ExecutionMode::DryRun {
                self.record_action(
                    session,
                    ordinal,
                    &call,
                    &decision,
                    Some(critic_verdict),
                    None,
                    false,
                    None,
                    Some(format!("dry run: {reason}")),
                    &before_hash,
                    None,
                );
                session.history.push(StepSummary {
                    ordinal,
                    tool: call.tool.clone(),
                    target: target_label(&call),
                    outcome: "dry run stopped".into(),
                });
                return Ok(StepOutcome::DryRunStopped { call, reason: reason.clone() });
            }
            let request = ConfirmationRequest {
                session_id: session.id.clone(),
                call: call.clone(),
                reason: reason.clone(),
                rule: decision.rule.clone(),
                consequential: decision.consequential,
                preview: preview(&call),
                details: decision.details.clone(),
            };
            let answer = self.confirmer.confirm(&request).await?;
            match answer {
                ConfirmationAnswer::Rejected => {
                    self.record_action(
                        session,
                        ordinal,
                        &call,
                        &decision,
                        Some(critic_verdict),
                        Some(answer),
                        false,
                        None,
                        Some("rejected by user".into()),
                        &before_hash,
                        None,
                    );
                    session.history.push(StepSummary {
                        ordinal,
                        tool: call.tool.clone(),
                        target: target_label(&call),
                        outcome: "rejected by user".into(),
                    });
                    return Ok(StepOutcome::Rejected { call });
                }
                ConfirmationAnswer::ApprovedForSession => {
                    self.remember_grant(session, &call, &decision, now);
                    approval = Some(answer);
                }
                ConfirmationAnswer::Approved => approval = Some(answer),
            }
        }

        // 6. Execute.
        let result = self.execute(session, &call, &observation).await;
        let after_hash = self.engine.observe(&session.webview).await.ok().map(|o| o.page.snapshot_hash);
        let (tool_result, error) = match result {
            Ok(r) => (r, None),
            Err(e) => (
                ToolResult {
                    call_id: call.id.clone(),
                    ok: false,
                    output: LabeledValue::model(Value::Null),
                    error: Some(e.to_string()),
                },
                Some(e.to_string()),
            ),
        };
        if decision.consequential && tool_result.ok {
            session.consequential += 1;
        }
        if tool_result.ok {
            session.results.insert(call.id.clone(), tool_result.output.clone());
        }
        session.consecutive_denies = 0;
        self.record_action(
            session,
            ordinal,
            &call,
            &decision,
            Some(critic_verdict),
            approval,
            true,
            Some(tool_result.output.value.clone()),
            error,
            &before_hash,
            after_hash,
        );
        session.history.push(StepSummary {
            ordinal,
            tool: call.tool.clone(),
            target: target_label(&call),
            outcome: if tool_result.ok {
                "ok".into()
            } else {
                format!("error: {}", tool_result.error.clone().unwrap_or_default())
            },
        });
        Ok(StepOutcome::Acted { call, result: tool_result })
    }

    async fn execute(
        &self,
        session: &Session,
        call: &ToolCall,
        observation: &Observation,
    ) -> Result<ToolResult, AgentError> {
        let origin = observation.page.origin.clone();
        let page_sens = observation.page.sensitivity;
        match to_action(call, observation)? {
            Some(action) => {
                let r = self.engine.act(&session.webview, action).await?;
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    ok: r.ok,
                    output: LabeledValue::from_origin(
                        json!({ "url": r.url, "message": r.message, "output": r.output }),
                        origin,
                        page_sens,
                    ),
                    error: None,
                })
            }
            None => {
                match call.tool.as_str() {
                    "extract" => {
                        let ids: Vec<String> = match call.args.get("obs_ids").map(|v| &v.value) {
                            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                            Some(Value::String(s)) => vec![s.clone()],
                            _ => vec![],
                        };
                        let texts: Vec<Value> = observation
                        .content
                        .iter()
                        .filter(|c| ids.contains(&c.obs_id))
                        .map(|c| json!({ "obs_id": c.obs_id, "text": c.text, "suspect_injection": c.suspect_injection }))
                        .collect();
                        Ok(ToolResult {
                            call_id: call.id.clone(),
                            ok: true,
                            output: LabeledValue::from_origin(Value::Array(texts), origin, page_sens),
                            error: None,
                        })
                    }
                    "search_memory" => Ok(ToolResult {
                        call_id: call.id.clone(),
                        ok: false,
                        output: LabeledValue {
                            value: Value::Null,
                            provenance: Provenance::Tool { name: "search_memory".into() },
                            sensitivity: Sensitivity::Personal,
                        },
                        error: Some("search_memory is wired by the application (needs MemoryStore)".into()),
                    }),
                    other => Err(AgentError::UnknownTool(other.to_string())),
                }
            }
        }
    }

    fn deny_feedback(
        &self,
        session: &mut Session,
        call: ToolCall,
        rule: &str,
        reason: String,
        before_hash: &str,
    ) -> StepOutcome {
        let decision = Decision {
            verdict: Verdict::Deny { reason: reason.clone() },
            rule: rule.to_string(),
            consequential: false,
            details: vec![],
        };
        self.record_action(
            session,
            session.steps,
            &call,
            &decision,
            None,
            None,
            false,
            None,
            Some(reason.clone()),
            before_hash,
            None,
        );
        session.consecutive_denies += 1;
        session.history.push(StepSummary {
            ordinal: session.steps,
            tool: call.tool.clone(),
            target: target_label(&call),
            outcome: format!("denied ({rule}): {reason}"),
        });
        StepOutcome::Denied { call, reason, rule: rule.to_string() }
    }

    /// "Approve for session" turns a confirmation into a grant so the same
    /// class of action is not asked again. Consequential actions are never
    /// granted in bulk.
    fn remember_grant(&self, session: &mut Session, call: &ToolCall, decision: &Decision, now: i64) {
        if decision.consequential {
            return;
        }
        let until = session.started_at + session.scope.limits.max_duration_ms as i64;
        match decision.rule.as_str() {
            "scope.origin_out_of_scope" | "scope.navigation_out_of_scope" => {
                if let Some(o) = &call.target_origin {
                    session.grants.add(Grant::new(GrantKind::OriginScope, &session.id, o.as_str(), now, until));
                }
                if let Some(url) = call.args.get("url").and_then(|v| v.value.as_str()) {
                    if let Ok(o) = core_types::Origin::parse(url) {
                        session.grants.add(Grant::new(GrantKind::OriginScope, &session.id, o.as_str(), now, until));
                    }
                }
            }
            "flow.cross_origin" => {
                if let Some(to) = &call.target_origin {
                    for v in call.args.values() {
                        if let Provenance::Origin { origin } = &v.provenance {
                            if origin != to {
                                session.grants.add(Grant::new(
                                    GrantKind::CrossOriginFlow,
                                    &session.id,
                                    &format!("{origin}->{to}"),
                                    now,
                                    until,
                                ));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_action(
        &self,
        session: &Session,
        ordinal: u32,
        call: &ToolCall,
        decision: &Decision,
        critic: Option<Verdict>,
        approval: Option<ConfirmationAnswer>,
        executed: bool,
        result: Option<Value>,
        error: Option<String>,
        before_hash: &str,
        after_hash: Option<String>,
    ) {
        self.journal.record(JournalEvent::Action {
            session_id: session.id.clone(),
            ordinal,
            call: call.clone(),
            decision: decision.clone(),
            critic,
            approval,
            executed,
            result,
            error,
            before_hash: before_hash.to_string(),
            after_hash,
        });
    }
}

fn target_label(call: &ToolCall) -> Option<String> {
    call.target.as_ref().map(|t| format!("{} “{}”", t.role, t.name))
}

/// Deterministic confirmation preview built from the target snapshot.
pub fn preview(call: &ToolCall) -> String {
    let where_ = call.target_origin.as_ref().map(|o| format!(" on {o}")).unwrap_or_default();
    match (call.tool.as_str(), &call.target) {
        ("click", Some(t)) => format!("Click {} “{}”{where_}", t.role, t.name),
        ("type", Some(t)) => {
            format!("Type into {} “{}”{where_} (source: {})", t.role, t.name, provenance_label(call, "text"))
        }
        ("navigate", _) => format!("Open {}", call.args.get("url").and_then(|v| v.value.as_str()).unwrap_or("?")),
        ("call_site_tool", Some(t)) => format!("Call site tool “{}”{where_}", t.name),
        (tool, Some(t)) => format!("{tool} {} “{}”{where_}", t.role, t.name),
        (tool, None) => format!("{tool}{where_}"),
    }
}

fn provenance_label(call: &ToolCall, arg: &str) -> String {
    match call.args.get(arg).map(|v| &v.provenance) {
        Some(Provenance::User) => "your request".into(),
        Some(Provenance::Memory { .. }) => "your memory".into(),
        Some(Provenance::Origin { origin }) => format!("text read on {origin}"),
        Some(Provenance::Model) => "model-generated text".into(),
        Some(Provenance::Tool { name }) => format!("tool {name}"),
        None => "—".into(),
    }
}

#[cfg(test)]
mod tests;
