//! Runtime ↔ Shell boundaries: confirmations and the journal sink.

use std::sync::Mutex;

use async_trait::async_trait;
use core_types::{TaskScope, ToolCall, Verdict};
use policy::Decision;
use serde::{Deserialize, Serialize};

use crate::AgentError;

/// What the user sees before an action that needs confirmation. Built from
/// deterministic data (target snapshot, provenance), not from model prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationRequest {
    pub session_id: String,
    pub call: ToolCall,
    pub reason: String,
    pub rule: String,
    pub consequential: bool,
    /// Human-readable preview: "Click button “Place order” on https://shop.example".
    pub preview: String,
    /// Data-flow details from PolicyEngine (e.g. "`text` was read on https://a.example").
    pub details: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAnswer {
    Approved,
    /// Approve and remember for the rest of the session (creates a grant).
    ApprovedForSession,
    Rejected,
}

#[async_trait]
pub trait ConfirmationHandler: Send + Sync {
    async fn confirm(&self, request: &ConfirmationRequest) -> Result<ConfirmationAnswer, AgentError>;
}

/// Scripted handler for tests and headless runs.
pub struct ScriptedConfirmations {
    answers: Mutex<Vec<ConfirmationAnswer>>,
    pub seen: Mutex<Vec<ConfirmationRequest>>,
}

impl ScriptedConfirmations {
    pub fn new(mut answers: Vec<ConfirmationAnswer>) -> Self {
        answers.reverse();
        Self { answers: Mutex::new(answers), seen: Mutex::new(vec![]) }
    }

    pub fn reject_all() -> Self {
        Self::new(vec![])
    }
}

#[async_trait]
impl ConfirmationHandler for ScriptedConfirmations {
    async fn confirm(&self, request: &ConfirmationRequest) -> Result<ConfirmationAnswer, AgentError> {
        self.seen.lock().unwrap().push(request.clone());
        Ok(self.answers.lock().unwrap().pop().unwrap_or(ConfirmationAnswer::Rejected))
    }
}

/// Events the runtime writes to the append-only journal (`agent_sessions`,
/// `agent_steps`, `agent_actions`, `approvals`, `policy_decisions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JournalEvent {
    SessionStarted { session_id: String, request: String, scope: TaskScope, dry_run: bool },
    Step { session_id: String, ordinal: u32, observation_hash: String, observation_tokens: u32, thought: Option<String> },
    Action {
        session_id: String,
        ordinal: u32,
        call: ToolCall,
        decision: Decision,
        critic: Option<Verdict>,
        approval: Option<ConfirmationAnswer>,
        executed: bool,
        result: Option<serde_json::Value>,
        error: Option<String>,
        before_hash: String,
        after_hash: Option<String>,
    },
    SessionEnded { session_id: String, status: String, outcome: Option<String> },
}

pub trait Journal: Send + Sync {
    fn record(&self, event: JournalEvent);
}

#[derive(Default)]
pub struct InMemoryJournal {
    pub events: Mutex<Vec<JournalEvent>>,
}

impl InMemoryJournal {
    pub fn actions(&self) -> Vec<JournalEvent> {
        self.events.lock().unwrap().iter().filter(|e| matches!(e, JournalEvent::Action { .. })).cloned().collect()
    }
}

impl Journal for InMemoryJournal {
    fn record(&self, event: JournalEvent) {
        self.events.lock().unwrap().push(event);
    }
}
