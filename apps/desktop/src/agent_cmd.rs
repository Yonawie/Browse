//! Agent execution CLI (S6): runs the Agent Runtime against a real browser or mock,
//! enforcing deterministic policy rules, confirmations, blind critic, and SQLite journaling.

use std::sync::Arc;

use agent_runtime::{
    AgentRunner, ConfirmationAnswer, ConfirmationHandler, ConfirmationRequest, ExecutionMode, ModelCritic,
    ModelPlanner, StepOutcome, ToolRegistry,
};
use core_types::{Origin, Sensitivity, TaskScope};
use engine_adapter::EngineAdapter;
use engine_cdp::launcher::LaunchOptions;
use engine_cdp::CdpEngine;
use memory::MemoryStore;
use policy::{PolicyConfig, PolicyEngine};

use crate::journal::SqliteJournal;
use crate::memory_cmd::database_path;
use crate::models;

pub struct StdinConfirmations;

#[async_trait::async_trait]
impl ConfirmationHandler for StdinConfirmations {
    async fn confirm(&self, request: &ConfirmationRequest) -> Result<ConfirmationAnswer, agent_runtime::AgentError> {
        println!("\n  ┌── CONFIRMATION REQUIRED ({})", request.rule);
        println!("  │ Action: {}", request.preview);
        println!("  │ Reason: {}", request.reason);
        for detail in &request.details {
            println!("  │ Detail: {detail}");
        }
        print!("  └── Approve this action? [y/N/always]: ");
        use std::io::Write;
        std::io::stdout().flush().ok();

        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let input = line.trim().to_lowercase();
        if input == "y" || input == "yes" {
            Ok(ConfirmationAnswer::Approved)
        } else if input == "always" {
            Ok(ConfirmationAnswer::ApprovedForSession)
        } else {
            Ok(ConfirmationAnswer::Rejected)
        }
    }
}

pub struct AutoRejectConfirmations;

#[async_trait::async_trait]
impl ConfirmationHandler for AutoRejectConfirmations {
    async fn confirm(&self, request: &ConfirmationRequest) -> Result<ConfirmationAnswer, agent_runtime::AgentError> {
        eprintln!(
            "  [confirmation required but running non-interactively; auto-rejecting: {} - {}]",
            request.rule, request.preview
        );
        Ok(ConfirmationAnswer::Rejected)
    }
}

pub async fn run_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (url, task, dry_run, interactive) = parse_run_args(args)?;

    let target_origin = Origin::parse(&url).map_err(|e| format!("invalid URL {url}: {e}"))?;
    let scope = TaskScope::new([target_origin.as_str()], ["navigate", "click", "type", "extract"]);

    println!("Browse Agent starting...");
    println!("  Target: {url}");
    println!("  Task:   {task}");
    println!("  Mode:   {}", if dry_run { "DryRun (stops before consequential actions)" } else { "Live" });
    println!("  Input:  {}", if interactive { "Interactive (confirmations on stdin)" } else { "Auto-reject confirmations" });

    // Connect to persistent SQLite database
    let db_path = database_path();
    let store = MemoryStore::open(&db_path)?;
    let journal = Arc::new(SqliteJournal::new(store, "agent")?);

    // Check configured models
    let configured = models::from_env(None).await;
    for line in &configured.summary {
        eprintln!("  model: {line}");
    }

    // Launch CDP browser engine
    eprintln!("launching headless browser...");
    let engine: Arc<dyn EngineAdapter> = CdpEngine::launch(LaunchOptions::headless()).await?;

    // Build planner and critic
    let router_arc = Arc::new(configured.gateway.router().clone());
    let planner: Arc<dyn agent_runtime::Planner> = Arc::new(ModelPlanner::new(router_arc.clone()));
    let critic: Arc<dyn agent_runtime::Critic> = Arc::new(ModelCritic::new(router_arc.clone(), core_types::ModelTier::Fast));

    let confirmer: Arc<dyn ConfirmationHandler> = if interactive {
        Arc::new(StdinConfirmations)
    } else {
        Arc::new(AutoRejectConfirmations)
    };

    let policy = PolicyEngine::new(PolicyConfig {
        agent_enabled: true,
        ..Default::default()
    });

    let runner = AgentRunner::new(
        engine.clone(),
        policy,
        ToolRegistry::builtin(),
        planner,
        critic,
        confirmer,
        journal.clone(),
    );

    let mode = if dry_run { ExecutionMode::DryRun } else { ExecutionMode::Live };
    let mut session = runner
        .start(&task, Sensitivity::Personal, scope, &url, mode)
        .await?;

    println!("\nAgent session started [{}]", session.id);

    let mut step_count = 0;
    loop {
        step_count += 1;
        print!("\nStep {step_count}: ");
        use std::io::Write;
        std::io::stdout().flush().ok();

        let outcome = runner.step(&mut session).await?;
        match &outcome {
            StepOutcome::Acted { call, result } => {
                println!("ACTED: {} (ok={})", call.tool, result.ok);
            }
            StepOutcome::Denied { call, reason, rule } => {
                println!("DENIED: {} [{rule}] {reason}", call.tool);
            }
            StepOutcome::Rejected { call } => {
                println!("REJECTED: {} (confirmation declined)", call.tool);
            }
            StepOutcome::DryRunStopped { call, reason } => {
                println!("DRY-RUN STOPPED: {} (would require confirmation: {reason})", call.tool);
                break;
            }
            StepOutcome::Finished { summary } => {
                println!("FINISHED: {summary}");
                break;
            }
            StepOutcome::NeedsUser { question } => {
                println!("NEEDS USER INPUT: {question}");
                break;
            }
            StepOutcome::HumanChallenge => {
                println!("CAPTCHA/HUMAN CHALLENGE DETECTED: handing over to human");
                break;
            }
            StepOutcome::LimitReached { what } => {
                println!("LIMIT REACHED: {what}");
                break;
            }
        }

        if outcome.is_terminal() {
            break;
        }
    }

    for sidecar in configured.sidecars {
        sidecar.stop().await;
    }

    println!("\nSession ended. Audit trail saved to {}.", db_path.display());
    Ok(())
}

fn parse_run_args(args: &[String]) -> Result<(String, String, bool, bool), Box<dyn std::error::Error>> {
    let usage = "usage: browse-desktop run <url> <task> [--dry-run] [--interactive]";
    let mut url = None;
    let mut task_words = Vec::new();
    let mut dry_run = false;
    let mut interactive = false;

    for arg in args {
        if arg == "--dry-run" {
            dry_run = true;
        } else if arg == "--interactive" {
            interactive = true;
        } else if url.is_none() && (arg.starts_with("http://") || arg.starts_with("https://")) {
            url = Some(arg.clone());
        } else {
            task_words.push(arg.clone());
        }
    }

    let url = url.ok_or_else(|| usage.to_string())?;
    let task = task_words.join(" ").trim().to_string();
    if task.is_empty() {
        return Err("task description cannot be empty".into());
    }

    Ok((url, task, dry_run, interactive))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_arguments() {
        let args = vec![
            "https://shop.example".into(),
            "find".into(),
            "shoes".into(),
            "--dry-run".into(),
            "--interactive".into(),
        ];
        let (url, task, dry_run, interactive) = parse_run_args(&args).unwrap();
        assert_eq!(url, "https://shop.example");
        assert_eq!(task, "find shoes");
        assert!(dry_run);
        assert!(interactive);
    }
}
