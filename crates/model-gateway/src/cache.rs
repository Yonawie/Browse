//! Response and embedding cache keyed by a content hash of (provider, model,
//! request). Deterministic requests (summaries, extractions, embeddings) are
//! the majority of model traffic; re-opening a page must not re-run them.
//!
//! The gateway decides *what* is cacheable ([`cacheable`]); the store decides
//! *where* it lives. [`MemoryCache`] is the in-process LRU; the desktop app
//! provides a persistent store over SQLite that honours "forget this page".

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use core_types::Sensitivity;
use serde_json::json;

use crate::{ModelRequest, ModelResponse};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(pub String);

impl CacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable key over everything that influences the answer. `cloud_opt_in` and
/// `sensitivity` are routing inputs, not content, and are excluded on purpose:
/// the provider/model already identify where the answer came from.
pub fn chat_key(provider: &str, model: &str, request: &ModelRequest) -> CacheKey {
    let canonical = json!({
        "provider": provider,
        "model": model,
        "messages": request.messages,
        "tools": request.tools,
        "json_schema": request.json_schema,
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
    });
    CacheKey(blake3::hash(canonical.to_string().as_bytes()).to_hex().to_string())
}

pub fn embed_key(model: &str, text: &str) -> CacheKey {
    let mut h = blake3::Hasher::new();
    h.update(model.as_bytes());
    h.update(&[0]);
    h.update(text.as_bytes());
    CacheKey(h.finalize().to_hex().to_string())
}

/// Only near-deterministic, tool-free requests are cached. Tool-calling turns
/// depend on live page state that is not part of the key.
pub fn cacheable(request: &ModelRequest) -> bool {
    request.tools.is_empty() && request.temperature.unwrap_or(0.1) <= 0.1
}

pub trait ResponseCache: Send + Sync {
    fn get_chat(&self, key: &CacheKey) -> Option<ModelResponse>;
    fn put_chat(&self, key: &CacheKey, response: &ModelResponse, sensitivity: Sensitivity);
    fn get_embedding(&self, key: &CacheKey) -> Option<Vec<f32>>;
    fn put_embedding(&self, key: &CacheKey, vector: &[f32]);
}

struct Lru<V> {
    cap: usize,
    map: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V: Clone> Lru<V> {
    fn new(cap: usize) -> Self {
        Self { cap, map: HashMap::new(), order: VecDeque::new() }
    }

    fn get(&mut self, k: &str) -> Option<V> {
        let v = self.map.get(k)?.clone();
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
        }
        self.order.push_back(k.to_string());
        Some(v)
    }

    fn put(&mut self, k: &str, v: V) {
        if self.cap == 0 {
            return;
        }
        if self.map.insert(k.to_string(), v).is_none() {
            self.order.push_back(k.to_string());
        } else if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
            self.order.push_back(k.to_string());
        }
        while self.map.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    fn len(&self) -> usize {
        self.map.len()
    }
}

/// In-process LRU cache.
pub struct MemoryCache {
    chats: Mutex<Lru<ModelResponse>>,
    embeddings: Mutex<Lru<Vec<f32>>>,
}

impl MemoryCache {
    pub fn new(chat_capacity: usize, embedding_capacity: usize) -> Self {
        Self { chats: Mutex::new(Lru::new(chat_capacity)), embeddings: Mutex::new(Lru::new(embedding_capacity)) }
    }

    pub fn len(&self) -> (usize, usize) {
        (self.chats.lock().unwrap().len(), self.embeddings.lock().unwrap().len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == (0, 0)
    }
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new(256, 20_000)
    }
}

impl ResponseCache for MemoryCache {
    fn get_chat(&self, key: &CacheKey) -> Option<ModelResponse> {
        self.chats.lock().unwrap().get(&key.0)
    }
    fn put_chat(&self, key: &CacheKey, response: &ModelResponse, _sensitivity: Sensitivity) {
        self.chats.lock().unwrap().put(&key.0, response.clone());
    }
    fn get_embedding(&self, key: &CacheKey) -> Option<Vec<f32>> {
        self.embeddings.lock().unwrap().get(&key.0)
    }
    fn put_embedding(&self, key: &CacheKey, vector: &[f32]) {
        self.embeddings.lock().unwrap().put(&key.0, vector.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ToolSpec};
    use core_types::{Locality, ModelTier};

    fn req(text: &str) -> ModelRequest {
        ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user(text)])
    }

    #[test]
    fn keys_depend_on_content_model_and_provider_but_not_routing_flags() {
        let a = chat_key("llama-server", "m", &req("hi"));
        let mut b_req = req("hi");
        b_req.cloud_opt_in = true;
        b_req.sensitivity = Sensitivity::Private;
        assert_eq!(a, chat_key("llama-server", "m", &b_req));
        assert_ne!(a, chat_key("llama-server", "m", &req("hi!")));
        assert_ne!(a, chat_key("llama-server", "other", &req("hi")));
        assert_ne!(a, chat_key("cloud", "m", &req("hi")));
        assert_ne!(a, chat_key("llama-server", "m", &req("hi").with_schema(json!({ "type": "object" }))));
        assert_ne!(embed_key("e", "a"), embed_key("e", "b"));
        assert_ne!(embed_key("e", "a"), embed_key("f", "a"));
    }

    #[test]
    fn tool_calls_and_creative_sampling_are_not_cacheable() {
        assert!(cacheable(&req("x")));
        let mut r = req("x");
        r.temperature = Some(0.7);
        assert!(!cacheable(&r));
        let r = req("x").with_tools(vec![ToolSpec { name: "t".into(), description: "".into(), parameters: json!({}) }]);
        assert!(!cacheable(&r));
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let cache = MemoryCache::new(2, 2);
        let resp = |s: &str| ModelResponse {
            content: s.into(),
            tool_calls: vec![],
            usage: Default::default(),
            provider: "p".into(),
            model: "m".into(),
            locality: Locality::Local,
        };
        cache.put_chat(&CacheKey("a".into()), &resp("a"), Sensitivity::Public);
        cache.put_chat(&CacheKey("b".into()), &resp("b"), Sensitivity::Public);
        assert!(cache.get_chat(&CacheKey("a".into())).is_some()); // a becomes most recent
        cache.put_chat(&CacheKey("c".into()), &resp("c"), Sensitivity::Public);
        assert!(cache.get_chat(&CacheKey("b".into())).is_none(), "b was least recently used");
        assert!(cache.get_chat(&CacheKey("a".into())).is_some());
        assert_eq!(cache.len().0, 2);

        cache.put_embedding(&CacheKey("e".into()), &[1.0, 2.0]);
        assert_eq!(cache.get_embedding(&CacheKey("e".into())), Some(vec![1.0, 2.0]));
    }
}
