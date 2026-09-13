//! Provider speaking the OpenAI-compatible chat/embeddings protocol. Used for
//! the local `llama-server` (llama.cpp) and for cloud endpoints that expose
//! the same API; the two differ only in [`Locality`] and in how the transport
//! authenticates.
//!
//! Request/response shaping lives here; bytes on the wire go through
//! [`Transport`], so the protocol handling is unit-testable without a server.

use std::collections::HashMap;

use core_types::{Locality, ModelTier};
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::stream::{ChatStream, JsonStream, StreamEvent};
use crate::{ModelError, ModelProvider, ModelRequest, ModelResponse, ProviderInfo, Role, ToolCallOut, Usage};

#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// POST `body` as JSON to `path` (e.g. `/v1/chat/completions`) and return the
    /// parsed JSON response.
    async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError>;

    /// POST `body` and return the server-sent-event payloads as parsed JSON
    /// values, one per `data:` line, ending before `[DONE]`. Transports that
    /// cannot stream return `ModelError::Unsupported`; callers then fall back
    /// to [`Transport::post_json`].
    async fn post_stream(&self, _path: &str, _body: Value) -> Result<JsonStream, ModelError> {
        Err(ModelError::Unsupported("transport does not stream".into()))
    }
}

pub struct OpenAiCompatProvider {
    name: String,
    locality: Locality,
    transport: Box<dyn Transport>,
    /// Which model id (`--alias` on llama-server, model name on cloud APIs) serves each tier.
    models: HashMap<ModelTier, String>,
}

/// The local `llama-server` provider.
pub type LlamaServerProvider = OpenAiCompatProvider;

impl OpenAiCompatProvider {
    /// Local `llama-server` on loopback.
    pub fn new(transport: Box<dyn Transport>, models: HashMap<ModelTier, String>) -> Self {
        Self { name: "llama-server".into(), locality: Locality::Local, transport, models }
    }

    /// A cloud endpoint (OpenAI, OpenRouter, Mistral, a self-hosted vLLM…).
    /// The router treats it as `Locality::Cloud`: never for Private/Secret data.
    pub fn cloud(name: impl Into<String>, transport: Box<dyn Transport>, models: HashMap<ModelTier, String>) -> Self {
        Self { name: name.into(), locality: Locality::Cloud, transport, models }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn model_for(&self, tier: ModelTier) -> Result<&str, ModelError> {
        self.models
            .get(&tier)
            .map(String::as_str)
            .ok_or_else(|| ModelError::NoRoute(format!("{} has no model for tier {tier:?}", self.name)))
    }

    /// Build the OpenAI-compatible request body. Public for testing and for
    /// logging the exact payload into `model_calls` (without message content).
    pub fn build_chat_body(&self, request: &ModelRequest) -> Result<Value, ModelError> {
        let model = self.model_for(request.tier)?;
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let mut v = json!({ "role": role, "content": m.content });
                if let Some(id) = &m.tool_call_id {
                    v["tool_call_id"] = json!(id);
                }
                v
            })
            .collect();

