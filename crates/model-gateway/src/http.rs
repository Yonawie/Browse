//! `reqwest`-based [`Transport`]: JSON POST and server-sent-events streaming.
//!
//! One instance per endpoint. The API key, when set, is sent as
//! `Authorization: Bearer …` and never logged. Loopback endpoints get a short
//! connect timeout so a dead sidecar fails fast instead of hanging the UI.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;

use crate::openai_compat::Transport;
use crate::stream::JsonStream;
use crate::ModelError;

#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<Arc<str>>,
    name: String,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

impl HttpTransport {
    /// `base_url` like `http://127.0.0.1:8081` or `https://api.example.com/v1`
    /// (paths passed to the transport are appended, with `/v1` de-duplicated).
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url: String = base_url.into();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("reqwest client");
        Self { client, base_url: base_url.trim_end_matches('/').to_string(), api_key: None, name: "http".into() }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        let key: String = key.into();
        self.api_key = if key.is_empty() { None } else { Some(Arc::from(key.as_str())) };
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout)
            .build()
            .expect("reqwest client");
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        // Cloud base URLs conventionally already end in `/v1`.
        if self.base_url.ends_with("/v1") {
            format!("{}/{}", self.base_url, path.trim_start_matches("v1/"))
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    fn request(&self, path: &str, body: &Value) -> reqwest::RequestBuilder {
        let mut r = self.client.post(self.url(path)).json(body);
        if let Some(k) = &self.api_key {
            r = r.bearer_auth(k);
        }
        r
    }

    fn err(&self, e: impl std::fmt::Display) -> ModelError {
        ModelError::Provider { provider: self.name.clone(), message: e.to_string() }
    }

    /// GET a JSON document (health checks, model listings).
    pub async fn get_json(&self, path: &str) -> Result<Value, ModelError> {
        let mut r = self.client.get(self.url(path));
        if let Some(k) = &self.api_key {
            r = r.bearer_auth(k);
        }
        let resp = r.send().await.map_err(|e| self.err(e))?;
        resp.json().await.map_err(|e| ModelError::Protocol(e.to_string()))
    }
}

#[async_trait::async_trait]
impl Transport for HttpTransport {
    async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError> {
        let resp = self.request(path, &body).send().await.map_err(|e| self.err(e))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| self.err(e))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => Ok(v),
            Err(_) if !status.is_success() => Err(self.err(format!("HTTP {status}: {}", truncate(&text, 300)))),
            Err(e) => Err(ModelError::Protocol(format!("non-JSON response ({status}): {e}"))),
        }
    }

    async fn post_stream(&self, path: &str, body: Value) -> Result<JsonStream, ModelError> {
        let resp =
            self.request(path, &body).header("Accept", "text/event-stream").send().await.map_err(|e| self.err(e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(self.err(format!("HTTP {status}: {}", truncate(&text, 300))));
        }
        let name = self.name.clone();
        let bytes = resp.bytes_stream();
        let s = futures_util::stream::unfold(
            (bytes, SseBuffer::default(), false, name),
            |(mut bytes, mut buf, done, name)| async move {
                if done {
                    return None;
                }
                loop {
                    if let Some(item) = buf.next_event() {
                        return match item {
                            SseItem::Done => None,
                            SseItem::Json(v) => Some((Ok(v), (bytes, buf, false, name))),
                            SseItem::Bad(line) => Some((
                                Err(ModelError::Protocol(format!("bad SSE payload: {}", truncate(&line, 200)))),
                                (bytes, buf, true, name),
                            )),
                        };
                    }
                    match bytes.next().await {
                        Some(Ok(chunk)) => buf.push(&chunk),
                        Some(Err(e)) => {
                            let err = ModelError::Provider { provider: name.clone(), message: e.to_string() };
                            return Some((Err(err), (bytes, buf, true, name)));
                        }
                        None => {
                            buf.finish();
                            if let Some(SseItem::Json(v)) = buf.next_event() {
                                return Some((Ok(v), (bytes, buf, true, name)));
                            }
                            return None;
                        }
                    }
                }
            },
        );
        Ok(Box::pin(s))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

enum SseItem {
    Json(Value),
    Done,
    Bad(String),
}

/// Incremental parser for `text/event-stream`: events are separated by a
/// blank line; only `data:` fields matter for the OpenAI protocol.
#[derive(Default)]
struct SseBuffer {
    pending: String,
    eof: bool,
}

impl SseBuffer {
    fn push(&mut self, chunk: &[u8]) {
        self.pending.push_str(&String::from_utf8_lossy(chunk));
    }

    fn finish(&mut self) {
        self.eof = true;
        if !self.pending.ends_with("\n\n") && !self.pending.is_empty() {
            self.pending.push_str("\n\n");
        }
    }

    fn next_event(&mut self) -> Option<SseItem> {
        loop {
            let normalized = self.pending.replace("\r\n", "\n");
            let end = normalized.find("\n\n")?;
            let event = normalized[..end].to_string();
            self.pending = normalized[end + 2..].to_string();
            let data: Vec<&str> = event
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if data.is_empty() {
                continue; // comment / event-name only
            }
            let payload = data.join("\n");
            if payload.trim() == "[DONE]" {
                return Some(SseItem::Done);
            }
            return Some(match serde_json::from_str::<Value>(&payload) {
                Ok(v) => SseItem::Json(v),
                Err(_) => SseItem::Bad(payload),
            });
        }
    }
}

/// Dispatches to one [`HttpTransport`] per model id, for setups where each
/// model is served by its own `llama-server` process (see [`crate::sidecar`]).
#[derive(Default)]
pub struct RoutingTransport {
    by_model: std::sync::RwLock<HashMap<String, Arc<HttpTransport>>>,
}

impl RoutingTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, model: impl Into<String>, transport: Arc<HttpTransport>) {
        self.by_model.write().unwrap().insert(model.into(), transport);
    }

    pub fn remove(&self, model: &str) {
        self.by_model.write().unwrap().remove(model);
    }

    pub fn models(&self) -> Vec<String> {
        self.by_model.read().unwrap().keys().cloned().collect()
    }

    fn pick(&self, body: &Value) -> Result<Arc<HttpTransport>, ModelError> {
        let model = body.get("model").and_then(Value::as_str).unwrap_or("");
        self.by_model
            .read()
            .unwrap()
            .get(model)
            .cloned()
            .ok_or_else(|| ModelError::NoRoute(format!("model `{model}` is not being served")))
    }
}

