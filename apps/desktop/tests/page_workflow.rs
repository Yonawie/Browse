//! Real Chromium and fixture pages with a deterministic local model endpoint.
//! This verifies wiring and privacy routing, not language-model answer quality.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::routing::{get, post};
use axum::{Json, Router};
use fixtures::Fixture;
use serde_json::{json, Value};

async fn chat(State(requests): State<Arc<Mutex<Vec<Value>>>>, Json(body): Json<Value>) -> (HeaderMap, String) {
    requests.lock().unwrap().push(body);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
    let text = "The city approved solar roofs for new schools [c0].";
    let event = json!({ "choices": [{ "delta": { "content": text } }] });
    (headers, format!("data: {event}\n\ndata: [DONE]\n\n"))
}

#[tokio::test]
async fn page_commands_use_observed_content_and_local_model() {
    let Some(browser) = engine_cdp::launcher::find_browser() else {
        eprintln!("skipping: no Chromium-based browser found");
        return;
    };
    let fixture = Fixture::spawn().await;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_url = format!("http://{}", listener.local_addr().unwrap());
    let server = Router::new()
        .route("/v1/models", get(|| async { Json(json!({ "data": [{ "id": "test-model" }] })) }))
        .route("/v1/chat/completions", post(chat))
        .with_state(requests.clone());
    let server_task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let page_url = fixture.url("news", "/a/solar-roofs");

    for action in [vec!["summarize"], vec!["ask", "What did the council approve?"], vec!["translate", "Russian"]] {
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_browse-desktop"))
                .arg("page")
                .arg(&page_url)
                .args(&action)
                .env("BROWSE_CHROME", &browser)
                .env("BROWSE_LLAMA_CHAT_URL", &model_url)
                .env("BROWSE_OFFLINE", "1")
                .output(),
        )
        .await
        .expect("page command timed out")
        .expect("start page command");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "action {action:?}: {stderr}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "The city approved solar roofs for new schools [c0]."
        );
        assert!(stderr.contains("route: llama-server (Local)"), "{stderr}");
        assert!(!stderr.contains("warning: answer cites missing"), "{stderr}");
    }

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 3);
    for request in captured.iter() {
        let user = request["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("city council voted 9"), "{user}");
        assert!(user.contains("[c0]"), "{user}");
    }
    assert!(captured[1]["messages"][1]["content"].as_str().unwrap().contains("What did the council approve?"));
    assert!(captured[2]["messages"][1]["content"].as_str().unwrap().contains("Russian"));
    server_task.abort();
    fixture.abort();
}
