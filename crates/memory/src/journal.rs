//! Agent journal: sessions, steps and the append-only action log.

use crate::store::MemoryStore;
use crate::{new_id, now_ms, Result};
use core_types::{TaskScope, ToolCall, Verdict};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub session_id: String,
    pub step_id: String,
    pub call: ToolCall,
    pub consequential: bool,
    pub policy_verdict: Verdict,
    pub policy_rule: String,
    pub critic_verdict: Option<Verdict>,
    pub approval_id: Option<String>,
    pub executed: bool,
    pub result_json: Option<String>,
    pub error: Option<String>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub duration_ms: Option<i64>,
}

fn verdict_str(v: &Verdict) -> &'static str {
    match v {
        Verdict::Allow => "allow",
        Verdict::Confirm { .. } => "confirm",
        Verdict::Deny { .. } => "deny",
    }
}

fn verdict_reason(v: &Verdict) -> Option<&str> {
    match v {
        Verdict::Allow => None,
        Verdict::Confirm { reason } | Verdict::Deny { reason } => Some(reason),
    }
}

impl MemoryStore {
    pub fn start_session(
        &self,
        profile_id: &str,
        task_id: Option<&str>,
        request: &str,
        scope: &TaskScope,
        dry_run: bool,
    ) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO agent_sessions(id, task_id, profile_id, request, scope_json, mode, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'planning', ?7)",
            params![
                id,
                task_id,
                profile_id,
                request,
                serde_json::to_string(scope)?,
                if dry_run { "dry_run" } else { "live" },
                now_ms()
            ],
        )?;
        Ok(id)
    }

    pub fn set_session_status(&self, session_id: &str, status: &str, outcome: Option<&str>) -> Result<()> {
        let ended = matches!(status, "done" | "failed" | "cancelled").then(now_ms);
        self.conn().execute(
            "UPDATE agent_sessions SET status = ?2, outcome = COALESCE(?3, outcome), ended_at = COALESCE(?4, ended_at) WHERE id = ?1",
            params![session_id, status, outcome, ended],
        )?;
        Ok(())
    }

    pub fn add_step(
        &self,
        session_id: &str,
        ordinal: u32,
        observation_hash: Option<&str>,
        observation_tokens: Option<u32>,
        thought: Option<&str>,
    ) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO agent_steps(id, session_id, ordinal, observation_hash, observation_tokens, thought, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, session_id, ordinal, observation_hash, observation_tokens, thought, now_ms()],
        )?;
        Ok(id)
    }

    pub fn journal_action(&self, e: &JournalEntry) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO agent_actions(id, step_id, tool, args_json, target_origin, consequential, policy_verdict, policy_reason, critic_verdict, critic_reason, approval_id, executed, result_json, error, before_hash, after_hash, created_at, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                id,
                e.step_id,
                e.call.tool,
                serde_json::to_string(&e.call.args)?,
                e.call.target_origin.as_ref().map(|o| o.to_string()),
                e.consequential as i64,
                verdict_str(&e.policy_verdict),
                verdict_reason(&e.policy_verdict),
                e.critic_verdict.as_ref().map(verdict_str),
                e.critic_verdict.as_ref().and_then(verdict_reason),
                e.approval_id,
                e.executed as i64,
                e.result_json,
                e.error,
                e.before_hash,
                e.after_hash,
                now_ms(),
                e.duration_ms,
            ],
        )?;
        self.conn().execute(
            "INSERT INTO policy_decisions(id, session_id, action_id, rule, verdict, details_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            params![new_id(), e.session_id, id, e.policy_rule, verdict_str(&e.policy_verdict), now_ms()],
        )?;
        Ok(id)
    }

    pub fn record_approval(&self, session_id: &str, kind: &str, preview_json: &str) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO approvals(id, session_id, kind, preview_json, requested_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, session_id, kind, preview_json, now_ms()],
        )?;
        Ok(id)
    }

    pub fn decide_approval(&self, approval_id: &str, approved: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE approvals SET decision = ?2, decided_at = ?3 WHERE id = ?1",
            params![approval_id, if approved { "approved" } else { "rejected" }, now_ms()],
        )?;
        Ok(())
    }

    /// Local audit of model usage (`model_calls`). Never contains prompt text.
    pub fn record_model_call(&self, call: &ModelCallRecord) -> Result<String> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO model_calls(id, session_id, purpose, provider, model, locality, sensitivity,
                                     input_tokens, output_tokens, latency_ms, cost_usd, cache_hit, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                call.session_id,
                call.purpose,
                call.provider,
                call.model,
                call.locality,
                call.sensitivity,
                call.input_tokens,
                call.output_tokens,
                call.latency_ms,
                call.cost_usd,
                call.cache_hit as i64,
                call.created_at.unwrap_or_else(now_ms),
            ],
        )?;
        Ok(id)
    }

    /// Aggregate view for the settings UI: calls, tokens and cache hits per (provider, model, locality).
    pub fn model_usage(&self) -> Result<Vec<ModelUsageRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT provider, model, locality, COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    SUM(cache_hit), COALESCE(SUM(cost_usd),0)
             FROM model_calls GROUP BY provider, model, locality ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ModelUsageRow {
                    provider: r.get(0)?,
                    model: r.get(1)?,
                    locality: r.get(2)?,
                    calls: r.get(3)?,
                    input_tokens: r.get(4)?,
                    output_tokens: r.get(5)?,
                    cache_hits: r.get(6)?,
                    cost_usd: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCallRecord {
    pub session_id: Option<String>,
    pub purpose: String,
    pub provider: String,
    pub model: String,
    /// `local` | `cloud`
    pub locality: String,
    /// `public` | `personal` | `private`
    pub sensitivity: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub cost_usd: Option<f64>,
    pub cache_hit: bool,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageRow {
    pub provider: String,
    pub model: String,
    pub locality: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_hits: i64,
    pub cost_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{LabeledValue, Origin};

    #[test]
    fn journal_is_append_only() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.ensure_profile("agent", "agent", "agent").unwrap();
        let scope = TaskScope::new(["https://a.example"], ["click"]);
        let sid = s.start_session("agent", None, "do it", &scope, false).unwrap();
        let step = s.add_step(&sid, 0, Some("h"), Some(1200), None).unwrap();
        let call = ToolCall::new("c1", "click")
            .on(Origin::parse("https://a.example").unwrap())
            .arg("ref", LabeledValue::user(1));
        let entry = JournalEntry {
            session_id: sid.clone(),
            step_id: step,
            call,
            consequential: false,
            policy_verdict: Verdict::Allow,
            policy_rule: "allow".into(),
            critic_verdict: Some(Verdict::Allow),
            approval_id: None,
            executed: true,
            result_json: None,
            error: None,
            before_hash: None,
            after_hash: None,
            duration_ms: Some(12),
        };
        let aid = s.journal_action(&entry).unwrap();
        let err = s.conn().execute("UPDATE agent_actions SET tool = 'x' WHERE id = ?1", params![aid]).unwrap_err();
        assert!(err.to_string().contains("append-only"));
        let err = s.conn().execute("DELETE FROM agent_actions WHERE id = ?1", params![aid]).unwrap_err();
        assert!(err.to_string().contains("append-only"));
        assert_eq!(s.count("agent_actions").unwrap(), 1);
        s.set_session_status(&sid, "done", Some("ok")).unwrap();
    }

    #[test]
    fn model_calls_are_recorded_and_aggregated() {
        let s = MemoryStore::open_in_memory().unwrap();
        let rec = |cache_hit: bool| ModelCallRecord {
            session_id: None,
            purpose: "summarize".into(),
            provider: "llama-server".into(),
            model: "qwen".into(),
            locality: "local".into(),
            sensitivity: "private".into(),
            input_tokens: Some(100),
            output_tokens: Some(20),
            latency_ms: Some(300),
            cost_usd: None,
            cache_hit,
            created_at: None,
        };
        s.record_model_call(&rec(false)).unwrap();
        s.record_model_call(&rec(true)).unwrap();
        let usage = s.model_usage().unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].calls, 2);
        assert_eq!(usage[0].cache_hits, 1);
        assert_eq!(usage[0].input_tokens, 200);

        let mut bad = rec(false);
        bad.locality = "moon".into();
        assert!(s.record_model_call(&bad).is_err(), "CHECK constraint on locality");
    }
}
