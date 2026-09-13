//! Integration tests against a real llama.cpp. They skip themselves unless
//! the environment points at running servers and/or a binary + model:
//!
//! * `BROWSE_LLAMA_CHAT_URL`  e.g. `http://127.0.0.1:8081` (any chat model)
//! * `BROWSE_LLAMA_EMBED_URL` e.g. `http://127.0.0.1:8082` (`--embeddings`)
//! * `BROWSE_LLAMA_SERVER` + `BROWSE_LLAMA_MODEL` to test spawning a sidecar.
//!
//! The model alias is read from `/v1/models`, so any model works.

#![cfg(feature = "http")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_types::{ModelTier, Sensitivity};
use futures_util::StreamExt;
use model_gateway::sidecar::{LlamaSidecar, SidecarConfig, SidecarPool};
use model_gateway::{
    Gateway, HttpTransport, LoadPlan, MemoryCache, MemoryCallLog, Message, ModelRequest, OpenAiCompatProvider, Router,
    RouterConfig, StreamEvent,
};
use serde_json::json;

async fn alias_of(t: &HttpTransport) -> Option<String> {
    let v = t.get_json("/v1/models").await.ok()?;
    v["data"][0]["id"].as_str().map(str::to_string)
}

async fn chat_provider() -> Option<(OpenAiCompatProvider, String)> {
    let url = std::env::var("BROWSE_LLAMA_CHAT_URL").ok()?;
    let t = HttpTransport::new(&url).with_timeout(Duration::from_secs(600));
    let Some(alias) = alias_of(&t).await else {
        eprintln!("skipping: {url} not reachable");
        return None;
    };
    let models = HashMap::from([(ModelTier::Fast, alias.clone()), (ModelTier::Smart, alias.clone())]);
    Some((OpenAiCompatProvider::new(Box::new(t), models), alias))
}

async fn embed_provider() -> Option<OpenAiCompatProvider> {
    let url = std::env::var("BROWSE_LLAMA_EMBED_URL").ok()?;
    let t = HttpTransport::new(&url).with_timeout(Duration::from_secs(120));
    let Some(alias) = alias_of(&t).await else {
        eprintln!("skipping: {url} not reachable");
        return None;
    };
    Some(OpenAiCompatProvider::new(Box::new(t), HashMap::from([(ModelTier::Embed, alias)])))
}

#[tokio::test]
async fn chat_plain_and_streaming_agree_and_schema_is_enforced() {
    let Some((provider, alias)) = chat_provider().await else { return };
    let provider = Arc::new(provider);
    let router =
        Router::new(RouterConfig { offline_mode: true, cloud_allowed_classes: vec![] }).with_provider(provider);
    let log = Arc::new(MemoryCallLog::default());
    let gw = Gateway::new(router)
        .with_cache(Arc::new(MemoryCache::default()))
        .with_log(log.clone())
        .with_limits(1, Duration::from_secs(600));

    let mut req = ModelRequest::new(
        ModelTier::Fast,
        Sensitivity::Private,
        vec![
            Message::system("Answer with exactly one word, no punctuation."),
            Message::user("What colour is a clear daytime sky?"),
        ],
    );
    req.max_tokens = Some(8);
    req.temperature = Some(0.0);

    let t = Instant::now();
    let a = gw.chat("test", &req).await.unwrap();
    eprintln!("plain: {:?} in {:?} usage={:?}", a.response.content, t.elapsed(), a.response.usage);
    assert!(a.response.content.to_lowercase().contains("blue"), "{}", a.response.content);
    assert_eq!(a.response.model, alias);
    assert!(a.response.usage.prompt_tokens > 0);
    assert!(!a.cache_hit);
    assert_eq!(a.decision.reason, "local: offline mode");

    // Second identical call is served from cache without touching the server.
    let b = gw.chat("test", &req).await.unwrap();
    assert!(b.cache_hit);
    assert_eq!(a.response.content, b.response.content);

    // Streaming for a fresh prompt: deltas concatenate to Done.content.
    let mut req2 = req.clone();
    req2.messages[1] = Message::user("What colour is fresh grass?");
    let t = Instant::now();
    let (mut s, _) = gw.chat_stream("test", &req2).await.unwrap();
    let mut deltas = String::new();
    let mut done = None;
    let mut first_token = None;
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            StreamEvent::Delta(d) => {
                first_token.get_or_insert_with(|| t.elapsed());
                deltas.push_str(&d)
            }
            StreamEvent::Done(r) => done = Some(r),
        }
    }
    let done = done.expect("Done");
    eprintln!(
        "stream: {:?} first token {:?} total {:?} usage={:?}",
        done.content,
        first_token,
        t.elapsed(),
        done.usage
    );
    assert_eq!(deltas, done.content);
    assert!(done.content.to_lowercase().contains("green"), "{}", done.content);
    assert!(done.usage.completion_tokens > 0, "usage must arrive via stream_options.include_usage");

    // Structured output: grammar-constrained JSON.
    let schema = json!({
        "type": "object",
        "properties": { "city": { "type": "string" }, "country": { "type": "string" } },
        "required": ["city", "country"],
        "additionalProperties": false
    });
    let mut req3 = ModelRequest::new(
        ModelTier::Fast,
        Sensitivity::Public,
        vec![Message::user("Extract the city and country: 'Rain is expected in Paris, France, tomorrow.'")],
    )
    .with_schema(schema);
    req3.max_tokens = Some(64);
    let r = gw.chat("extract", &req3).await.unwrap();
    let v = r.response.json().expect("valid JSON");
    eprintln!("schema: {v}");
    assert_eq!(v["city"].as_str().map(|s| s.to_lowercase()), Some("paris".into()));
    assert_eq!(v["country"].as_str().map(|s| s.to_lowercase()), Some("france".into()));

    let recs = log.0.lock().unwrap();
    assert_eq!(recs.len(), 4);
    assert!(recs.iter().all(|r| r.locality == core_types::Locality::Local));
}

