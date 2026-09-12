use super::*;
use core_types::TaskScope;
use engine_adapter::mock::{MockEngine, MockPage};
use policy::PolicyConfig;
use std::collections::VecDeque;
use std::sync::Mutex;

struct ScriptedPlanner(Mutex<VecDeque<PlanStep>>);

impl ScriptedPlanner {
    fn new(steps: Vec<PlanStep>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(steps.into())))
    }
}

#[async_trait::async_trait]
impl Planner for ScriptedPlanner {
    async fn plan(&self, _ctx: PlanContext<'_>) -> Result<PlanStep, AgentError> {
        Ok(self.0.lock().unwrap().pop_front().unwrap_or(PlanStep::Finish { summary: "done".into() }))
    }
}

fn call(id: &str, tool: &str, args: Value) -> PlanStep {
    PlanStep::Call { call_id: id.into(), tool: tool.into(), args, thought: None }
}

struct Harness {
    engine: Arc<MockEngine>,
    journal: Arc<InMemoryJournal>,
    confirmer: Arc<ScriptedConfirmations>,
    runner: AgentRunner,
}

fn harness(planner: Arc<dyn Planner>, answers: Vec<ConfirmationAnswer>) -> Harness {
    let engine = Arc::new(MockEngine::new());
    engine.add_page(
        MockPage::simple("https://shop.example/", "Shop", "Promo code today: SAVE10")
            .with_element(1, "button", "Details")
            .with_element(2, "textbox", "Promo code")
            .with_element(3, "button", "Place order"),
    );
    engine.add_page(
        MockPage::simple("https://notes.example/", "Notes", "My notes").with_element(10, "textbox", "New note"),
    );
    engine.add_page(MockPage::simple("https://evil.example/", "Evil", "..."));

    let journal = Arc::new(InMemoryJournal::default());
    let confirmer = Arc::new(ScriptedConfirmations::new(answers));
    let policy = PolicyEngine::new(PolicyConfig { agent_enabled: true, ..Default::default() });
    let runner = AgentRunner::new(
        engine.clone(),
        policy,
        ToolRegistry::builtin(),
        planner,
        Arc::new(PermissiveCritic),
        confirmer.clone(),
        journal.clone(),
    );
    Harness { engine, journal, confirmer, runner }
}

fn scope() -> TaskScope {
    TaskScope::new(["https://shop.example", "https://notes.example"], ["navigate", "click", "type", "extract"])
}

async fn start(h: &Harness, mode: ExecutionMode) -> Session {
    h.runner
        .start(
            "apply the promo code shown on the shop page",
            Sensitivity::Personal,
            scope(),
            "https://shop.example/",
            mode,
        )
        .await
        .unwrap()
}

fn executed_actions(j: &InMemoryJournal) -> Vec<(String, bool)> {
    j.actions()
        .into_iter()
        .map(|e| match e {
            JournalEvent::Action { call, executed, .. } => (call.tool, executed),
            _ => unreachable!(),
        })
        .collect()
}

#[tokio::test]
async fn happy_path_executes_and_journals() {
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "click", json!({ "ref": 1 })),
            PlanStep::Finish { summary: "opened details".into() },
        ]),
        vec![],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    let out = h.runner.run(&mut s).await.unwrap();
    assert_eq!(out, StepOutcome::Finished { summary: "opened details".into() });
    assert_eq!(executed_actions(&h.journal), vec![("click".to_string(), true)]);
    assert_eq!(h.engine.actions.lock().unwrap().len(), 1);
    assert!(h.confirmer.seen.lock().unwrap().is_empty());
    let events = h.journal.events.lock().unwrap();
    assert!(matches!(events.first(), Some(JournalEvent::SessionStarted { .. })));
    assert!(matches!(events.last(), Some(JournalEvent::SessionEnded { status, .. }) if status == "finished"));
}

#[tokio::test]
async fn agent_disabled_by_default_policy() {
    let engine = Arc::new(MockEngine::new());
    let runner = AgentRunner::new(
        engine,
        PolicyEngine::new(PolicyConfig::default()),
        ToolRegistry::builtin(),
        ScriptedPlanner::new(vec![]),
        Arc::new(PermissiveCritic),
        Arc::new(ScriptedConfirmations::reject_all()),
        Arc::new(InMemoryJournal::default()),
    );
    let err = runner
        .start("x", Sensitivity::Personal, scope(), "https://shop.example/", ExecutionMode::Live)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, AgentError::AgentDisabled));
}

