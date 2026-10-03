//! The [`Gateway`] is what the rest of the browser calls: routing, cache,
//! concurrency limit, timeout and the local `model_calls` audit log, in one
//! place. Providers and the router stay free of these cross-cutting concerns.

use std::sync::Arc;
use std::time::{Duration, Instant};

use core_types::{Locality, Sensitivity};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::cache::{cacheable, chat_key, embed_key, ResponseCache};
use crate::stream::{self, ChatStream, StreamEvent};
use crate::{ModelError, ModelRequest, ModelResponse, RouteDecision, Router};

/// One row of `model_calls` (ADR-007: local audit, never leaves the device).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRecord {
    pub purpose: String,
    pub provider: String,
    pub model: String,
    pub locality: Locality,
    pub sensitivity: Sensitivity,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u32,
    pub cache_hit: bool,
    pub created_at: i64,
}

pub trait CallLog: Send + Sync {
    fn record(&self, call: CallRecord);
}

/// Collects records in memory (tests, the desktop demo).
#[derive(Default)]
pub struct MemoryCallLog(pub std::sync::Mutex<Vec<CallRecord>>);

impl CallLog for MemoryCallLog {
    fn record(&self, call: CallRecord) {
        self.0.lock().unwrap().push(call);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayResponse {
    pub response: ModelResponse,
    pub decision: RouteDecision,
    pub cache_hit: bool,
    pub latency_ms: u32,
}

pub struct Gateway {
    router: Router,
    cache: Option<Arc<dyn ResponseCache>>,
    log: Option<Arc<dyn CallLog>>,
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl Gateway {
    pub fn new(router: Router) -> Self {
        Self { router, cache: None, log: None, permits: Arc::new(Semaphore::new(4)), timeout: Duration::from_secs(120) }
    }

    pub fn with_cache(mut self, cache: Arc<dyn ResponseCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn with_log(mut self, log: Arc<dyn CallLog>) -> Self {
        self.log = Some(log);
        self
    }

    /// `max_concurrent` model calls in flight (a single GPU serialises anyway;
    /// the limit keeps the queue visible instead of piling up inside the
    /// sidecar), and a per-call timeout.
    pub fn with_limits(mut self, max_concurrent: usize, timeout: Duration) -> Self {
        self.permits = Arc::new(Semaphore::new(max_concurrent.max(1)));
        self.timeout = timeout;
        self
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub fn router_mut(&mut self) -> &mut Router {
        &mut self.router
    }

    fn log(&self, purpose: &str, req: &ModelRequest, resp: &ModelResponse, cache_hit: bool, started: Instant) -> u32 {
        let latency_ms = started.elapsed().as_millis() as u32;
        if let Some(log) = &self.log {
            log.record(CallRecord {
                purpose: purpose.to_string(),
                provider: resp.provider.clone(),
                model: resp.model.clone(),
                locality: resp.locality,
                sensitivity: req.sensitivity,
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
                latency_ms,
                cache_hit,
                created_at: now_ms(),
            });
        }
        latency_ms
    }

    pub async fn chat(&self, purpose: &str, req: &ModelRequest) -> Result<GatewayResponse, ModelError> {
        let started = Instant::now();
        let (provider, decision) = self.router.route(req)?;
        let model = provider.info().name;
        let key = cacheable(req).then(|| chat_key(&model, &model_id(&provider, req), req));
        if let (Some(cache), Some(key)) = (&self.cache, &key) {
            if let Some(hit) = cache.get_chat(key) {
                let latency_ms = self.log(purpose, req, &hit, true, started);
                return Ok(GatewayResponse { response: hit, decision, cache_hit: true, latency_ms });
            }
        }
        let _permit = self.permits.acquire().await.map_err(|_| ModelError::Provider {
            provider: decision.provider.clone(),
            message: "gateway shut down".into(),
        })?;
        let response = tokio::time::timeout(self.timeout, provider.chat(req))
            .await
            .map_err(|_| ModelError::Timeout(self.timeout.as_millis() as u64))??;
        if let (Some(cache), Some(key)) = (&self.cache, &key) {
            cache.put_chat(key, &response, req.sensitivity);
        }
        let latency_ms = self.log(purpose, req, &response, false, started);
        Ok(GatewayResponse { response, decision, cache_hit: false, latency_ms })
    }

    /// Streaming variant. Cache hits are replayed as a single delta; on a live
    /// stream the response is cached and logged when `Done` arrives.
    pub async fn chat_stream(
        &self,
        purpose: &str,
        req: &ModelRequest,
    ) -> Result<(ChatStream, RouteDecision), ModelError> {
        let started = Instant::now();
        let (provider, decision) = self.router.route(req)?;
        let key = cacheable(req).then(|| chat_key(&provider.info().name, &model_id(&provider, req), req));
        if let (Some(cache), Some(key)) = (&self.cache, &key) {
            if let Some(hit) = cache.get_chat(key) {
                self.log(purpose, req, &hit, true, started);
                return Ok((stream::single(hit), decision));
            }
        }
        let permit = self.permits.clone().acquire_owned().await.map_err(|_| ModelError::Provider {
            provider: decision.provider.clone(),
            message: "gateway shut down".into(),
        })?;
        let inner = tokio::time::timeout(self.timeout, provider.chat_stream(req))
            .await
            .map_err(|_| ModelError::Timeout(self.timeout.as_millis() as u64))??;

        let cache = self.cache.clone();
        let log = self.log.clone();
        let purpose = purpose.to_string();
        let sensitivity = req.sensitivity;
        let mut permit = Some(permit);
        let wrapped = inner.map(move |ev| {
            if let Ok(StreamEvent::Done(resp)) = &ev {
                if let (Some(cache), Some(key)) = (&cache, &key) {
                    cache.put_chat(key, resp, sensitivity);
                }
                if let Some(log) = &log {
                    log.record(CallRecord {
                        purpose: purpose.clone(),
                        provider: resp.provider.clone(),
                        model: resp.model.clone(),
                        locality: resp.locality,
                        sensitivity,
                        input_tokens: resp.usage.prompt_tokens,
                        output_tokens: resp.usage.completion_tokens,
                        latency_ms: started.elapsed().as_millis() as u32,
                        cache_hit: false,
                        created_at: now_ms(),
                    });
                }
                permit.take(); // release the concurrency slot as soon as the answer is complete
            }
            ev
        });
        Ok((Box::pin(wrapped), decision))
    }

    /// Embeddings, always local, cached per text. Only uncached texts hit the model.
    pub async fn embed(&self, purpose: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
        let started = Instant::now();
        let provider = self.router.local_embedder()?;
        let model = model_id_embed(&provider);
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut missing: Vec<usize> = vec![];
        for (i, t) in texts.iter().enumerate() {
            match self.cache.as_ref().and_then(|c| c.get_embedding(&embed_key(&model, t))) {
                Some(v) => out[i] = Some(v),
                None => missing.push(i),
            }
        }
        let hits = texts.len() - missing.len();
        if !missing.is_empty() {
            let _permit = self.permits.acquire().await.map_err(|_| ModelError::Provider {
                provider: provider.info().name,
                message: "gateway shut down".into(),
            })?;
            let batch: Vec<String> = missing.iter().map(|&i| texts[i].clone()).collect();
            let vectors = tokio::time::timeout(self.timeout, provider.embed(&batch))
                .await
                .map_err(|_| ModelError::Timeout(self.timeout.as_millis() as u64))??;
            for (&i, v) in missing.iter().zip(vectors) {
                if let Some(c) = &self.cache {
                    c.put_embedding(&embed_key(&model, &texts[i]), &v);
                }
                out[i] = Some(v);
            }
        }
        if let Some(log) = &self.log {
            let info = provider.info();
            log.record(CallRecord {
                purpose: purpose.to_string(),
                provider: info.name,
                model,
                locality: info.locality,
                sensitivity: Sensitivity::Private,
                input_tokens: 0,
                output_tokens: 0,
                latency_ms: started.elapsed().as_millis() as u32,
                cache_hit: hits == texts.len() && !texts.is_empty(),
                created_at: now_ms(),
            });
        }
        Ok(out.into_iter().map(|v| v.unwrap_or_default()).collect())
    }
}

fn model_id(provider: &Arc<dyn crate::ModelProvider>, req: &ModelRequest) -> String {
    provider.model_id(req.tier).unwrap_or_else(|| format!("{:?}", req.tier))
}

fn model_id_embed(provider: &Arc<dyn crate::ModelProvider>) -> String {
    provider.model_id(core_types::ModelTier::Embed).unwrap_or_else(|| "embed".into())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
    use crate::{Message, ModelProvider, ProviderInfo, RouterConfig};
    use core_types::ModelTier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl ModelProvider for Counting {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                name: "llama-server".into(),
                locality: Locality::Local,
                tiers: vec![ModelTier::Fast, ModelTier::Embed],
            }
        }
        fn model_id(&self, _tier: ModelTier) -> Option<String> {
            Some("m".into())
        }
        async fn chat(&self, r: &ModelRequest) -> Result<ModelResponse, ModelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            Ok(ModelResponse {
                content: format!("echo:{}", r.messages[0].content),
                tool_calls: vec![],
                usage: crate::Usage { prompt_tokens: 3, completion_tokens: 2 },
                provider: "llama-server".into(),
                model: "m".into(),
                locality: Locality::Local,
            })
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(texts.iter().map(|t| vec![t.len() as f32]).collect())
        }
    }

    fn gateway(delay: Duration) -> (Gateway, Arc<Counting>, Arc<MemoryCallLog>) {
        let p = Arc::new(Counting { calls: AtomicUsize::new(0), delay });
        let log = Arc::new(MemoryCallLog::default());
        let router = Router::new(RouterConfig::default()).with_provider(p.clone());
        let gw = Gateway::new(router).with_cache(Arc::new(MemoryCache::default())).with_log(log.clone());
        (gw, p, log)
    }

    fn req(text: &str) -> ModelRequest {
        ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user(text)])
    }

