//! `browse-desktop mcp` — Stdio Model Context Protocol Server (ADR-008).
//!
//! Exposes Browse browser tools to external agents (Claude Desktop, Cursor, etc.)
//! via newline-delimited JSON-RPC 2.0 over standard I/O.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use agent_runtime::mcp::{McpRequest, McpServer};
use core_types::TaskScope;
use engine_adapter::mock::{MockEngine, MockPage};
use engine_adapter::{EngineAdapter, WebViewOptions};

pub async fn mcp_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut dev_mode = false;
    let mut auth_token = None;
    let mut allowed_origins = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dev-mode" => {
                dev_mode = true;
                i += 1;
            }
            "--auth-token" => {
                if i + 1 < args.len() {
                    auth_token = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("Missing argument for --auth-token".into());
                }
            }
            "--origin" => {
                if i + 1 < args.len() {
                    allowed_origins.push(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("Missing argument for --origin".into());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    if allowed_origins.is_empty() {
        allowed_origins.push("https://*".into());
        allowed_origins.push("http://*".into());
    }

    let mock_engine = Arc::new(MockEngine::new());
    // Pre-populate sample page for mock demonstration
    mock_engine.add_page(
        MockPage::simple(
            "https://browse.local",
            "Browse Local Start",
            "Welcome to Browse Agent Playground. You are connected via MCP.",
        )
        .with_element(1, "button", "Learn More"),
    );

    let webview_id = mock_engine
        .create_webview(WebViewOptions::agent("mcp-external"))
        .await
        .map_err(|e| format!("Failed to create webview: {e}"))?;

    let scope = TaskScope::new(
        allowed_origins.iter().map(String::as_str),
        vec!["navigate", "click", "type", "extract", "scroll", "evaluate"],
    );

    let server = McpServer::new(mock_engine, webview_id, scope, dev_mode, auth_token);

    eprintln!("[browse-mcp] Server started (stdio JSON-RPC). Ready for external agent connections.");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<McpRequest>(trimmed) {
            Ok(req) => {
                if let Some(resp) = server.handle_request(req).await {
                    let json_str = serde_json::to_string(&resp)?;
                    writeln!(stdout_lock, "{json_str}")?;
                    stdout_lock.flush()?;
                }
            }
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {e}")
                    }
                });
                let json_str = serde_json::to_string(&err_resp)?;
                writeln!(stdout_lock, "{json_str}")?;
                stdout_lock.flush()?;
            }
        }
    }

    eprintln!("[browse-mcp] Server shutting down (stdin closed).");
    Ok(())
}
