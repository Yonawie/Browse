//! Model Gateway configuration for the desktop binary.
//!
//! Until the settings UI exists (stage 9) the configuration comes from the
//! environment:
//!
//! ```text
//! BROWSE_LLAMA_CHAT_URL   http://127.0.0.1:8081   running llama-server for the fast/smart tiers
//! BROWSE_LLAMA_EMBED_URL  http://127.0.0.1:8082   running llama-server started with --embeddings
//! BROWSE_LLAMA_SERVER     path to llama-server    spawn sidecars instead (with BROWSE_MODELS_DIR)
//! BROWSE_MODELS_DIR       directory with the GGUF files of the ADR-003 preset
//! BROWSE_CLOUD_BASE_URL   https://api.example.com/v1   optional OpenAI-compatible cloud endpoint
//! BROWSE_CLOUD_API_KEY    key for that endpoint (sent as a bearer token, never logged)
//! BROWSE_CLOUD_MODEL      model name at that endpoint
//! BROWSE_OFFLINE=1        never route to the cloud regardless of the above
//! ```
//!
//! Loopback servers are addressed by whatever alias they announce on
//! `/v1/models`, so the exact model file does not matter.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core_types::{Locality, ModelTier, Sensitivity};
use memory::{MemoryStore, ModelCallRecord};
use model_gateway::sidecar::{find_llama_server, LlamaSidecar, SidecarConfig};
use model_gateway::{
    CallLog, CallRecord, Gateway, HttpTransport, MemoryCache, Message, ModelRequest, OpenAiCompatProvider, Router,
    RouterConfig, StreamEvent,
};

/// `CallLog` writing to `model_calls` in the memory database.
pub struct SqliteCallLog(pub Arc<Mutex<MemoryStore>>);

impl CallLog for SqliteCallLog {
    fn record(&self, c: CallRecord) {
        let rec = ModelCallRecord {
            session_id: None,
            purpose: c.purpose,
            provider: c.provider,
            model: c.model,
            locality: match c.locality {
                Locality::Local => "local".into(),
                Locality::Cloud => "cloud".into(),
            },
            sensitivity: match c.sensitivity {
                Sensitivity::Public => "public".into(),
                Sensitivity::Personal => "personal".into(),
                _ => "private".into(),
            },
            input_tokens: Some(c.input_tokens as i64),
            output_tokens: Some(c.output_tokens as i64),
            latency_ms: Some(c.latency_ms as i64),
            cost_usd: None,
            cache_hit: c.cache_hit,
            created_at: Some(c.created_at),
        };
        if let Err(e) = self.0.lock().unwrap().record_model_call(&rec) {
            eprintln!("model_calls: {e}");
        }
    }
}

pub struct ConfiguredGateway {
    pub gateway: Gateway,
    pub summary: Vec<String>,
    /// Sidecars spawned by us; dropping them stops the processes.
    pub sidecars: Vec<LlamaSidecar>,
}

async fn alias_of(t: &HttpTransport) -> Option<String> {
    let v = t.get_json("/v1/models").await.ok()?;
    v["data"][0]["id"].as_str().map(str::to_string)
}