#[tokio::test]
async fn model_literal_text_is_denied_and_deny_is_final() {
    // Step 1: model authors literal text → PolicyEngine denies (critic is permissive and cannot override).
    // Step 2: model references the user's request → allowed.
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "type", json!({ "ref": 2, "text": "SAVE10" })),
            call("2", "type", json!({ "ref": 2, "text": "$user" })),
        ]),
        vec![],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Denied { ref rule, .. } if rule == "args.free_text_not_allowed"), "{out:?}");
    assert_eq!(s.consecutive_denies, 1);

    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Acted { .. }), "{out:?}");
    assert_eq!(s.consecutive_denies, 0);
    assert_eq!(executed_actions(&h.journal), vec![("type".to_string(), false), ("type".to_string(), true)]);
    assert!(s.history[0].outcome.contains("denied"));
}

#[tokio::test]
async fn cross_origin_data_flow_requires_confirmation_and_rejection_is_respected() {
    // extract promo text on shop.example, then type it into notes.example → flow.cross_origin.
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "extract", json!({ "obs_ids": ["c0"] })),
            call("2", "navigate", json!({ "url": "https://notes.example/" })),
            call("3", "type", json!({ "ref": 10, "text": "$result:1" })),
        ]),
        vec![ConfirmationAnswer::Rejected],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    assert!(matches!(h.runner.step(&mut s).await.unwrap(), StepOutcome::Acted { .. }));
    assert!(matches!(h.runner.step(&mut s).await.unwrap(), StepOutcome::Acted { .. }));
    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Rejected { .. }), "{out:?}");

    let seen = h.confirmer.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].rule, "flow.cross_origin");
    assert!(seen[0].preview.contains("text read on https://shop.example"), "{}", seen[0].preview);
    assert!(seen[0].details.iter().any(|d| d.contains("shop.example")));
    // The typed value never reached the engine.
    assert!(!h.engine.actions.lock().unwrap().iter().any(|(_, a)| matches!(a, engine_adapter::Action::Type { .. })));
}

