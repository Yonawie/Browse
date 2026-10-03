//! Bridge from the runtime's `Journal` trait to the SQLite tables
//! (`agent_sessions`, `agent_steps`, `agent_actions`, `approvals`, `policy_decisions`).
//!
//! The runtime generates its own session ids; the store generates row ids.
//! This adapter keeps the mapping so the append-only journal in SQLite is the
//! single source of truth for "what did the agent do".

use std::collections::HashMap;
use std::sync::Mutex;

use agent_runtime::{ConfirmationAnswer, Journal, JournalEvent};
use memory::{JournalEntry, MemoryStore};

pub struct SqliteJournal {
    store: Mutex<MemoryStore>,
    profile_id: String,
    sessions: Mutex<HashMap<String, String>>,
    steps: Mutex<HashMap<(String, u32), String>>,
}

impl SqliteJournal {
    pub fn new(store: MemoryStore, profile_id: &str) -> memory::Result<Self> {
        store.ensure_profile(profile_id, "agent", "Agent")?;
        Ok(Self {
            store: Mutex::new(store),
            profile_id: profile_id.into(),
            sessions: Mutex::default(),
            steps: Mutex::default(),
        })
    }

    pub fn with_store<T>(&self, f: impl FnOnce(&MemoryStore) -> T) -> T {
        f(&self.store.lock().unwrap())
    }

    fn db_session(&self, runtime_id: &str) -> Option<String> {
        self.sessions.lock().unwrap().get(runtime_id).cloned()
    }

    fn step_id(&self, db_session: &str, ordinal: u32) -> memory::Result<String> {
        let key = (db_session.to_string(), ordinal);
        if let Some(id) = self.steps.lock().unwrap().get(&key) {
            return Ok(id.clone());
        }
        // Denied-before-plan-record paths may journal an action without a prior Step event.
        let id = self.store.lock().unwrap().add_step(db_session, ordinal, None, None, None)?;
        self.steps.lock().unwrap().insert(key, id.clone());
        Ok(id)
    }

    fn record_inner(&self, event: JournalEvent) -> memory::Result<()> {
        match event {
            JournalEvent::SessionStarted { session_id, request, scope, dry_run } => {
                let db_id =
                    self.store.lock().unwrap().start_session(&self.profile_id, None, &request, &scope, dry_run)?;
                self.sessions.lock().unwrap().insert(session_id, db_id.clone());
                self.store.lock().unwrap().set_session_status(&db_id, "running", None)?;
            }
            JournalEvent::Step { session_id, ordinal, observation_hash, observation_tokens, thought } => {
                let Some(db) = self.db_session(&session_id) else { return Ok(()) };
                let id = self.store.lock().unwrap().add_step(
                    &db,
                    ordinal,
                    Some(&observation_hash),
                    Some(observation_tokens),
                    thought.as_deref(),
                )?;
                self.steps.lock().unwrap().insert((db, ordinal), id);
            }
            JournalEvent::Action {
                session_id,
                ordinal,
                call,
                decision,
                critic,
                approval,
                executed,
                result,
                error,
                before_hash,
                after_hash,
            } => {
                let Some(db) = self.db_session(&session_id) else { return Ok(()) };
                let step_id = self.step_id(&db, ordinal)?;
                let approval_id = match approval {
                    Some(answer) => {
                        let store = self.store.lock().unwrap();
                        let kind = if decision.consequential { "consequential" } else { decision.rule.as_str() };
                        let id = store.record_approval(&db, kind, &serde_json::to_string(&call)?)?;
                        store.decide_approval(&id, !matches!(answer, ConfirmationAnswer::Rejected))?;
                        Some(id)
                    }
                    None => None,
                };
                let entry = JournalEntry {
                    session_id: db,
                    step_id,
                    call,
                    consequential: decision.consequential,
                    policy_verdict: decision.verdict,
                    policy_rule: decision.rule,
                    critic_verdict: critic,
                    approval_id,
                    executed,
                    result_json: result.map(|v| v.to_string()),
                    error,
                    before_hash: Some(before_hash),
                    after_hash,
                    duration_ms: None,
                };
                self.store.lock().unwrap().journal_action(&entry)?;
            }
            JournalEvent::SessionEnded { session_id, status, outcome } => {
                let Some(db) = self.db_session(&session_id) else { return Ok(()) };
                let db_status = match status.as_str() {
                    "finished" | "dry_run_stopped" => "done",
                    "needs_user" | "human_challenge" => "paused",
                    _ => "cancelled",
                };
                self.store.lock().unwrap().set_session_status(&db, db_status, outcome.as_deref())?;
            }
        }
        Ok(())
    }
}

impl Journal for SqliteJournal {
    fn record(&self, event: JournalEvent) {
        if let Err(e) = self.record_inner(event) {
            // The journal must never take the agent down; surface loudly instead.
            eprintln!("journal write failed: {e}");
        }
    }
}