#[async_trait::async_trait]
impl Transport for RoutingTransport {
    async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError> {
        self.pick(&body)?.post_json(path, body).await
    }
    async fn post_stream(&self, path: &str, body: Value) -> Result<JsonStream, ModelError> {
        self.pick(&body)?.post_stream(path, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_buffer_splits_events_and_handles_partial_chunks() {
        let mut b = SseBuffer::default();
        b.push(b"data: {\"a\":1}\n\ndata: {\"b\"");
        assert!(matches!(b.next_event(), Some(SseItem::Json(v)) if v["a"] == 1));
        assert!(b.next_event().is_none(), "second event incomplete");
        b.push(b":2}\r\n\r\n: keep-alive\n\ndata: [DONE]\n\n");
        assert!(matches!(b.next_event(), Some(SseItem::Json(v)) if v["b"] == 2));
        assert!(matches!(b.next_event(), Some(SseItem::Done)));
    }

    #[test]
    fn sse_buffer_flushes_trailing_event_at_eof() {
        let mut b = SseBuffer::default();
        b.push(b"data: {\"x\":true}");
        assert!(b.next_event().is_none());
        b.finish();
        assert!(matches!(b.next_event(), Some(SseItem::Json(v)) if v["x"] == true));
    }

    #[test]
    fn url_joins_v1_once() {
        let t = HttpTransport::new("https://api.example.com/v1/");
        assert_eq!(t.url("/v1/chat/completions"), "https://api.example.com/v1/chat/completions");
        let t = HttpTransport::new("http://127.0.0.1:8081");
        assert_eq!(t.url("/v1/embeddings"), "http://127.0.0.1:8081/v1/embeddings");
        assert_eq!(t.url("/health"), "http://127.0.0.1:8081/health");
    }

    #[test]
    fn debug_never_prints_the_key() {
        let t = HttpTransport::new("http://x").with_api_key("sk-secret");
        assert!(!format!("{t:?}").contains("sk-secret"));
    }

    #[tokio::test]
    async fn routing_transport_requires_a_served_model() {
        let r = RoutingTransport::new();
        let err = r.post_json("/v1/chat/completions", serde_json::json!({ "model": "ghost" })).await.err().unwrap();
        assert!(matches!(err, ModelError::NoRoute(_)));
        r.set("ghost", Arc::new(HttpTransport::new("http://127.0.0.1:1")));
        assert_eq!(r.models(), vec!["ghost".to_string()]);
    }
}