/// Build a gateway from the environment. Missing local servers are reported,
/// not fatal: the router simply has no local provider for those tiers.
pub async fn from_env(store: Option<Arc<Mutex<MemoryStore>>>) -> ConfiguredGateway {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut summary = vec![];
    let mut sidecars = vec![];
    let offline = env("BROWSE_OFFLINE").is_some();
    let mut router = Router::new(RouterConfig { offline_mode: offline, ..RouterConfig::default() });

    // Local chat tiers.
    let mut chat_alias: Option<(HttpTransport, String)> = None;
    if let Some(url) = env("BROWSE_LLAMA_CHAT_URL") {
        let t = HttpTransport::new(&url).with_name("llama-server").with_timeout(Duration::from_secs(600));
        match alias_of(&t).await {
            Some(a) => chat_alias = Some((t, a)),
            None => summary.push(format!("local chat: {url} not reachable")),
        }
    } else if let (Some(bin), Some(dir)) = (find_llama_server(), env("BROWSE_MODELS_DIR")) {
        let preset = model_gateway::sidecar::preset_paths(std::path::Path::new(&dir));
        if let Some(path) = preset.get("fast").filter(|p| p.is_file()) {
            match LlamaSidecar::start(&SidecarConfig::chat(&bin, path, "fast")).await {
                Ok(s) => {
                    chat_alias = Some(((*s.transport()).clone(), "fast".into()));
                    summary.push(format!("local chat: spawned {} on port {}", path.display(), s.port()));
                    sidecars.push(s);
                }
                Err(e) => summary.push(format!("local chat: sidecar failed: {e}")),
            }
        }
    }
    if let Some((t, alias)) = chat_alias {
        summary.push(format!("local chat: {} serves `{alias}` (fast, smart)", t.base_url()));
        let models = HashMap::from([(ModelTier::Fast, alias.clone()), (ModelTier::Smart, alias)]);
        router = router.with_provider(Arc::new(OpenAiCompatProvider::new(Box::new(t), models)));
    } else if summary.is_empty() {
        summary.push("local chat: not configured (set BROWSE_LLAMA_CHAT_URL)".into());
    }

    // Local embedder.
    if let Some(url) = env("BROWSE_LLAMA_EMBED_URL") {
        let t = HttpTransport::new(&url).with_name("llama-server-embed");
        match alias_of(&t).await {
            Some(alias) => {
                summary.push(format!("local embed: {url} serves `{alias}`"));
                let models = HashMap::from([(ModelTier::Embed, alias)]);
                router = router
                    .with_provider(Arc::new(OpenAiCompatProvider::new(Box::new(t), models).with_name("llama-embed")));
            }
            None => summary.push(format!("local embed: {url} not reachable")),
        }
    }

    // Optional cloud endpoint.
    if let (Some(base), Some(key), Some(model)) =
        (env("BROWSE_CLOUD_BASE_URL"), env("BROWSE_CLOUD_API_KEY"), env("BROWSE_CLOUD_MODEL"))
    {
        let t = HttpTransport::new(&base).with_api_key(key).with_name("cloud");
        let models = HashMap::from([(ModelTier::Fast, model.clone()), (ModelTier::Smart, model.clone())]);
        router = router.with_provider(Arc::new(OpenAiCompatProvider::cloud("cloud", Box::new(t), models)));
        summary.push(format!("cloud: {base} model `{model}` (Public only; Personal needs per-request opt-in)"));
    }
    if offline {
        summary.push("offline mode: cloud routing disabled".into());
    }

    let mut gateway = Gateway::new(router).with_cache(Arc::new(MemoryCache::default()));
    if let Some(store) = store {
        gateway = gateway.with_log(Arc::new(SqliteCallLog(store)));
    }
    ConfiguredGateway { gateway, summary, sidecars }
}

/// `browse-desktop models`: show configuration, routing per sensitivity class
/// and a streamed smoke prompt.
pub async fn models_command() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(Mutex::new(MemoryStore::open_in_memory()?));
    let cfg = from_env(Some(store.clone())).await;
    println!("Model Gateway");
    for line in &cfg.summary {
        println!("  {line}");
    }
    println!("Routing (tier=fast):");
    for s in [Sensitivity::Public, Sensitivity::Personal, Sensitivity::Private, Sensitivity::Secret] {
        let mut req = ModelRequest::new(ModelTier::Fast, s, vec![Message::user("x")]);
        req.cloud_opt_in = true;
        match cfg.gateway.router().route(&req) {
            Ok((_, d)) => println!("  {s:<9?} → {} ({:?}, {})", d.provider, d.locality, d.reason),
            Err(e) => println!("  {s:<9?} → {e}"),
        }
    }

    let mut req = ModelRequest::new(
        ModelTier::Fast,
        Sensitivity::Private,
        vec![Message::system("Reply in one short sentence."), Message::user("Say hello to a developer.")],
    );
    req.max_tokens = Some(32);
    match cfg.gateway.chat_stream("smoke", &req).await {
        Ok((mut stream, decision)) => {
            use futures_util::StreamExt;
            print!("Smoke prompt via {}: ", decision.provider);
            while let Some(ev) = stream.next().await {
                match ev? {
                    StreamEvent::Delta(d) => {
                        print!("{d}");
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    }
                    StreamEvent::Done(r) => println!(
                        "\n  [{} tokens in, {} out, model {}]",
                        r.usage.prompt_tokens, r.usage.completion_tokens, r.model
                    ),
                }
            }
        }
        Err(e) => println!("Smoke prompt: {e}"),
    }
    let usage = store.lock().unwrap().model_usage()?;
    for u in usage {
        println!(
            "model_calls: {} {} {} calls={} cache_hits={}",
            u.provider, u.model, u.locality, u.calls, u.cache_hits
        );
    }
    if !cfg.sidecars.is_empty() {
        println!("stopping {} sidecar(s)", cfg.sidecars.len());
        for s in cfg.sidecars {
            s.stop().await;
        }
    }
    Ok(())
}
