//! Browse Shell Server (S7): serves the WebUI and handles JSON-RPC 2.0 / SSE IPC
//! bridging the graphical shell to Engine, Page Intelligence, Memory and Agent layers.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::header;
use axum::response::{sse::Event, sse::Sse, Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use core_types::Sensitivity;
use futures_util::stream::Stream;
use memory::{auto_cluster_tab, MemoryStore, SearchFilters};
use page_intelligence::{detect_dark_patterns, inspect_url_phishing};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::memory_cmd::database_path;

const INDEX_HTML: &str = include_str!("../ui/index.html");
const STYLE_CSS: &str = include_str!("../ui/style.css");
const APP_JS: &str = include_str!("../ui/app.js");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub profile: String,
    pub active: bool,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Clone)]
pub struct ShellServerState {
    pub tabs: Arc<Mutex<Vec<TabInfo>>>,
    pub store: Arc<Mutex<MemoryStore>>,
}

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

pub fn create_router(state: ShellServerState) -> axum::Router {
    axum::Router::new()
        .route("/", get(serve_index))
        .route("/style.css", get(serve_css))
        .route("/app.js", get(serve_js))
        .route("/api/rpc", post(handle_rpc))
        .route("/api/events", get(handle_events))
        .with_state(state)
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn serve_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE_CSS)
}

async fn serve_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")], APP_JS)
}

async fn handle_events() -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream = futures_util::stream::unfold(tokio::time::interval(Duration::from_secs(15)), |mut interval| async move {
        interval.tick().await;
        Some((Ok(Event::default().event("ping").data("{\"type\":\"ping\"}")), interval))
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

pub async fn dispatch_rpc(
    state: ShellServerState,
    req: RpcRequest,
) -> Result<Value, Value> {
    let method = req.method.as_str();
    match method {
        "tabs.list" => {
            let tabs = state.tabs.lock().unwrap().clone();
            Ok(json!(tabs))
        }
        "tabs.create" | "tabs.new" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("https://example.com");
            let profile = req.params.get("profile").and_then(Value::as_str).unwrap_or("User");
            let group = req.params.get("group")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| Some(auto_cluster_tab(url).to_string()));
            let mut tabs = state.tabs.lock().unwrap();
            for t in tabs.iter_mut() {
                t.active = false;
            }
            let new_tab = TabInfo {
                id: format!("tab-{}", tabs.len() + 1),
                url: url.to_string(),
                title: "New Tab".to_string(),
                profile: profile.to_string(),
                active: true,
                group,
            };
            tabs.push(new_tab.clone());
            Ok(json!(new_tab))
        }
        "tabs.navigate" => {
            let tab_id = req.params.get("tab_id").and_then(Value::as_str);
            let url = req.params.get("url").and_then(Value::as_str);
            if let (Some(id), Some(url)) = (tab_id, url) {
                let mut tabs = state.tabs.lock().unwrap();
                if let Some(tab) = tabs.iter_mut().find(|t| t.id == id) {
                    tab.url = url.to_string();
                    tab.title = url.to_string();
                    tab.group = Some(auto_cluster_tab(url).to_string());
                }
            }
            Ok(json!({ "status": "ok" }))
        }
        "tabs.close" => {
            let tab_id = req.params.get("tab_id").and_then(Value::as_str);
            if let Some(id) = tab_id {
                let mut tabs = state.tabs.lock().unwrap();
                tabs.retain(|t| t.id != id);
                if !tabs.is_empty() && !tabs.iter().any(|t| t.active) {
                    tabs[0].active = true;
                }
            }
            Ok(json!({ "status": "ok" }))
        }
        "page.analyze_safety" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("");
            let text = req.params.get("text").and_then(Value::as_str).unwrap_or("");
            let phishing = inspect_url_phishing(url);
            let dark_patterns = detect_dark_patterns(text);
            Ok(json!({
                "phishing": phishing,
                "dark_patterns": dark_patterns,
            }))
        }
        "tabs.groups.list" => {
            let lock = state.store.lock().unwrap();
            let groups = lock.list_tab_groups().unwrap_or_default();
            Ok(json!(groups))
        }
        "tabs.groups.create" => {
            let title = req.params.get("title").and_then(Value::as_str).unwrap_or("New Group");
            let task_id = req.params.get("task_id").and_then(Value::as_str);
            let auto = req.params.get("auto").and_then(Value::as_bool).unwrap_or(false);
            let lock = state.store.lock().unwrap();
            let group_id = lock.create_tab_group(task_id, title, auto).map_err(|e| json!({"error": e.to_string()}))?;
            Ok(json!({ "id": group_id, "title": title }))
        }
        "tasks.list" => {
            let lock = state.store.lock().unwrap();
            let tasks = lock.list_tasks().unwrap_or_default();
            Ok(json!(tasks))
        }
        "tasks.create" => {
            let title = req.params.get("title").and_then(Value::as_str).unwrap_or("New Task");
            let lock = state.store.lock().unwrap();
            let task_id = lock.create_task(title).map_err(|e| json!({"error": e.to_string()}))?;
            Ok(json!({ "id": task_id, "title": title }))
        }
        "memory.stats" => {
            let lock = state.store.lock().unwrap();
            let pages = lock.count("pages").unwrap_or(0);
            let versions = lock.count("page_versions").unwrap_or(0);
            let chunks = lock.count("chunks").unwrap_or(0);
            let vectors = lock.count("chunk_vectors_raw").unwrap_or(0);
            Ok(json!({
                "pages": pages,
                "page_versions": versions,
                "chunks": chunks,
                "vectors": vectors,
            }))
        }
        "memory.search" => {
            let query = req.params.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
            let filters = SearchFilters {
                domain: None,
                since_ms: None,
                until_ms: None,
                max_sensitivity: Some(Sensitivity::Personal),
            };
            let lock = state.store.lock().unwrap();
            let hits = lock.hybrid_search(query, None, &filters, limit).unwrap_or_default();
            let hits_json: Vec<Value> = hits
                .iter()
                .map(|h| {
                    json!({
                        "chunk_id": h.chunk_id,
                        "url": h.url,
                        "title": h.title,
                        "heading_path": h.heading_path,
                        "text": h.text,
                        "score": h.score
                    })
                })
                .collect();
            Ok(json!(hits_json))
        }
        "memory.entities" => {
            let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let lock = state.store.lock().unwrap();
            let entities = lock.list_top_entities(limit).unwrap_or_default();
            let entities_json: Vec<Value> = entities
                .into_iter()
                .map(|(id, kind, name, mentions)| {
                    json!({
                        "id": id,
                        "kind": kind,
                        "name": name,
                        "mentions": mentions
                    })
                })
                .collect();
            Ok(json!(entities_json))
        }
        "skills.list" => {
            let skills = core_types::builtin_skills();
            Ok(json!(skills))
        }
        "skills.render" => {
            let skill_id = req.params.get("id").and_then(Value::as_str).unwrap_or("");
            let params_map: std::collections::HashMap<String, String> = req
                .params
                .get("parameters")
                .and_then(Value::as_object)
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            let skills = core_types::builtin_skills();
            if let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) {
                let prompt = skill.render_prompt(&params_map);
                Ok(json!({
                    "id": skill.id,
                    "prompt": prompt,
                    "scope": skill.scope
                }))
            } else {
                Err(json!({ "code": -32602, "message": format!("Skill not found: {skill_id}") }))
            }
        }
        "system.status" => {
            Ok(json!({
                "engine": "cdp-chromium",
                "security": "architectural-provenance",
                "telemetry": false,
                "status": "ready"
            }))
        }
        "omnibox.classify" => {
            let input = req.params.get("input").and_then(Value::as_str).unwrap_or("");
            let classification = core_types::classify_omnibox_input(input);
            Ok(json!(classification))
        }
        _ => Err(json!({ "code": -32601, "message": format!("Method not found: {method}") })),
    }
}