#[tokio::test]
async fn embeddings_are_normalised_and_semantically_ordered() {
    let Some(provider) = embed_provider().await else { return };
    let router = Router::new(RouterConfig::default()).with_provider(Arc::new(provider));
    let gw = Gateway::new(router).with_cache(Arc::new(MemoryCache::default()));
    let texts = vec![
        "How do I renew my passport online?".to_string(),
        "Passport renewal application form and fees".to_string(),
        "Best chocolate cake recipe with cocoa".to_string(),
    ];
    let t = Instant::now();
    let v = gw.embed("index", &texts).await.unwrap();
    eprintln!("embed 3 texts: {:?}, dim={}", t.elapsed(), v[0].len());
    assert_eq!(v.len(), 3);
    assert!(v[0].len() >= 256);
    let cos = |a: &[f32], b: &[f32]| {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    };
    let same_topic = cos(&v[0], &v[1]);
    let other_topic = cos(&v[0], &v[2]);
    eprintln!("cos(passport, passport)={same_topic:.3} cos(passport, cake)={other_topic:.3}");
    assert!(same_topic > other_topic + 0.1, "related texts must be closer");

    let t = Instant::now();
    let again = gw.embed("index", &texts[..1]).await.unwrap();
    assert_eq!(again[0], v[0]);
    assert!(t.elapsed() < Duration::from_millis(50), "cache hit must not call the model");
}

#[tokio::test]
async fn sidecar_starts_serves_and_stops() {
    let (Ok(bin), Ok(model)) = (std::env::var("BROWSE_LLAMA_SERVER"), std::env::var("BROWSE_LLAMA_MODEL")) else {
        eprintln!("skipping: BROWSE_LLAMA_SERVER / BROWSE_LLAMA_MODEL not set");
        return;
    };
    let mut cfg = SidecarConfig::chat(&bin, &model, "sidecar-test");
    cfg.ctx_size = 1024;
    cfg.gpu_layers = Some(0);
    let t = Instant::now();
    let mut side = LlamaSidecar::start(&cfg).await.expect("sidecar starts");
    eprintln!("sidecar ready in {:?} on port {}", t.elapsed(), side.port());
    assert!(side.healthy().await);
    assert!(side.is_running());

    let provider = OpenAiCompatProvider::new(
        Box::new((*side.transport()).clone()),
        HashMap::from([(ModelTier::Fast, "sidecar-test".to_string())]),
    );
    let mut req = ModelRequest::new(ModelTier::Fast, Sensitivity::Public, vec![Message::user("Say hi.")]);
    req.max_tokens = Some(4);
    let r = model_gateway::ModelProvider::chat(&provider, &req).await.unwrap();
    assert!(!r.content.is_empty());

    let port = side.port();
    side.stop().await;
    let probe = HttpTransport::new(format!("http://127.0.0.1:{port}"));
    assert!(probe.get_json("/health").await.is_err(), "process must be gone after stop()");

    // Pool: apply a plan, reap, stop.
    let mut pool = SidecarPool::new();
    pool.register("fast", cfg.clone());
    pool.apply(&LoadPlan { unload: vec![], load: Some("fast".into()) }).await.unwrap();
    assert_eq!(pool.running_ids(), vec!["fast".to_string()]);
    assert_eq!(pool.transport().models(), vec!["sidecar-test".to_string()]);
    assert!(pool.reap().is_empty());
    pool.apply(&LoadPlan { unload: vec!["fast".into()], load: None }).await.unwrap();
    assert!(pool.running_ids().is_empty());
    assert!(pool.transport().models().is_empty());
}
