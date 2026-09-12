//! `llama-server` (llama.cpp) adapter speaking the OpenAI-compatible HTTP API.
//!
//! Request/response shaping lives here; bytes on the wire go through
//! [`Transport`], which the application implements with its HTTP client of
//! choice. Only `127.0.0.1` targets are accepted by the desktop shell; the
//! adapter itself does not know about network policy.

use std::collections::HashMap;

use core_types::{Locality, ModelTier};
use serde_json::{json, Value};

use crate::{ModelError, ModelProvider, ModelRequest, ModelResponse, ProviderInfo, Role, ToolCallOut, Usage};

#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// POST `body` as JSON to `path` (e.g. `/v1/chat/completions`) and return the
    /// parsed JSON response.
    async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError>;
}

pub struct LlamaServerProvider {
    name: String,
    transport: Box<dyn Transport>,
    /// Which model alias (`--alias` / `--models-preset` name on the server) serves each tier.
    models: HashMap<ModelTier, String>,
}

impl LlamaServerProvider {
    pub fn new(transport: Box<dyn Transport>, models: HashMap<ModelTier, String>) -> Self {
        Self { name: "llama-server".into(), transport, models }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn model_for(&self, tier: ModelTier) -> Result<&str, ModelError> {
        self.models
            .get(&tier)
            .map(String::as_str)
            .ok_or_else(|| ModelError::NoRoute(format!("llama-server has no model for tier {tier:?}")))
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
        let message = v
            .pointer("/choices/0/message")
            .ok_or_else(|| ModelError::Protocol("missing choices[0].message".into()))?;
        let content = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| {
                        let id = c.get("id")?.as_str()?.to_string();
                        let f = c.get("function")?;
                        let name = f.get("name")?.as_str()?.to_string();
                        let raw = f.get("arguments")?;
                        let arguments = match raw {
                            Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                            other => other.clone(),
                        };
                        Some(ToolCallOut { id, name, arguments })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let usage = Usage {
            prompt_tokens: v.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            completion_tokens: v.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        };
        Ok(ModelResponse {
            content,
            tool_calls,
            usage,
            provider: self.name.clone(),
            model: model.to_string(),
            locality: Locality::Local,
        })
    }
}

#[async_trait::async_trait]
impl ModelProvider for LlamaServerProvider {
    fn info(&self) -> ProviderInfo {
        let mut tiers: Vec<ModelTier> = self.models.keys().copied().collect();
        tiers.sort();
        ProviderInfo { name: self.name.clone(), locality: Locality::Local, tiers }
    }

    async fn chat(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let body = self.build_chat_body(request)?;
        let model = self.model_for(request.tier)?.to_string();
        let v = self.transport.post_json("/v1/chat/completions", body).await?;
        self.parse_chat_response(&model, &v)
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
        let model = self.model_for(ModelTier::Embed)?;
        let body = json!({ "model": model, "input": texts, "encoding_format": "float" });
        let v = self.transport.post_json("/v1/embeddings", body).await?;
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| ModelError::Protocol("missing data[]".into()))?;
        let mut out = Vec::with_capacity(data.len());
        for item in data {
            let emb = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| ModelError::Protocol("missing embedding".into()))?;
            out.push(emb.iter().filter_map(Value::as_f64).map(|f| f as f32).collect());
        }
        if out.len() != texts.len() {
            return Err(ModelError::Protocol(format!("expected {} embeddings, got {}", texts.len(), out.len())));
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
    }

    #[async_trait::async_trait]
    impl Transport for Arc<Recording> {
        async fn post_json(&self, path: &str, body: Value) -> Result<Value, ModelError> {
            *self.last.lock().unwrap() = Some((path.to_string(), body));
            Ok(self.reply.clone())
        }
    }

    fn provider(reply: Value) -> (LlamaServerProvider, Arc<Recording>) {
        let rec = Arc::new(Recording { last: Mutex::new(None), reply });
        let models = HashMap::from([
            (ModelTier::Fast, "qwen3-4b".to_string()),
            (ModelTier::Smart, "qwen3-8b".to_string()),
            (ModelTier::Embed, "embeddinggemma-300m".to_string()),
        ]);
        (LlamaServerProvider::new(Box::new(rec.clone()), models), rec)
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
        let req = ModelRequest::new(ModelTier::Smart, Sensitivity::Public, vec![Message::system("s"), Message::user("u")])
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
    async fn embeddings_are_parsed_and_length_checked() {
        let (p, _) = provider(json!({ "data": [ { "embedding": [0.1, 0.2] }, { "embedding": [0.3, 0.4] } ] }));
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
}
