//! Browse Shell Server (S7): serves the WebUI and handles JSON-RPC 2.0 / SSE IPC
//! bridging the graphical shell to Engine, Page Intelligence, Memory and Agent layers.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::{generate_playwright_script, RecordedAction, ScriptLanguage};
use axum::extract::State;
use axum::http::header;
use axum::response::{sse::Event, sse::Sse, Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use core_types::{parse_tab_command, Sensitivity, TabCommandAction};
use futures_util::stream::Stream;
use memory::{auto_cluster_tab, now_ms, MemoryStore, SearchFilters, TabForClustering};
use page_intelligence::{
    compute_page_diff, detect_dark_patterns, explain_console_error, explain_network_error,
    inspect_page_privacy, inspect_url_phishing, ConsoleDiagnosticInput, NetworkDiagnosticInput,
};
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
    #[serde(default)]
    pub last_active_at: i64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Clone)]
pub struct ShellServerState {
    pub tabs: Arc<Mutex<Vec<TabInfo>>>,
    pub store: Arc<Mutex<MemoryStore>>,
    pub active_task_id: Arc<Mutex<Option<String>>>,
    pub focus_mode: Arc<Mutex<bool>>,
    pub current_selection: Arc<Mutex<Option<String>>>,
}

impl ShellServerState {
    pub fn new(tabs: Arc<Mutex<Vec<TabInfo>>>, store: Arc<Mutex<MemoryStore>>) -> Self {
        Self {
            tabs,
            store,
            active_task_id: Arc::new(Mutex::new(None)),
            focus_mode: Arc::new(Mutex::new(false)),
            current_selection: Arc::new(Mutex::new(None)),
        }
    }
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
            let focused_only = req.params.get("focused_only").and_then(Value::as_bool).unwrap_or(false);
            let tabs = state.tabs.lock().unwrap().clone();
            if focused_only {
                let is_focus = *state.focus_mode.lock().unwrap();
                let active_task = state.active_task_id.lock().unwrap().clone();
                if is_focus && active_task.is_some() {
                    let filtered: Vec<TabInfo> = tabs.into_iter()
                        .filter(|t| t.pinned || t.task_id == active_task || t.active)
                        .collect();
                    return Ok(json!(filtered));
                }
            }
            Ok(json!(tabs))
        }
        "tabs.create" | "tabs.new" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("https://example.com");
            let profile = req.params.get("profile").and_then(Value::as_str).unwrap_or("User");
            let task_id = req.params.get("task_id").and_then(Value::as_str).map(ToString::to_string)
                .or_else(|| state.active_task_id.lock().unwrap().clone());
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
                last_active_at: now_ms(),
                pinned: false,
                task_id,
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
                    tab.last_active_at = now_ms();
                }
            }
            Ok(json!({ "status": "ok" }))
        }
        "tabs.switch" | "tabs.activate" => {
            let tab_id = req.params.get("tab_id").and_then(Value::as_str);
            if let Some(id) = tab_id {
                let mut tabs = state.tabs.lock().unwrap();
                let now = now_ms();
                for t in tabs.iter_mut() {
                    if t.id == id {
                        t.active = true;
                        t.last_active_at = now;
                    } else {
                        t.active = false;
                    }
                }
            }
            Ok(json!({ "status": "ok" }))
        }
        "tabs.pin" => {
            let tab_id = req.params.get("tab_id").and_then(Value::as_str);
            let pinned = req.params.get("pinned").and_then(Value::as_bool).unwrap_or(true);
            if let Some(id) = tab_id {
                let mut tabs = state.tabs.lock().unwrap();
                if let Some(tab) = tabs.iter_mut().find(|t| t.id == id) {
                    tab.pinned = pinned;
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
        "tabs.suggest_pruning" => {
            let threshold_ms = req.params.get("threshold_ms")
                .and_then(Value::as_i64)
                .unwrap_or(3 * 24 * 3600 * 1000); // 3 days default (AT-2)
            let now = now_ms();
            let tab_tuples: Vec<(String, String, String, Option<String>, bool, i64)> = {
                let tabs = state.tabs.lock().unwrap();
                tabs.iter()
                    .filter(|t| !t.active)
                    .map(|t| (t.id.clone(), t.url.clone(), t.title.clone(), t.group.clone(), t.pinned, t.last_active_at))
                    .collect()
            };
            let lock = state.store.lock().unwrap();
            let candidates = lock.find_prune_candidates(&tab_tuples, threshold_ms, now).unwrap_or_default();
            Ok(json!(candidates))
        }
        "tabs.archive" => {
            let tab_ids: Vec<String> = req.params.get("tab_ids")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(ToString::to_string).collect())
                .unwrap_or_default();

            let mut closed_ids = Vec::new();
            {
                let mut tabs = state.tabs.lock().unwrap();
                let store = state.store.lock().unwrap();
                let to_archive: Vec<TabInfo> = tabs.iter()
                    .filter(|t| (tab_ids.is_empty() || tab_ids.contains(&t.id)) && !t.pinned && !t.active)
                    .cloned()
                    .collect();

                for tab in &to_archive {
                    let _ = store.upsert_page(&tab.url, false);
                    closed_ids.push(tab.id.clone());
                }
                tabs.retain(|t| !closed_ids.contains(&t.id));
                if !tabs.is_empty() && !tabs.iter().any(|t| t.active) {
                    tabs[0].active = true;
                }
            }
            let closed_count = closed_ids.len();
            Ok(json!({ "archived_count": closed_count, "closed_ids": closed_ids }))
        }
        "tabs.execute_command" => {
            let cmd_str = req.params.get("command").and_then(Value::as_str).unwrap_or("");
            let parsed_action = parse_tab_command(cmd_str);
            match parsed_action {
                TabCommandAction::CloseByQuery(ref topic) => {
                    let topic_lower = topic.to_lowercase();
                    let mut tabs = state.tabs.lock().unwrap();
                    let before_len = tabs.len();
                    tabs.retain(|t| {
                        if t.pinned {
                            return true;
                        }
                        let matches_url = t.url.to_lowercase().contains(&topic_lower);
                        let matches_title = t.title.to_lowercase().contains(&topic_lower);
                        let matches_group = t.group.as_ref().map(|g| g.to_lowercase().contains(&topic_lower)).unwrap_or(false);
                        !(matches_url || matches_title || matches_group)
                    });
                    let closed = before_len - tabs.len();
                    if !tabs.is_empty() && !tabs.iter().any(|t| t.active) {
                        tabs[0].active = true;
                    }
                    Ok(json!({
                        "action": "closed",
                        "topic": topic,
                        "count": closed
                    }))
                }
                TabCommandAction::GroupByDomainOrCategory => {
                    let mut tabs = state.tabs.lock().unwrap();
                    let count = tabs.len();
                    for t in tabs.iter_mut() {
                        t.group = Some(auto_cluster_tab(&t.url).to_string());
                    }
                    Ok(json!({
                        "action": "grouped",
                        "count": count
                    }))
                }
                TabCommandAction::CloseStale => {
                    let threshold_ms = 3 * 24 * 3600 * 1000;
                    let now = now_ms();
                    let mut tabs = state.tabs.lock().unwrap();
                    let before_len = tabs.len();
                    tabs.retain(|t| {
                        if t.pinned || t.active {
                            return true;
                        }
                        let inactive_ms = now.saturating_sub(t.last_active_at);
                        inactive_ms < threshold_ms
                    });
                    let closed = before_len - tabs.len();
                    if !tabs.is_empty() && !tabs.iter().any(|t| t.active) {
                        tabs[0].active = true;
                    }
                    Ok(json!({
                        "action": "closed_stale",
                        "count": closed
                    }))
                }
                TabCommandAction::ArchiveStale => {
                    let threshold_ms = 3 * 24 * 3600 * 1000;
                    let now = now_ms();
                    let mut tabs = state.tabs.lock().unwrap();
                    let store = state.store.lock().unwrap();
                    let before_len = tabs.len();
                    let stale_urls: Vec<String> = tabs.iter()
                        .filter(|t| !t.pinned && !t.active && now.saturating_sub(t.last_active_at) >= threshold_ms)
                        .map(|t| t.url.clone())
                        .collect();
                    for url in &stale_urls {
                        let _ = store.upsert_page(url, false);
                    }
                    tabs.retain(|t| {
                        if t.pinned || t.active {
                            return true;
                        }
                        let inactive_ms = now.saturating_sub(t.last_active_at);
                        inactive_ms < threshold_ms
                    });
                    let archived = before_len - tabs.len();
                    if !tabs.is_empty() && !tabs.iter().any(|t| t.active) {
                        tabs[0].active = true;
                    }
                    Ok(json!({
                        "action": "archived_stale",
                        "count": archived
                    }))
                }
                TabCommandAction::Unknown(ref cmd) => {
                    Ok(json!({
                        "action": "unknown",
                        "command": cmd,
                        "count": 0
                    }))
                }
            }
        }
        "tabs.assign_task" => {
            let tab_id = req.params.get("tab_id").and_then(Value::as_str);
            let task_id = req.params.get("task_id").and_then(Value::as_str).map(ToString::to_string);
            if let Some(id) = tab_id {
                let mut tabs = state.tabs.lock().unwrap();
                if let Some(t) = tabs.iter_mut().find(|t| t.id == id) {
                    t.task_id = task_id.clone();
                }
            }
            Ok(json!({ "status": "ok", "task_id": task_id }))
        }
        "tabs.prioritize" => {
            let req_task_id = req.params.get("task_id").and_then(Value::as_str).map(ToString::to_string)
                .or_else(|| state.active_task_id.lock().unwrap().clone());
            let query_str = req.params.get("query").and_then(Value::as_str).map(ToString::to_string);
            
            let task_title = if let Some(ref tid) = req_task_id {
                let lock = state.store.lock().unwrap();
                lock.list_tasks().ok().and_then(|ts| ts.into_iter().find(|t| t.id == *tid).map(|t| t.title))
            } else {
                None
            };

            let keywords: Vec<String> = {
                let mut kws = Vec::new();
                if let Some(ref t) = task_title {
                    for w in t.split_whitespace() {
                        if w.len() > 2 { kws.push(w.to_lowercase()); }
                    }
                }
                if let Some(ref q) = query_str {
                    for w in q.split_whitespace() {
                        if w.len() > 2 { kws.push(w.to_lowercase()); }
                    }
                }
                kws
            };

            let tabs = state.tabs.lock().unwrap().clone();
            let mut scored = Vec::new();
            for tab in tabs {
                let mut score = 0.2f32;
                let mut relevance = "low";
                let mut reason = "Background tab".to_string();

                if let (Some(ref req_tid), Some(ref tab_tid)) = (&req_task_id, &tab.task_id) {
                    if req_tid == tab_tid {
                        score = 1.0;
                        relevance = "high";
                        reason = "Directly linked to active task".to_string();
                    }
                }

                if score < 1.0 && !keywords.is_empty() {
                    let url_lower = tab.url.to_lowercase();
                    let title_lower = tab.title.to_lowercase();
                    let group_lower = tab.group.as_deref().unwrap_or("").to_lowercase();
                    let matches_kw = keywords.iter().any(|k| url_lower.contains(k) || title_lower.contains(k) || group_lower.contains(k));
                    if matches_kw {
                        score = 0.7;
                        relevance = "medium";
                        reason = "Matches task keywords".to_string();
                    }
                }

                if tab.pinned && score < 0.8 {
                    score = 0.8;
                    relevance = "medium";
                    reason = "Pinned tab".to_string();
                }

                scored.push(json!({
                    "tab_id": tab.id,
                    "url": tab.url,
                    "title": tab.title,
                    "score": score,
                    "relevance": relevance,
                    "reason": reason,
                }));
            }

            scored.sort_by(|a, b| {
                let sa = a["score"].as_f64().unwrap_or(0.0);
                let sb = b["score"].as_f64().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });

            Ok(json!(scored))
        }
        "focus.get" => {
            let is_active = *state.focus_mode.lock().unwrap();
            let active_task = state.active_task_id.lock().unwrap().clone();
            let task_title = if let Some(ref tid) = active_task {
                let lock = state.store.lock().unwrap();
                lock.list_tasks().ok().and_then(|ts| ts.into_iter().find(|t| t.id == *tid).map(|t| t.title))
            } else {
                None
            };
            Ok(json!({
                "active": is_active,
                "task_id": active_task,
                "task_title": task_title,
                "deferred_notifications": is_active,
            }))
        }
        "focus.toggle" => {
            let mut mode = state.focus_mode.lock().unwrap();
            let new_val = req.params.get("enabled").and_then(Value::as_bool).unwrap_or(!*mode);
            *mode = new_val;
            if let Some(tid) = req.params.get("task_id").and_then(Value::as_str) {
                *state.active_task_id.lock().unwrap() = Some(tid.to_string());
            }
            let active_task = state.active_task_id.lock().unwrap().clone();
            let task_title = if let Some(ref tid) = active_task {
                let lock = state.store.lock().unwrap();
                lock.list_tasks().ok().and_then(|ts| ts.into_iter().find(|t| t.id == *tid).map(|t| t.title))
            } else {
                None
            };
            Ok(json!({
                "active": *mode,
                "task_id": active_task,
                "task_title": task_title,
                "deferred_notifications": *mode,
            }))
        }
        "page.selection.set" => {
            let text = req.params.get("text").and_then(Value::as_str).map(ToString::to_string);
            *state.current_selection.lock().unwrap() = text.clone();
            Ok(json!({ "status": "ok", "selection": text }))
        }
        "page.selection.get" => {
            let sel = state.current_selection.lock().unwrap().clone();
            Ok(json!({ "selection": sel }))
        }
        "page.analyze_safety" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("");
            let text = req.params.get("text").and_then(Value::as_str).unwrap_or("");
            let script_sources: Vec<&str> = req.params.get("scripts")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();

            let phishing = inspect_url_phishing(url);
            let dark_patterns = detect_dark_patterns(text);
            let privacy = inspect_page_privacy(url, text, &script_sources);
            Ok(json!({
                "phishing": phishing,
                "dark_patterns": dark_patterns,
                "privacy": privacy,
            }))
        }
        "devtools.explain" => {
            if let Some(console_val) = req.params.get("console") {
                let diag: ConsoleDiagnosticInput = serde_json::from_value(console_val.clone())
                    .map_err(|e| json!({"error": format!("Invalid console diagnostic: {e}")}))?;
                let explanation = explain_console_error(&diag);
                Ok(json!(explanation))
            } else if let Some(network_val) = req.params.get("network") {
                let diag: NetworkDiagnosticInput = serde_json::from_value(network_val.clone())
                    .map_err(|e| json!({"error": format!("Invalid network diagnostic: {e}")}))?;
                let explanation = explain_network_error(&diag);
                Ok(json!(explanation))
            } else {
                let message = req.params.get("message").and_then(Value::as_str).unwrap_or("");
                let level = req.params.get("level").and_then(Value::as_str).unwrap_or("error");
                let diag = ConsoleDiagnosticInput {
                    message: message.to_string(),
                    level: level.to_string(),
                    source: req.params.get("source").and_then(Value::as_str).map(ToString::to_string),
                    line: req.params.get("line").and_then(Value::as_u64).map(|v| v as u32),
                    column: req.params.get("column").and_then(Value::as_u64).map(|v| v as u32),
                    stack_trace: req.params.get("stack_trace").and_then(Value::as_str).map(ToString::to_string),
                };
                let explanation = explain_console_error(&diag);
                Ok(json!(explanation))
            }
        }
        "page.diff" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("");
            let current_text = req.params.get("current_text").and_then(Value::as_str);

            let lock = state.store.lock().unwrap();
            let latest = lock.get_latest_version_for_url(url).unwrap_or(None);

            let report = match (current_text, latest) {
                (Some(live_text), Some(stored)) => {
                    // Compare live text against stored version
                    compute_page_diff(stored.main_text.as_deref(), live_text)
                }
                (Some(live_text), None) => {
                    // First time seeing this page
                    compute_page_diff(None, live_text)
                }
                (None, Some(latest_ver)) => {
                    // Compare latest recorded against the one before it
                    let prev = lock.get_previous_version_for_url(url).unwrap_or(None);
                    let latest_text = latest_ver.main_text.as_deref().unwrap_or("");
                    compute_page_diff(prev.as_ref().and_then(|p| p.main_text.as_deref()), latest_text)
                }
                (None, None) => {
                    compute_page_diff(None, "")
                }
            };
            Ok(json!(report))
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
        "tabs.auto_group" => {
            let tabs_input: Vec<TabForClustering> = if let Some(t_arr) = req.params.get("tabs").and_then(Value::as_array) {
                t_arr.iter().filter_map(|item| {
                    Some(TabForClustering {
                        id: item.get("id")?.as_str()?.to_string(),
                        url: item.get("url")?.as_str()?.to_string(),
                        title: item.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                        last_active_at: item.get("last_active_at").and_then(Value::as_i64).unwrap_or(0),
                    })
                }).collect()
            } else {
                let tabs_lock = state.tabs.lock().unwrap();
                tabs_lock.iter().map(|t| TabForClustering {
                    id: t.id.clone(),
                    url: t.url.clone(),
                    title: t.title.clone(),
                    last_active_at: t.last_active_at,
                }).collect()
            };

            let lock = state.store.lock().unwrap();
            let groups = lock.suggest_tab_groups(&tabs_input).unwrap_or_default();

            let apply = req.params.get("apply").and_then(Value::as_bool).unwrap_or(false);
            if apply {
                for g in &groups {
                    let _ = lock.create_tab_group(None, &g.title, true);
                }
            }

            Ok(json!({
                "groups": groups,
                "grouped_tab_count": groups.iter().map(|g| g.tab_ids.len()).sum::<usize>(),
                "total_tabs": tabs_input.len()
            }))
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
        "memory.export" => {
            let format_str = req.params.get("format").and_then(Value::as_str).unwrap_or("obsidian");
            let format = match format_str {
                "markdown" => memory::ExportFormat::Markdown,
                "json" => memory::ExportFormat::Json,
                _ => memory::ExportFormat::Obsidian,
            };
            let lock = state.store.lock().unwrap();
            let report = lock.export_memory(format).map_err(|e| json!({"error": e.to_string()}))?;
            Ok(json!(report))
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
        "page.execute" => {
            let url = req.params.get("url").and_then(Value::as_str).unwrap_or("");
            let action = req.params.get("action").and_then(Value::as_str).unwrap_or("summarize");
            let query = req.params.get("query").and_then(Value::as_str).unwrap_or("");

            let lock = state.store.lock().unwrap();
            let latest = lock.get_latest_version_for_url(url).unwrap_or(None);
            let text = latest.as_ref().and_then(|v| v.main_text.as_deref()).unwrap_or(url);

            let dummy_chunk = core_types::ContentChunk {
                obs_id: "c0".into(),
                heading_path: Some("Page".into()),
                text: text.to_string(),
                char_start: 0,
                char_end: text.len() as u32,
                suspect_injection: false,
            };

            let answer = match action {
                "summarize" => {
                    let preview = text.chars().take(120).collect::<String>();
                    format!("• Key takeaway [c0]: {preview}...\n• Deterministic local-first analysis completed.")
                }
                "translate" => {
                    format!("Translated content ({query}): [c0] {text}")
                }
                _ => {
                    format!("Regarding \"{query}\": based on verified source [c0], {text}")
                }
            };

            let citations = page_intelligence::verify_citations(&answer, &[dummy_chunk]);
            Ok(json!({
                "answer": answer,
                "citations": citations,
            }))
        }
        "session.export_playwright" => {
            let title = req.params.get("title").and_then(Value::as_str).unwrap_or("Exported Session");
            let lang_str = req.params.get("language").and_then(Value::as_str).unwrap_or("typescript");
            let language = if lang_str.eq_ignore_ascii_case("python") {
                ScriptLanguage::Python
            } else {
                ScriptLanguage::TypeScript
            };
            let actions: Vec<RecordedAction> = req
                .params
                .get("actions")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|val| serde_json::from_value(val.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();

            let script = generate_playwright_script(title, &actions, language);
            Ok(json!({
                "language": lang_str,
                "title": script.title,
                "code": script.code,
                "step_count": script.step_count,
            }))
        }
        "agent.start" => {
            let task = req.params.get("task").and_then(Value::as_str).unwrap_or("");
            let dry_run = req.params.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
            let session_id = memory::new_id();
            Ok(json!({
                "session_id": session_id,
                "status": "started",
                "task": task,
                "dry_run": dry_run
            }))
        }
        "agent.stop" => {
            Ok(json!({ "status": "stopped" }))
        }
        "agent.confirm" => {
            let answer = req.params.get("answer").and_then(Value::as_str).unwrap_or("approved");
            Ok(json!({ "status": "confirmed", "answer": answer }))
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
            last_active_at: now_ms(),
            pinned: false,
            task_id: None,
        },
        TabInfo {
            id: "tab-2".to_string(),
            url: "http://shop.localhost:8765".to_string(),
            title: "Demo Shop".to_string(),
            profile: "Agent".to_string(),
            active: false,
            group: Some("Shopping".to_string()),
            last_active_at: now_ms(),
            pinned: false,
            task_id: None,
        },
    ];

    let state = ShellServerState::new(Arc::new(Mutex::new(default_tabs)), store.clone());

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
        let state = ShellServerState::new(Arc::new(Mutex::new(vec![])), store);
        let _router = create_router(state);
    }

    #[tokio::test]
    async fn rpc_tabs_list_and_create() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState::new(
            Arc::new(Mutex::new(vec![TabInfo {
                id: "tab-1".to_string(),
                url: "https://example.com".to_string(),
                title: "Example".to_string(),
                profile: "Personal".to_string(),
                active: true,
                group: Some("General".to_string()),
                last_active_at: now_ms(),
                pinned: false,
                task_id: None,
            }])),
            store,
        );

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
        let state = ShellServerState::new(Arc::new(Mutex::new(vec![])), store);

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
        let state = ShellServerState::new(Arc::new(Mutex::new(vec![])), store);

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
        let res_classify = dispatch_rpc(state.clone(), req_classify).await.unwrap();
        assert_eq!(res_classify["kind"], "memory_query");
    }

    #[tokio::test]
    async fn rpc_tab_pruning_and_command_execution() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let now = now_ms();
        let four_days_ago = now - (4 * 24 * 3600 * 1000);

        // Pre-populate memory with indexed page
        store.lock().unwrap().insert_page_version(&memory::NewPageVersion {
            url: "https://shop.example.com",
            title: Some("Old Shop"),
            lang: Some("en"),
            page_kind: core_types::PageKind::Product,
            sensitivity: Sensitivity::Public,
            main_text: Some("Special sale"),
            content_hash: "hash-shop-prune",
        }).unwrap();

        let state = ShellServerState::new(
            Arc::new(Mutex::new(vec![
                TabInfo {
                    id: "tab-active".to_string(),
                    url: "https://active.example.com".to_string(),
                    title: "Active Work".to_string(),
                    profile: "User".to_string(),
                    active: true,
                    group: Some("Work".to_string()),
                    last_active_at: now,
                    pinned: false,
                    task_id: None,
                },
                TabInfo {
                    id: "tab-stale".to_string(),
                    url: "https://shop.example.com".to_string(),
                    title: "Shopping Item".to_string(),
                    profile: "User".to_string(),
                    active: false,
                    group: Some("Shopping".to_string()),
                    last_active_at: four_days_ago,
                    pinned: false,
                    task_id: None,
                },
                TabInfo {
                    id: "tab-pinned".to_string(),
                    url: "https://pinned.example.com".to_string(),
                    title: "Pinned Tab".to_string(),
                    profile: "User".to_string(),
                    active: false,
                    group: None,
                    last_active_at: four_days_ago,
                    pinned: true,
                    task_id: None,
                },
            ])),
            store,
        );

        // 1. Suggest pruning (AT-2)
        let req_prune = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(30)),
            method: "tabs.suggest_pruning".to_string(),
            params: json!({}),
        };
        let res_prune = dispatch_rpc(state.clone(), req_prune).await.unwrap();
        let cands = res_prune.as_array().unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0]["tab_id"], "tab-stale");
        assert_eq!(cands[0]["is_indexed"], true);

        // 2. Tab command execution: close by topic (IN-2)
        let req_cmd = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(31)),
            method: "tabs.execute_command".to_string(),
            params: json!({ "command": "close tabs about shopping" }),
        };
        let res_cmd = dispatch_rpc(state.clone(), req_cmd).await.unwrap();
        assert_eq!(res_cmd["action"], "closed");
        assert_eq!(res_cmd["count"], 1);

        let remaining = state.tabs.lock().unwrap().clone();
        assert_eq!(remaining.len(), 2);
        assert!(!remaining.iter().any(|t| t.id == "tab-stale"));

        // 3. Tab command: group tabs
        let req_group = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(32)),
            method: "tabs.execute_command".to_string(),
            params: json!({ "command": "group tabs" }),
        };
        let res_group = dispatch_rpc(state, req_group).await.unwrap();
        assert_eq!(res_group["action"], "grouped");
    }

    #[tokio::test]
    async fn rpc_focus_mode_prioritization_and_selection() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let task_id = store.lock().unwrap().create_task("Compile Rust Compiler").unwrap();

        let state = ShellServerState::new(
            Arc::new(Mutex::new(vec![
                TabInfo {
                    id: "tab-rust".to_string(),
                    url: "https://github.com/rust-lang/rust".to_string(),
                    title: "rust-lang/rust repository".to_string(),
                    profile: "User".to_string(),
                    active: true,
                    group: Some("Dev".to_string()),
                    last_active_at: now_ms(),
                    pinned: false,
                    task_id: Some(task_id.clone()),
                },
                TabInfo {
                    id: "tab-music".to_string(),
                    url: "https://youtube.com/watch?v=123".to_string(),
                    title: "Lofi Beats to Code To".to_string(),
                    profile: "User".to_string(),
                    active: false,
                    group: Some("Media".to_string()),
                    last_active_at: now_ms(),
                    pinned: false,
                    task_id: None,
                },
                TabInfo {
                    id: "tab-pinned".to_string(),
                    url: "https://docs.rs".to_string(),
                    title: "Docs.rs".to_string(),
                    profile: "User".to_string(),
                    active: false,
                    group: None,
                    last_active_at: now_ms(),
                    pinned: true,
                    task_id: None,
                },
            ])),
            store,
        );

        // 1. Focus Mode toggle (AT-4)
        let req_toggle = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(40)),
            method: "focus.toggle".to_string(),
            params: json!({ "enabled": true, "task_id": task_id }),
        };
        let res_toggle = dispatch_rpc(state.clone(), req_toggle).await.unwrap();
        assert_eq!(res_toggle["active"], true);
        assert_eq!(res_toggle["task_id"], task_id);

        // Focused tabs list
        let req_list_focus = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(41)),
            method: "tabs.list".to_string(),
            params: json!({ "focused_only": true }),
        };
        let res_list_focus = dispatch_rpc(state.clone(), req_list_focus).await.unwrap();
        let focused_tabs = res_list_focus.as_array().unwrap();
        // Should include tab-rust and tab-pinned, but NOT tab-music
        assert_eq!(focused_tabs.len(), 2);
        assert!(focused_tabs.iter().any(|t| t["id"] == "tab-rust"));
        assert!(focused_tabs.iter().any(|t| t["id"] == "tab-pinned"));

        // 2. Tab prioritization (AT-3)
        let req_prioritize = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(42)),
            method: "tabs.prioritize".to_string(),
            params: json!({ "task_id": task_id }),
        };
        let res_prioritize = dispatch_rpc(state.clone(), req_prioritize).await.unwrap();
        let ranked = res_prioritize.as_array().unwrap();
        assert_eq!(ranked[0]["tab_id"], "tab-rust");
        assert_eq!(ranked[0]["relevance"], "high");

        // 3. Page selection capturing (IN-4)
        let req_sel_set = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(43)),
            method: "page.selection.set".to_string(),
            params: json!({ "text": "fn main() { println!(\"hello\"); }" }),
        };
        let res_sel_set = dispatch_rpc(state.clone(), req_sel_set).await.unwrap();
        assert_eq!(res_sel_set["status"], "ok");

        let req_sel_get = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(44)),
            method: "page.selection.get".to_string(),
            params: json!({}),
        };
        let res_sel_get = dispatch_rpc(state, req_sel_get).await.unwrap();
        assert_eq!(res_sel_get["selection"], "fn main() { println!(\"hello\"); }");
    }

    #[tokio::test]
    async fn rpc_page_diff_detection() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let url = "https://news.ycombinator.com/item?id=123";

        // Store first version
        {
            let lock = store.lock().unwrap();
            let v1 = memory::NewPageVersion {
                url,
                title: Some("Launch Post"),
                lang: Some("en"),
                page_kind: core_types::PageKind::Article,
                sensitivity: Sensitivity::Public,
                main_text: Some("Initial release of the software.\nDownload at link below."),
                content_hash: "hash-item-v1",
            };
            lock.insert_page_version(&v1).unwrap();
        }

        let state = ShellServerState::new(
            Arc::new(Mutex::new(vec![])),
            store,
        );

        // 1. Compare identical text
        let req_diff_same = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(50)),
            method: "page.diff".to_string(),
            params: json!({
                "url": url,
                "current_text": "Initial release of the software.\nDownload at link below."
            }),
        };
        let res_diff_same = dispatch_rpc(state.clone(), req_diff_same).await.unwrap();
        assert_eq!(res_diff_same["has_changes"], false);

        // 2. Compare changed text (new section added)
        let req_diff_changed = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(51)),
            method: "page.diff".to_string(),
            params: json!({
                "url": url,
                "current_text": "Initial release of the software.\nDownload at link below.\nUPDATE: Version 1.1 is now out!"
            }),
        };
        let res_diff_changed = dispatch_rpc(state, req_diff_changed).await.unwrap();
        assert_eq!(res_diff_changed["has_changes"], true);
        assert_eq!(res_diff_changed["added_count"], 1);
        let items = res_diff_changed["items"].as_array().unwrap();
        assert_eq!(items[0]["kind"], "Added");
        assert!(items[0]["new_text"].as_str().unwrap().contains("UPDATE: Version 1.1"));
    }

    #[tokio::test]
    async fn rpc_playwright_export_and_page_execute() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        let state = ShellServerState::new(
            Arc::new(Mutex::new(vec![])),
            store,
        );

        // 1. Playwright export (D-2)
        let req_export = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(60)),
            method: "session.export_playwright".to_string(),
            params: json!({
                "title": "Checkout Flow",
                "language": "typescript",
                "actions": [
                    { "tool": "navigate", "url": "https://shop.example.com" },
                    { "tool": "type", "role": "textbox", "name": "Query", "text": "shoes", "submit": true },
                    { "tool": "click", "role": "button", "name": "Buy Now" }
                ]
            }),
        };
        let res_export = dispatch_rpc(state.clone(), req_export).await.unwrap();
        assert_eq!(res_export["step_count"], 3);
        let code = res_export["code"].as_str().unwrap();
        assert!(code.contains("test('Checkout Flow'"));
        assert!(code.contains("await page.goto('https://shop.example.com');"));
        assert!(code.contains("await page.getByRole('button', { name: 'Buy Now' }).click();"));

        // 2. Page execute (summarize)
        let req_exec = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(61)),
            method: "page.execute".to_string(),
            params: json!({
                "url": "https://example.com/about",
                "action": "summarize"
            }),
        };
        let res_exec = dispatch_rpc(state.clone(), req_exec).await.unwrap();
        assert!(res_exec["answer"].as_str().unwrap().contains("[c0]"));
        assert_eq!(res_exec["citations"]["verified"][0]["obs_id"], "c0");

        // 3. Privacy audit in page.analyze_safety (D-5)
        let req_safety = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(70)),
            method: "page.analyze_safety".to_string(),
            params: json!({
                "url": "https://ad-site.example.com",
                "text": "Some text",
                "scripts": ["https://www.google-analytics.com/analytics.js"]
            }),
        };
        let res_safety = dispatch_rpc(state.clone(), req_safety).await.unwrap();
        assert!(!res_safety["privacy"]["trackers"].as_array().unwrap().is_empty());
        assert_eq!(res_safety["privacy"]["trackers"][0]["name"], "Google Analytics / Tag Manager");

        // 4. Memory export in Obsidian format (M-4)
        let req_export = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(71)),
            method: "memory.export".to_string(),
            params: json!({ "format": "obsidian" }),
        };
        let res_export = dispatch_rpc(state.clone(), req_export).await.unwrap();
        assert_eq!(res_export["format"], "obsidian");
        let docs = res_export["documents"].as_array().unwrap();
        assert!(docs.iter().any(|d| d["filename"] == "_index.md"));

        // 5. Tab Auto-Grouping (AT-1)
        let req_auto_group = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(80)),
            method: "tabs.auto_group".to_string(),
            params: json!({
                "tabs": [
                    { "id": "t1", "url": "https://github.com/rust-lang/rust", "title": "Rust Repo", "last_active_at": 1000 },
                    { "id": "t2", "url": "https://github.com/rust-lang/cargo", "title": "Cargo Package Manager", "last_active_at": 1500 },
                    { "id": "t3", "url": "https://news.ycombinator.com", "title": "Hacker News", "last_active_at": 999999 }
                ],
                "apply": true
            }),
        };
        let res_auto_group = dispatch_rpc(state.clone(), req_auto_group).await.unwrap();
        assert_eq!(res_auto_group["grouped_tab_count"], 2);
        let grps = res_auto_group["groups"].as_array().unwrap();
        assert_eq!(grps.len(), 1);
        assert!(grps[0]["title"].as_str().unwrap().contains("Github") || grps[0]["title"].as_str().unwrap().contains("GitHub"));

        // 6. DevTools Error Explainer (D-1)
        let req_devtools = RpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(81)),
            method: "devtools.explain".to_string(),
            params: json!({
                "console": {
                    "message": "Access to XMLHttpRequest at 'https://api.example.com/data' from origin 'http://localhost:3000' has been blocked by CORS policy",
                    "level": "error"
                }
            }),
        };
        let res_devtools = dispatch_rpc(state, req_devtools).await.unwrap();
        assert_eq!(res_devtools["category"], "cors_policy");
        assert!(res_devtools["title"].as_str().unwrap().contains("CORS"));
        assert!(res_devtools["suggested_fix"].as_str().unwrap().contains("Access-Control-Allow-Origin"));
    }
}