#[tokio::test]
async fn cross_origin_flow_approved_for_session_creates_grant() {
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "extract", json!({ "obs_ids": ["c0"] })),
            call("2", "navigate", json!({ "url": "https://notes.example/" })),
            call("3", "type", json!({ "ref": 10, "text": "$result:1" })),
            call("4", "type", json!({ "ref": 10, "text": "$result:1" })),
        ]),
        vec![ConfirmationAnswer::ApprovedForSession],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    for _ in 0..4 {
        let out = h.runner.step(&mut s).await.unwrap();
        assert!(matches!(out, StepOutcome::Acted { .. }), "{out:?}");
    }
    assert_eq!(h.confirmer.seen.lock().unwrap().len(), 1, "second identical flow is covered by the grant");
    assert!(s.grants.has(
        policy::GrantKind::CrossOriginFlow,
        "https://shop.example->https://notes.example",
        s.started_at + 1
    ));
    let actions = h.engine.actions.lock().unwrap().clone();
    match &actions[1].1 {
        engine_adapter::Action::Type { text, .. } => assert_eq!(text, "Promo code today: SAVE10"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn consequential_action_stops_dry_run_without_executing() {
    let h = harness(
        ScriptedPlanner::new(vec![call("1", "click", json!({ "ref": 1 })), call("2", "click", json!({ "ref": 3 }))]),
        vec![ConfirmationAnswer::Approved],
    );
    let mut s = start(&h, ExecutionMode::DryRun).await;
    let out = h.runner.run(&mut s).await.unwrap();
    match out {
        StepOutcome::DryRunStopped { call, reason } => {
            assert_eq!(call.target.as_ref().unwrap().name, "Place order");
            assert!(reason.contains("irreversible"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    // Reversible click ran, consequential click did not; the user was never asked in dry run.
    assert_eq!(executed_actions(&h.journal), vec![("click".to_string(), true), ("click".to_string(), false)]);
    assert_eq!(h.engine.actions.lock().unwrap().len(), 1);
    assert!(h.confirmer.seen.lock().unwrap().is_empty());
    assert!(!s.open);
}

#[tokio::test]
async fn consequential_action_live_requires_confirmation_and_counts() {
    let h = harness(
        ScriptedPlanner::new(vec![call("1", "click", json!({ "ref": 3 }))]),
        vec![ConfirmationAnswer::Approved],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Acted { .. }), "{out:?}");
    assert_eq!(s.consequential, 1);
    let seen = h.confirmer.seen.lock().unwrap();
    assert_eq!(seen[0].rule, "action.consequential");
    assert!(seen[0].consequential);
    assert_eq!(seen[0].preview, "Click button “Place order” on https://shop.example");
}

#[tokio::test]
async fn out_of_scope_and_blocked_navigation() {
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "navigate", json!({ "url": "file:///etc/passwd" })),
            call("2", "navigate", json!({ "url": "https://evil.example/" })),
        ]),
        vec![ConfirmationAnswer::Rejected],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Denied { ref rule, .. } if rule == "url.blocked"), "{out:?}");
    let out = h.runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Rejected { .. }), "{out:?}");
    assert_eq!(h.confirmer.seen.lock().unwrap()[0].rule, "scope.navigation_out_of_scope");
    assert!(h.engine.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn repeated_denies_stop_the_session() {
    let h = harness(
        ScriptedPlanner::new(vec![
            call("1", "scroll", json!({})), // not in scope
            call("2", "scroll", json!({})),
            call("3", "scroll", json!({})),
            call("4", "click", json!({ "ref": 1 })),
        ]),
        vec![],
    );
    let mut s = start(&h, ExecutionMode::Live).await;
    let out = h.runner.run(&mut s).await.unwrap();
    assert_eq!(out, StepOutcome::LimitReached { what: "consecutive_denies".into() });
    assert!(h.engine.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn masked_fields_and_unknown_refs_never_reach_the_engine() {
    let engine = Arc::new(MockEngine::new());
    let mut page = MockPage::simple("https://shop.example/", "Login", "…").with_element(1, "textbox", "Password");
    page.interactive[0].state.masked = true;
    engine.add_page(page);
    let journal = Arc::new(InMemoryJournal::default());
    let runner = AgentRunner::new(
        engine.clone(),
        PolicyEngine::new(PolicyConfig { agent_enabled: true, ..Default::default() }),
        ToolRegistry::builtin(),
        ScriptedPlanner::new(vec![
            call("1", "type", json!({ "ref": 1, "text": "$user" })),
            call("2", "click", json!({ "ref": 42 })),
        ]),
        Arc::new(PermissiveCritic),
        Arc::new(ScriptedConfirmations::reject_all()),
        journal.clone(),
    );
    let mut s = runner
        .start(
            "log me in",
            Sensitivity::Personal,
            TaskScope::new(["https://shop.example"], ["type", "click"]),
            "https://shop.example/",
            ExecutionMode::Live,
        )
        .await
        .unwrap();
    let out = runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Denied { ref reason, .. } if reason.contains("masked")), "{out:?}");
    let out = runner.step(&mut s).await.unwrap();
    assert!(matches!(out, StepOutcome::Denied { ref reason, .. } if reason.contains("42")), "{out:?}");
    assert!(engine.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn scope_validation_rejects_bad_scopes() {
    let h = harness(ScriptedPlanner::new(vec![]), vec![]);
    let bad = TaskScope::new(["http://shop.example"], ["click"]);
    let err = h
        .runner
        .start("x", Sensitivity::Personal, bad, "http://shop.example/", ExecutionMode::Live)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, AgentError::InvalidScope(_)));

    let mut ro = TaskScope::new(["https://shop.example"], ["click"]);
    ro.profile = core_types::ScopeProfile::UserReadonly;
    let err = h
        .runner
        .start("x", Sensitivity::Personal, ro, "https://shop.example/", ExecutionMode::Live)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, AgentError::InvalidScope(ref m) if m.contains("read-only")), "{err}");

    let err = h
        .runner
        .start("x", Sensitivity::Personal, scope(), "https://evil.example/", ExecutionMode::Live)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, AgentError::InvalidScope(ref m) if m.contains("outside")), "{err}");
}
