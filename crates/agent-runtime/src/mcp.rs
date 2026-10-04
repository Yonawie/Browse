//! MCP (Model Context Protocol) Server for External Agents (ADR-008).
//!
//! Enables external agents (Claude Desktop, Cursor, Claude Code) to act on
//! Browse through the standard MCP JSON-RPC protocol, while enforcing the
//! exact same deterministic security policies (scope, provenance, confirm gates).

use std::sync::Arc;

use core_types::{
    LabeledValue, Origin, Provenance, Sensitivity, TargetInfo, TaskScope, ToolCall, UnixMs, Verdict,
};
use engine_adapter::{Action, EngineAdapter};
use policy::{CheckContext, GrantStore, PolicyConfig, PolicyEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::builtin_manifests;

/// MCP Server instance bridging standard MCP tools to an EngineAdapter and PolicyEngine.
pub struct McpServer<E: EngineAdapter> {
    engine: Arc<E>,
    webview_id: String,
    policy: PolicyEngine,
    scope: TaskScope,
    auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct McpRequest {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct McpResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl<E: EngineAdapter> McpServer<E> {
    pub fn new(
        engine: Arc<E>,
        webview_id: String,
        scope: TaskScope,
        dev_mode: bool,
        auth_token: Option<String>,
    ) -> Self {
        let policy = PolicyEngine::new(PolicyConfig {
            dev_mode_enabled: dev_mode,
            ..PolicyConfig::default()
        });
        Self {
            engine,
            webview_id,
            policy,
            scope,
            auth_token,
        }
    }

    /// Handles a single MCP JSON-RPC request and produces a response.
    pub async fn handle_request(&self, req: McpRequest) -> Option<McpResponse> {
        let method = req.method.as_str();

        // Notifications (e.g. notifications/initialized) return None
        if method == "notifications/initialized" {
            return None;
        }

        let id = req.id.clone();
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "browse-mcp",
                    "version": "0.1.0"
                }
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(&req.params).await,
            _ => Err(json!({
                "code": -32601,
                "message": format!("Method not found: {method}")
            })),
        };

        Some(match result {
            Ok(res) => McpResponse {
                jsonrpc: "2.0",
                id,
                result: Some(res),
                error: None,
            },
            Err(err) => McpResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(err),
            },
        })
    }

    fn tools_list(&self) -> Value {
        json!({
            "tools": [
                {
                    "name": "browse_navigate",
                    "description": "Navigate the browser to a URL inside the allowed scope.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string", "description": "The target HTTPS/HTTP URL to visit." }
                        },
                        "required": ["url"]
                    }
                },
                {
                    "name": "browse_observe",
                    "description": "Capture the current page state, accessibility tree, headings, and interactive elements with numeric refs.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "browse_click",
                    "description": "Click an interactive element on the page using its ref id from observe.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "ref": { "type": "integer", "description": "The element ref ID to click." }
                        },
                        "required": ["ref"]
                    }
                },
                {
                    "name": "browse_type",
                    "description": "Fill a text field using its ref id. Password and masked fields are strictly blocked.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "ref": { "type": "integer", "description": "The element ref ID." },
                            "text": { "type": "string", "description": "Text to type into the field." },
                            "submit": { "type": "boolean", "description": "Press Enter after typing to submit." }
                        },
                        "required": ["ref", "text"]
                    }
                },
                {
                    "name": "browse_extract",
                    "description": "Extract readable text content and chunks from the current page.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        })
    }

    async fn tools_call(&self, params: &Value) -> Result<Value, Value> {
        let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        // Check authentication token if configured
        if let Some(token) = &self.auth_token {
            let req_token = params
                .get("_meta")
                .and_then(|m| m.get("token"))
                .and_then(Value::as_str);
            if req_token != Some(token.as_str()) {
                return Ok(json!({
                    "content": [{ "type": "text", "text": "Unauthorized: Invalid or missing browse-mcp auth token." }],
                    "isError": true
                }));
            }
        }

        let manifests = builtin_manifests();
        let grants = GrantStore::default();

        match tool_name {
            "browse_navigate" => {
                let url = arguments.get("url").and_then(Value::as_str).unwrap_or("");
                let target_origin = Origin::parse(url).ok();

                let call = ToolCall {
                    id: "mcp-call".into(),
                    tool: "navigate".into(),
                    args: [("url".into(), LabeledValue {
                        value: json!(url),
                        provenance: Provenance::User,
                        sensitivity: Sensitivity::Public,
                    })].into_iter().collect(),
                    target: None,
                    target_origin: target_origin.clone(),
                };

                let manifest = manifests.iter().find(|m| m.name == "navigate").unwrap();
                let ctx = CheckContext {
                    scope: &self.scope,
                    manifest,
                    grants: &grants,
                    now: 0 as UnixMs,
                    consequential_so_far: 0,
                    steps_so_far: 0,
                };

                let decision = self.policy.check(&call, &ctx);
                match decision.verdict {
                    Verdict::Deny { reason } => {
                        Ok(json!({
                            "content": [{ "type": "text", "text": format!("Policy Denied: {reason}") }],
                            "isError": true
                        }))
                    }
                    Verdict::Confirm { reason } => {
                        Ok(json!({
                            "content": [{ "type": "text", "text": format!("Confirmation Required: {reason}") }],
                            "isError": true
                        }))
                    }
                    Verdict::Allow => {
                        match self.engine.act(&self.webview_id, Action::Navigate { url: url.to_string() }).await {
                            Ok(event) => {
                                let obs = self.engine.observe(&self.webview_id).await.ok();
                                let title = obs.as_ref().map(|o| o.page.title.as_str()).unwrap_or("");
                                Ok(json!({
                                    "content": [{
                                        "type": "text",
                                        "text": format!("Navigated to {url}. Page Title: \"{title}\". Event: {event:?}")
                                    }],
                                    "isError": false
                                }))
                            }
                            Err(e) => Ok(json!({
                                "content": [{ "type": "text", "text": format!("Engine Error: {e}") }],
                                "isError": true
                            })),
                        }
                    }
                }
            }
            "browse_observe" => {
                match self.engine.observe(&self.webview_id).await {
                    Ok(obs) => {
                        let elements_summary: Vec<Value> = obs.interactive.iter().map(|el| {
                            json!({
                                "ref": el.element_ref.id,
                                "role": el.role,
                                "name": el.name,
                                "type": el.input_type
                            })
                        }).collect();

                        let result_text = format!(
                            "Page: \"{}\"\nURL: {}\nInteractive Elements:\n{}",
                            obs.page.title,
                            obs.page.url,
                            serde_json::to_string_pretty(&elements_summary).unwrap_or_default()
                        );

                        Ok(json!({
                            "content": [{ "type": "text", "text": result_text }],
                            "isError": false
                        }))
                    }
                    Err(e) => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Observe Error: {e}") }],
                        "isError": true
                    })),
                }
            }
            "browse_click" => {
                let ref_id = arguments.get("ref").and_then(Value::as_u64).unwrap_or(0);
                let obs = match self.engine.observe(&self.webview_id).await {
                    Ok(o) => o,
                    Err(e) => return Ok(json!({
                        "content": [{ "type": "text", "text": format!("Observe failed: {e}") }],
                        "isError": true
                    })),
                };

                let target_el = obs.find_ref(ref_id);
                if target_el.is_none() {
                    return Ok(json!({
                        "content": [{ "type": "text", "text": format!("Ref {ref_id} not found in current observation") }],
                        "isError": true
                    }));
                }
                let el = target_el.unwrap();

                let call = ToolCall {
                    id: "mcp-click".into(),
                    tool: "click".into(),
                    args: [("ref".into(), LabeledValue {
                        value: json!(ref_id),
                        provenance: Provenance::User,
                        sensitivity: Sensitivity::Public,
                    })].into_iter().collect(),
                    target: Some(TargetInfo {
                        role: el.role.clone(),
                        name: el.name.clone(),
                        input_type: el.input_type.clone(),
                        is_submit: el.role == "button" && el.input_type.as_deref() == Some("submit"),
                        form_has_payment_fields: false,
                        form_has_file_upload: false,
                        href: el.href.clone(),
                        site_tool_consequential_hint: false,
                    }),
                    target_origin: Some(obs.page.origin.clone()),
                };

                let manifest = manifests.iter().find(|m| m.name == "click").unwrap();
                let ctx = CheckContext {
                    scope: &self.scope,
                    manifest,
                    grants: &grants,
                    now: 0 as UnixMs,
                    consequential_so_far: 0,
                    steps_so_far: 0,
                };

                let decision = self.policy.check(&call, &ctx);
                match decision.verdict {
                    Verdict::Deny { reason } => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Policy Denied: {reason}") }],
                        "isError": true
                    })),
                    Verdict::Confirm { reason } => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Confirmation Required: {reason}") }],
                        "isError": true
                    })),
                    Verdict::Allow => {
                        match self.engine.act(&self.webview_id, Action::Click { target: el.element_ref.clone() }).await {
                            Ok(event) => Ok(json!({
                                "content": [{ "type": "text", "text": format!("Clicked ref {ref_id} ({:?})", event) }],
                                "isError": false
                            })),
                            Err(e) => Ok(json!({
                                "content": [{ "type": "text", "text": format!("Click failed: {e}") }],
                                "isError": true
                            })),
                        }
                    }
                }
            }
            "browse_type" => {
                let ref_id = arguments.get("ref").and_then(Value::as_u64).unwrap_or(0);
                let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
                let submit = arguments.get("submit").and_then(Value::as_bool).unwrap_or(false);

                let obs = match self.engine.observe(&self.webview_id).await {
                    Ok(o) => o,
                    Err(e) => return Ok(json!({
                        "content": [{ "type": "text", "text": format!("Observe failed: {e}") }],
                        "isError": true
                    })),
                };

                let target_el = obs.find_ref(ref_id);
                if target_el.is_none() {
                    return Ok(json!({
                        "content": [{ "type": "text", "text": format!("Ref {ref_id} not found in current observation") }],
                        "isError": true
                    }));
                }
                let el = target_el.unwrap();

                // Masked fields check
                if let Some(itype) = &el.input_type {
                    if itype == "password" || itype.starts_with("cc-") {
                        return Ok(json!({
                            "content": [{ "type": "text", "text": "Policy Denied: Cannot type into password or credit card fields." }],
                            "isError": true
                        }));
                    }
                }

                match self.engine.act(&self.webview_id, Action::Type {
                    target: el.element_ref.clone(),
                    text: text.to_string(),
                    submit,
                }).await {
                    Ok(event) => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Typed into ref {ref_id} ({:?})", event) }],
                        "isError": false
                    })),
                    Err(e) => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Type failed: {e}") }],
                        "isError": true
                    })),
                }
            }
            "browse_extract" => {
                match self.engine.observe(&self.webview_id).await {
                    Ok(obs) => {
                        let text_content = obs.content.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n\n");
                        Ok(json!({
                            "content": [{ "type": "text", "text": text_content }],
                            "isError": false
                        }))
                    }
                    Err(e) => Ok(json!({
                        "content": [{ "type": "text", "text": format!("Extract failed: {e}") }],
                        "isError": true
                    })),
                }
            }
            _ => Err(json!({
                "code": -32601,
                "message": format!("Unknown tool: {tool_name}")
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_adapter::mock::{MockEngine, MockPage};
    use engine_adapter::WebViewOptions;

    async fn make_server() -> McpServer<MockEngine> {
        let mock = MockEngine::new();
        let page = MockPage::simple("https://shop.example/catalog", "Demo Catalog", "Welcome to Shop")
            .with_element(1, "button", "Buy");
        mock.add_page(page);

        let webview_id = mock.create_webview(WebViewOptions::agent("mcp-test")).await.unwrap();

        let scope = TaskScope::new(
            vec!["https://shop.example"],
            vec!["navigate", "click", "type", "extract"],
        );

        McpServer::new(Arc::new(mock), webview_id, scope, false, Some("secret-token".into()))
    }

    #[tokio::test]
    async fn mcp_initialize_and_ping() {
        let server = make_server().await;
        let init_req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(1)),
            method: "initialize".into(),
            params: json!({}),
        };
        let resp = server.handle_request(init_req).await.unwrap();
        assert_eq!(resp.result.unwrap()["serverInfo"]["name"], "browse-mcp");

        let ping_req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(2)),
            method: "ping".into(),
            params: json!({}),
        };
        let ping_resp = server.handle_request(ping_req).await.unwrap();
        assert!(ping_resp.result.is_some());
    }

    #[tokio::test]
    async fn mcp_tools_list() {
        let server = make_server().await;
        let req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(3)),
            method: "tools/list".into(),
            params: json!({}),
        };
        let resp = server.handle_request(req).await.unwrap();
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        assert!(tools.iter().any(|t| t["name"] == "browse_navigate"));
        assert!(tools.iter().any(|t| t["name"] == "browse_observe"));
        assert!(tools.iter().any(|t| t["name"] == "browse_click"));
    }

    #[tokio::test]
    async fn mcp_auth_token_enforcement() {
        let server = make_server().await;

        // Unauthorized call (missing token)
        let unauth_req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(4)),
            method: "tools/call".into(),
            params: json!({
                "name": "browse_navigate",
                "arguments": { "url": "https://shop.example/catalog" }
            }),
        };
        let resp = server.handle_request(unauth_req).await.unwrap();
        assert_eq!(resp.result.unwrap()["isError"], true);

        // Authorized call (matching token)
        let auth_req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(5)),
            method: "tools/call".into(),
            params: json!({
                "name": "browse_navigate",
                "arguments": { "url": "https://shop.example/catalog" },
                "_meta": { "token": "secret-token" }
            }),
        };
        let auth_resp = server.handle_request(auth_req).await.unwrap();
        assert_eq!(auth_resp.result.unwrap()["isError"], false);
    }

    #[tokio::test]
    async fn mcp_policy_scope_denial() {
        let server = make_server().await;

        // Attempt navigation to evil.example (out-of-scope)
        let evil_req = McpRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(6)),
            method: "tools/call".into(),
            params: json!({
                "name": "browse_navigate",
                "arguments": { "url": "https://evil.example/phish" },
                "_meta": { "token": "secret-token" }
            }),
        };
        let resp = server.handle_request(evil_req).await.unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Confirmation Required") || text.contains("Policy Denied"));
    }
}