async fn handle_rpc(
    State(state): State<ShellServerState>,
    Json(req): Json<RpcRequest>,
) -> Response {
    let req_id = req.id.clone();
    let result = dispatch_rpc(state, req).await;

    let resp = match result {
        Ok(res) => RpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: Some(res),
            error: None,
        },
        Err(err) => RpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: None,
            error: Some(err),
        },
    };

    Json(resp).into_response()
}

pub async fn shell_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut port = 3000u16;
    let mut open_browser = true;

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--port" && i + 1 < args.len() {
            port = args[i + 1].parse().unwrap_or(3000);
            i += 2;
        } else if args[i] == "--no-open" || args[i] == "--headless" {
            open_browser = false;
            i += 1;
        } else {
            i += 1;
        }
    }

    let db_path = database_path();
    let store = Arc::new(Mutex::new(MemoryStore::open(&db_path)?));
    store.lock().unwrap().ensure_profile("user", "user", "Default")?;

    let default_tabs = vec![
        TabInfo {
            id: "tab-1".to_string(),
            url: "https://example.com".to_string(),
            title: "Example Domain".to_string(),
            profile: "User".to_string(),
            active: true,
            group: Some("General".to_string()),
        },
        TabInfo {
            id: "tab-2".to_string(),
            url: "http://shop.localhost:8765".to_string(),
            title: "Demo Shop".to_string(),
            profile: "Agent".to_string(),
            active: false,
            group: Some("Shopping".to_string()),
        },
    ];

    let state = ShellServerState {
        tabs: Arc::new(Mutex::new(default_tabs)),
        store: store.clone(),
    };

    let router = create_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║                   Browse — AI-Native Browser                  ║");
    println!("║                                                               ║");
    println!("║  WebUI Shell running at: http://127.0.0.1:{:<5}              ║", port);
    println!("║  Zero Telemetry & Local-First Architectural Security         ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");

    if open_browser {
        println!("\nOpening Browse Shell in browser...");
        let url = format!("http://127.0.0.1:{port}");
        #[cfg(target_os = "windows")]
        {
            tokio::process::Command::new("cmd")
                .args(["/c", "start", &url])
                .spawn()
                .ok();
        }
    }

    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_ui_assets_are_non_empty() {
        assert!(!INDEX_HTML.is_empty(), "index.html must be embedded");
        assert!(!STYLE_CSS.is_empty(), "style.css must be embedded");
        assert!(!APP_JS.is_empty(), "app.js must be embedded");
        assert!(INDEX_HTML.contains("Browse — AI-Native Smart Browser"));
    }

    #[tokio::test]
    async fn router_initializes_with_valid_routes() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState {
            tabs: Arc::new(Mutex::new(vec![])),
            store,
        };
        let _router = create_router(state);
    }

    #[tokio::test]
    async fn rpc_tabs_list_and_create() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState {
            tabs: Arc::new(Mutex::new(vec![TabInfo {
                id: "tab-1".to_string(),
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                profile: "Personal".to_string(),
                active: true,
                group: Some("General".to_string()),
            }])),
            store,
        };

        // tabs.list
        let req = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "tabs.list".to_string(),
            params: json!({}),
        };
        let res = dispatch_rpc(state.clone(), req).await.unwrap();
        assert_eq!(res.as_array().unwrap().len(), 1);

        // tabs.new
        let req_new = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(2)),
            method: "tabs.new".to_string(),
            params: json!({"url": "https://rust-lang.org", "profile": "Agent"}),
        };
        let res_new = dispatch_rpc(state.clone(), req_new).await.unwrap();
        assert_eq!(res_new["url"], "https://rust-lang.org");
        assert_eq!(res_new["profile"], "Agent");
        assert_eq!(res_new["group"], "General");

        // Verify count is now 2
        let tabs = state.tabs.lock().unwrap();
        assert_eq!(tabs.len(), 2);
    }

    #[tokio::test]
    async fn rpc_system_status_and_memory_stats() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState {
            tabs: Arc::new(Mutex::new(vec![])),
            store,
        };

        let req = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(10)),
            method: "system.status".to_string(),
            params: json!({}),
        };
        let res = dispatch_rpc(state.clone(), req).await.unwrap();
        assert_eq!(res["status"], "ready");
        assert_eq!(res["telemetry"], false);

        let req_mem = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(11)),
            method: "memory.stats".to_string(),
            params: json!({}),
        };
        let res_mem = dispatch_rpc(state, req_mem).await.unwrap();
        assert_eq!(res_mem["pages"], 0);
    }

    #[tokio::test]
    async fn rpc_safety_analysis_and_tab_groups() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState {
            tabs: Arc::new(Mutex::new(vec![])),
            store,
        };

        // page.analyze_safety
        let req_safety = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(20)),
            method: "page.analyze_safety".to_string(),
            params: json!({
                "url": "https://paypal-security-update.com/login",
                "text": "Only 2 items left in stock! Renews automatically at $99/mo."
            }),
        };
        let res_safety = dispatch_rpc(state.clone(), req_safety).await.unwrap();
        assert_eq!(res_safety["phishing"]["severity"], "Dangerous");
        assert_eq!(res_safety["dark_patterns"].as_array().unwrap().len(), 2);

        // tabs.groups.create & list
        let req_grp = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(21)),
            method: "tabs.groups.create".to_string(),
            params: json!({ "title": "Research" }),
        };
        let res_grp = dispatch_rpc(state.clone(), req_grp).await.unwrap();
        assert_eq!(res_grp["title"], "Research");

        let req_list = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(22)),
            method: "tabs.groups.list".to_string(),
            params: json!({}),
        };
        let res_list = dispatch_rpc(state.clone(), req_list).await.unwrap();
        assert_eq!(res_list.as_array().unwrap().len(), 1);

        let req_entities = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(23)),
            method: "memory.entities".to_string(),
            params: json!({ "limit": 10 }),
        };
        let res_entities = dispatch_rpc(state.clone(), req_entities).await.unwrap();
        assert!(res_entities.is_array());

        // skills.list & skills.render
        let req_skills = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(24)),
            method: "skills.list".to_string(),
            params: json!({}),
        };
        let res_skills = dispatch_rpc(state.clone(), req_skills).await.unwrap();
        assert!(res_skills.as_array().unwrap().len() >= 3);

        let req_render = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(25)),
            method: "skills.render".to_string(),
            params: json!({
                "id": "github_issue_checker",
                "parameters": { "repo": "Yonawie/Browse" }
            }),
        };
        let res_render = dispatch_rpc(state.clone(), req_render).await.unwrap();
        assert!(res_render["prompt"].as_str().unwrap().contains("https://github.com/Yonawie/Browse/issues"));

        // omnibox.classify
        let req_classify = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(26)),
            method: "omnibox.classify".to_string(),
            params: json!({ "input": "what did I read about SQLite yesterday?" }),
        };
        let res_classify = dispatch_rpc(state, req_classify).await.unwrap();
        assert_eq!(res_classify["kind"], "memory_query");
    }
}