    #[tokio::test]
    async fn identical_requests_hit_the_cache_and_are_logged() {
        let (gw, p, log) = gateway(Duration::ZERO);
        let a = gw.chat("summarize", &req("hello")).await.unwrap();
        let b = gw.chat("summarize", &req("hello")).await.unwrap();
        assert!(!a.cache_hit && b.cache_hit);
        assert_eq!(a.response, b.response);
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
        {
            let recs = log.0.lock().unwrap();
            assert_eq!(recs.len(), 2);
            assert!(!recs[0].cache_hit && recs[1].cache_hit);
            assert_eq!(recs[0].purpose, "summarize");
            assert_eq!(recs[0].input_tokens, 3);
        }

        // Streaming replays the cached answer without touching the provider.
        let (s, _) = gw.chat_stream("summarize", &req("hello")).await.unwrap();
        let r = stream::collect(s).await.unwrap();
        assert_eq!(r.content, "echo:hello");
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn streamed_answers_populate_the_cache_on_done() {
        let (gw, p, log) = gateway(Duration::ZERO);
        let (s, _) = gw.chat_stream("qa", &req("x")).await.unwrap();
        let r = stream::collect(s).await.unwrap();
        assert_eq!(r.content, "echo:x");
        let hit = gw.chat("qa", &req("x")).await.unwrap();
        assert!(hit.cache_hit);
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
        assert_eq!(log.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn creative_requests_bypass_the_cache() {
        let (gw, p, _) = gateway(Duration::ZERO);
        let mut r = req("x");
        r.temperature = Some(0.9);
        gw.chat("draft", &r).await.unwrap();
        gw.chat("draft", &r).await.unwrap();
        assert_eq!(p.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn timeout_is_enforced() {
        let (gw, _, _) = gateway(Duration::from_millis(200));
        let gw = gw.with_limits(1, Duration::from_millis(20));
        let err = gw.chat("x", &req("slow")).await.err().unwrap();
        assert!(matches!(err, ModelError::Timeout(_)), "{err:?}");
    }

    #[tokio::test]
    async fn embeddings_are_cached_per_text() {
        let (gw, p, _) = gateway(Duration::ZERO);
        let v = gw.embed("index", &["ab".into(), "cde".into()]).await.unwrap();
        assert_eq!(v, vec![vec![2.0], vec![3.0]]);
        let v = gw.embed("index", &["cde".into(), "f".into(), "ab".into()]).await.unwrap();
        assert_eq!(v, vec![vec![3.0], vec![1.0], vec![2.0]]);
        assert_eq!(p.calls.load(Ordering::SeqCst), 2, "second call only embeds the new text");
        gw.embed("index", &["ab".into()]).await.unwrap();
        assert_eq!(p.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrency_limit_serialises_calls() {
        let (gw, _, _) = gateway(Duration::from_millis(50));
        let gw = Arc::new(gw.with_limits(1, Duration::from_secs(5)));
        let started = Instant::now();
        let a = gw.clone();
        let b = gw.clone();
        let (r1, r2) = (req("1"), req("2"));
        let (ra, rb) = tokio::join!(a.chat("p", &r1), b.chat("p", &r2));
        ra.unwrap();
        rb.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(95), "two 50 ms calls must not overlap");
    }
}