        let mut body = json!({
            "model": model,
            "messages": messages,
            // Deterministic defaults: agents should not be creative.
            "temperature": request.temperature.unwrap_or(0.1),
        });
        if let Some(max) = request.max_tokens {
            body["max_tokens"] = json!(max);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = json!("auto");
        }
        if let Some(schema) = &request.json_schema {
            // llama-server converts this to a GBNF grammar → output is guaranteed valid.
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "response", "strict": true, "schema": schema }
            });
        }
        Ok(body)
    }

    pub fn parse_chat_response(&self, model: &str, v: &Value) -> Result<ModelResponse, ModelError> {
        if let Some(err) = v.get("error") {
            return Err(ModelError::Provider { provider: self.name.clone(), message: err.to_string() });
        }
        let message =
            v.pointer("/choices/0/message").ok_or_else(|| ModelError::Protocol("missing choices[0].message".into()))?;
        let content = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| calls.iter().filter_map(parse_tool_call).collect())
            .unwrap_or_default();
        Ok(ModelResponse {
            content,
            tool_calls,
            usage: parse_usage(v),
            provider: self.name.clone(),
            model: model.to_string(),
            locality: self.locality,
        })
    }

    /// Fold OpenAI streaming chunks into [`StreamEvent`]s. Tool-call arguments
    /// arrive as string fragments spread over many chunks; they are assembled
    /// per index and emitted once complete, at the end of the stream.
    fn fold_stream(&self, model: String, chunks: JsonStream) -> ChatStream {
        let provider = self.name.clone();
        let locality = self.locality;
        let state = StreamFold { content: String::new(), tools: Vec::new(), usage: Usage::default() };
        let s = futures_util::stream::unfold(
            (chunks, state, false, provider, model, locality),
            |(mut chunks, mut st, done, provider, model, locality)| async move {
                if done {
                    return None;
                }
                loop {
                    match chunks.next().await {
                        Some(Ok(v)) => {
                            if let Some(err) = v.get("error") {
                                let e = ModelError::Provider { provider: provider.clone(), message: err.to_string() };
                                return Some((Err(e), (chunks, st, true, provider, model, locality)));
                            }
                            if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                                st.usage = parse_usage(&json!({ "usage": u }));
                            }
                            let Some(delta) = v.pointer("/choices/0/delta") else { continue };
                            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                                for c in calls {
                                    st.push_tool_delta(c);
                                }
                            }
                            if let Some(text) = delta.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                                st.content.push_str(text);
                                return Some((
                                    Ok(StreamEvent::Delta(text.to_string())),
                                    (chunks, st, false, provider, model, locality),
                                ));
                            }
                        }
                        Some(Err(e)) => return Some((Err(e), (chunks, st, true, provider, model, locality))),
                        None => {
                            let resp = ModelResponse {
                                content: std::mem::take(&mut st.content),
                                tool_calls: st.finish_tools(),
                                usage: st.usage.clone(),
                                provider: provider.clone(),
                                model: model.clone(),
                                locality,
                            };
                            return Some((Ok(StreamEvent::Done(resp)), (chunks, st, true, provider, model, locality)));
                        }
                    }
                }
            },
        );
        Box::pin(s)
    }
}

struct ToolDelta {
    id: String,
    name: String,
    arguments: String,
}

struct StreamFold {
    content: String,
    tools: Vec<ToolDelta>,
    usage: Usage,
}

impl StreamFold {
    fn push_tool_delta(&mut self, c: &Value) {
        let idx = c.get("index").and_then(Value::as_u64).unwrap_or(self.tools.len() as u64) as usize;
        while self.tools.len() <= idx {
            self.tools.push(ToolDelta { id: String::new(), name: String::new(), arguments: String::new() });
        }
        let t = &mut self.tools[idx];
        if let Some(id) = c.get("id").and_then(Value::as_str) {
            t.id = id.to_string();
        }
        if let Some(f) = c.get("function") {
            if let Some(n) = f.get("name").and_then(Value::as_str) {
                t.name.push_str(n);
            }
            match f.get("arguments") {
                Some(Value::String(s)) => t.arguments.push_str(s),
                Some(other) if !other.is_null() => t.arguments.push_str(&other.to_string()),
                _ => {}
            }
        }
    }

    fn finish_tools(&mut self) -> Vec<ToolCallOut> {
        std::mem::take(&mut self.tools)
            .into_iter()
            .enumerate()
            .filter(|(_, t)| !t.name.is_empty())
            .map(|(i, t)| ToolCallOut {
                id: if t.id.is_empty() { format!("call_{i}") } else { t.id },
                name: t.name,
                arguments: serde_json::from_str(&t.arguments).unwrap_or(Value::Null),
            })
            .collect()
    }
}

fn parse_tool_call(c: &Value) -> Option<ToolCallOut> {
    let id = c.get("id")?.as_str()?.to_string();
    let f = c.get("function")?;
    let name = f.get("name")?.as_str()?.to_string();
    let raw = f.get("arguments")?;
    let arguments = match raw {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    Some(ToolCallOut { id, name, arguments })
}

fn parse_usage(v: &Value) -> Usage {
    Usage {
        prompt_tokens: v.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        completion_tokens: v.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}

#[async_trait::async_trait]
impl ModelProvider for OpenAiCompatProvider {
    fn info(&self) -> ProviderInfo {
        let mut tiers: Vec<ModelTier> = self.models.keys().copied().collect();
        tiers.sort();
        ProviderInfo { name: self.name.clone(), locality: self.locality, tiers }
    }

    fn model_id(&self, tier: ModelTier) -> Option<String> {
        self.models.get(&tier).cloned()
    }

    async fn chat(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let body = self.build_chat_body(request)?;
        let model = self.model_for(request.tier)?.to_string();
        let v = self.transport.post_json("/v1/chat/completions", body).await?;
        self.parse_chat_response(&model, &v)
    }

    async fn chat_stream(&self, request: &ModelRequest) -> Result<ChatStream, ModelError> {
        let mut body = self.build_chat_body(request)?;
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
        let model = self.model_for(request.tier)?.to_string();
        match self.transport.post_stream("/v1/chat/completions", body).await {
            Ok(chunks) => Ok(self.fold_stream(model, chunks)),
            Err(ModelError::Unsupported(_)) => {
                let resp = self.chat(request).await?;
                Ok(crate::stream::single(resp))
            }
            Err(e) => Err(e),
        }
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
        let model = self.model_for(ModelTier::Embed)?;
        let body = json!({ "model": model, "input": texts, "encoding_format": "float" });
        let v = self.transport.post_json("/v1/embeddings", body).await?;
        if let Some(err) = v.get("error") {
            return Err(ModelError::Provider { provider: self.name.clone(), message: err.to_string() });
        }
        let data =
            v.get("data").and_then(Value::as_array).ok_or_else(|| ModelError::Protocol("missing data[]".into()))?;
        let mut out = vec![Vec::new(); texts.len()];
        for (i, item) in data.iter().enumerate() {
            let emb = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| ModelError::Protocol("missing embedding".into()))?;
            // Servers may reorder; honour `index` when present.
            let idx = item.get("index").and_then(Value::as_u64).map(|x| x as usize).unwrap_or(i);
            if idx >= out.len() {
                return Err(ModelError::Protocol(format!("embedding index {idx} out of range")));
            }
            out[idx] = emb.iter().filter_map(Value::as_f64).map(|f| f as f32).collect();
        }
        if data.len() != texts.len() || out.iter().any(Vec::is_empty) {
            return Err(ModelError::Protocol(format!("expected {} embeddings, got {}", texts.len(), data.len())));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ToolSpec};
    use core_types::Sensitivity;
    use std::sync::{Arc, Mutex};

    struct Recording {
        last: Mutex<Option<(String, Value)>>,
        reply: Value,
        stream: Option<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl Transport for Arc<Recording> {
        async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError> {
            *self.last.lock().unwrap() = Some((path.to_string(), body));
            Ok(self.reply.clone())
        }
        async fn post_stream(&self, path: &str, body: Value) -> Result<JsonStream, ModelError> {
            *self.last.lock().unwrap() = Some((path.to_string(), body));
            match &self.stream {
                Some(chunks) => Ok(Box::pin(futures_util::stream::iter(chunks.clone().into_iter().map(Ok)))),
                None => Err(ModelError::Unsupported("no stream".into())),
            }
        }
    }

    fn models() -> HashMap<ModelTier, String> {
        HashMap::from([
            (ModelTier::Fast, "qwen3-4b".to_string()),
            (ModelTier::Smart, "qwen3-8b".to_string()),
            (ModelTier::Embed, "embeddinggemma-300m".to_string()),
        ])
    }

    fn provider(reply: Value) -> (OpenAiCompatProvider, Arc<Recording>) {
        let rec = Arc::new(Recording { last: Mutex::new(None), reply, stream: None });
        (OpenAiCompatProvider::new(Box::new(rec.clone()), models()), rec)
    }

    fn streaming(chunks: Vec<Value>) -> (OpenAiCompatProvider, Arc<Recording>) {
        let rec = Arc::new(Recording { last: Mutex::new(None), reply: Value::Null, stream: Some(chunks) });
        (OpenAiCompatProvider::new(Box::new(rec.clone()), models()), rec)
    }

    #[tokio::test]
    async fn builds_openai_compatible_body_with_schema_and_tools() {
        let reply = json!({
            "choices": [{ "message": {
                "content": "",
                "tool_calls": [{ "id": "c1", "type": "function",
                    "function": { "name": "click", "arguments": "{\"ref\":\"n17\"}" } }]
            }}],
            "usage": { "prompt_tokens": 120, "completion_tokens": 9 }
        });
        let (p, rec) = provider(reply);
        let req =
            ModelRequest::new(ModelTier::Smart, Sensitivity::Public, vec![Message::system("s"), Message::user("u")])
                .with_tools(vec![ToolSpec {
                    name: "click".into(),
                    description: "Click".into(),
                    parameters: json!({ "type": "object", "properties": { "ref": { "type": "string" } } }),
                }])
                .with_schema(json!({ "type": "object" }));

        let resp = p.chat(&req).await.unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "click");
        assert_eq!(resp.tool_calls[0].arguments["ref"], "n17");
        assert_eq!(resp.usage.prompt_tokens, 120);
        assert_eq!(resp.locality, Locality::Local);
        assert_eq!(resp.model, "qwen3-8b");

        let last = rec.last.lock().unwrap().clone().unwrap();
        assert_eq!(last.0, "/v1/chat/completions");
        assert_eq!(last.1["model"], "qwen3-8b");
        assert_eq!(last.1["tools"][0]["function"]["name"], "click");
        assert_eq!(last.1["response_format"]["type"], "json_schema");
        assert_eq!(last.1["messages"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn embeddings_are_parsed_reordered_and_length_checked() {
        let (p, _) = provider(json!({ "data": [
            { "index": 1, "embedding": [0.3, 0.4] }, { "index": 0, "embedding": [0.1, 0.2] } ] }));
        let out = p.embed(&["a".into(), "b".into()]).await.unwrap();
        assert_eq!(out, vec![vec![0.1f32, 0.2], vec![0.3, 0.4]]);

        let (p, _) = provider(json!({ "data": [ { "embedding": [0.1] } ] }));
        assert!(matches!(p.embed(&["a".into(), "b".into()]).await, Err(ModelError::Protocol(_))));
    }

    #[tokio::test]
    async fn missing_tier_is_a_route_error_and_server_errors_propagate() {
        let (p, _) = provider(json!({ "error": { "message": "model not loaded" } }));
        let req = ModelRequest::new(ModelTier::Vision, Sensitivity::Public, vec![Message::user("u")]);
        assert!(matches!(p.chat(&req).await, Err(ModelError::NoRoute(_))));
        let req = ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user("u")]);
        assert!(matches!(p.chat(&req).await, Err(ModelError::Provider { .. })));
    }

    #[tokio::test]
    async fn stream_folds_deltas_tool_calls_and_usage() {
        let (p, rec) = streaming(vec![
            json!({ "choices": [{ "delta": { "role": "assistant", "content": "" } }] }),
            json!({ "choices": [{ "delta": { "content": "Hel" } }] }),
            json!({ "choices": [{ "delta": { "content": "lo" } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "c9",
                "function": { "name": "click", "arguments": "{\"re" } }] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0,
                "function": { "arguments": "f\":\"n1\"}" } }] } }] }),
            json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 4 } }),
        ]);
        let req = ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user("u")]);
        let mut s = p.chat_stream(&req).await.unwrap();
        let mut deltas = vec![];
        let mut done = None;
        while let Some(ev) = s.next().await {
            match ev.unwrap() {
                StreamEvent::Delta(d) => deltas.push(d),
                StreamEvent::Done(r) => done = Some(r),
            }
        }
        assert_eq!(deltas, vec!["Hel", "lo"]);
        let r = done.expect("Done event");
        assert_eq!(r.content, "Hello");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].id, "c9");
        assert_eq!(r.tool_calls[0].arguments["ref"], "n1");
        assert_eq!(r.usage.completion_tokens, 4);
        let last = rec.last.lock().unwrap().clone().unwrap();
        assert_eq!(last.1["stream"], true);
        assert_eq!(last.1["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn stream_falls_back_to_plain_chat_when_transport_cannot_stream() {
        let (p, _) = provider(json!({ "choices": [{ "message": { "content": "plain" } }] }));
        let req = ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user("u")]);
        let events: Vec<_> = p.chat_stream(&req).await.unwrap().collect().await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], Ok(StreamEvent::Delta(d)) if d == "plain"));
        assert!(matches!(&events[1], Ok(StreamEvent::Done(r)) if r.content == "plain"));
    }

    #[tokio::test]
    async fn stream_error_chunk_surfaces_as_provider_error() {
        let (p, _) = streaming(vec![json!({ "error": { "message": "context overflow" } })]);
        let req = ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user("u")]);
        let events: Vec<_> = p.chat_stream(&req).await.unwrap().collect().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Err(ModelError::Provider { .. })));
    }

    #[test]
    fn cloud_constructor_marks_locality() {
        let rec = Arc::new(Recording { last: Mutex::new(None), reply: Value::Null, stream: None });
        let p = OpenAiCompatProvider::cloud("openrouter", Box::new(rec), models());
        assert_eq!(p.info().locality, Locality::Cloud);
        assert_eq!(p.info().name, "openrouter");
    }
}
