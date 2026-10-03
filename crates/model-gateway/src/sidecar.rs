//! Lifecycle of local `llama-server` processes (ADR-003).
//!
//! One process per model, bound to `127.0.0.1` on an ephemeral port, health
//! checked before use, killed when dropped. [`SidecarPool`] executes the
//! [`LoadPlan`]s produced by [`crate::ModelManager`] and exposes a
//! [`RoutingTransport`] so one [`crate::OpenAiCompatProvider`] can address all
//! running models by alias.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::http::{HttpTransport, RoutingTransport};
use crate::{LoadPlan, ModelError};

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub binary: PathBuf,
    pub model_path: PathBuf,
    /// Model id clients use in requests (`--alias`).
    pub alias: String,
    /// `0` picks a free ephemeral port.
    pub port: u16,
    pub ctx_size: u32,
    /// Layers to offload to the GPU; `None` = all (`-ngl 99`).
    pub gpu_layers: Option<u32>,
    pub embeddings: bool,
    pub threads: Option<u32>,
    /// Parallel request slots; the KV cache is split between them.
    pub parallel: u32,
    pub extra_args: Vec<String>,
    pub startup_timeout: Duration,
}

impl SidecarConfig {
    pub fn chat(binary: impl Into<PathBuf>, model_path: impl Into<PathBuf>, alias: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
            model_path: model_path.into(),
            alias: alias.into(),
            port: 0,
            ctx_size: 8192,
            gpu_layers: None,
            embeddings: false,
            threads: None,
            parallel: 1,
            extra_args: vec![],
            startup_timeout: Duration::from_secs(120),
        }
    }

    pub fn embedding(binary: impl Into<PathBuf>, model_path: impl Into<PathBuf>, alias: impl Into<String>) -> Self {
        let mut c = Self::chat(binary, model_path, alias);
        c.embeddings = true;
        c.ctx_size = 2048;
        c.parallel = 2;
        c
    }

    fn args(&self, port: u16) -> Vec<String> {
        let mut a = vec![
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "-m".into(),
            self.model_path.to_string_lossy().into_owned(),
            "--alias".into(),
            self.alias.clone(),
            "-c".into(),
            self.ctx_size.to_string(),
            "-ngl".into(),
            self.gpu_layers.unwrap_or(99).to_string(),
            "-np".into(),
            self.parallel.to_string(),
            // Chat templates and tool calls need the Jinja renderer.
            "--jinja".into(),
            // No request logging: prompts contain page content.
            "--log-disable".into(),
        ];
        if self.embeddings {
            a.push("--embeddings".into());
            a.push("--pooling".into());
            a.push("mean".into());
        }
        if let Some(t) = self.threads {
            a.push("-t".into());
            a.push(t.to_string());
        }
        a.extend(self.extra_args.iter().cloned());
        a
    }
}

/// Locate `llama-server`: `BROWSE_LLAMA_SERVER`, then `PATH`.
pub fn find_llama_server() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("BROWSE_LLAMA_SERVER") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    which::which("llama-server").ok()
}

pub struct LlamaSidecar {
    child: Child,
    alias: String,
    port: u16,
    transport: Arc<HttpTransport>,
    started: Instant,
}

impl LlamaSidecar {
    /// Spawn the process and wait until `/health` reports `ok` (the model is loaded).
    pub async fn start(cfg: &SidecarConfig) -> Result<LlamaSidecar, ModelError> {
        if !cfg.binary.is_file() {
            return Err(launch_err(format!("llama-server binary not found at {}", cfg.binary.display())));
        }
        if !cfg.model_path.is_file() {
            return Err(launch_err(format!("model file not found at {}", cfg.model_path.display())));
        }
        let port = if cfg.port == 0 { free_port()? } else { cfg.port };
        let mut child = Command::new(&cfg.binary)
            .args(cfg.args(port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| launch_err(format!("spawn {}: {e}", cfg.binary.display())))?;

        // Keep the last stderr lines for the error message if startup fails.
        let stderr = child.stderr.take();
        let tail: Arc<std::sync::Mutex<std::collections::VecDeque<String>>> = Default::default();
        if let Some(stderr) = stderr {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::trace!(target: "llama-server", "{line}");
                    let mut t = tail.lock().unwrap();
                    if t.len() >= 20 {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            });
        }

        let transport = Arc::new(HttpTransport::new(format!("http://127.0.0.1:{port}")).with_name("llama-server"));
        let started = Instant::now();
        let deadline = started + cfg.startup_timeout;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let log = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                return Err(launch_err(format!("llama-server exited during startup ({status}):\n{log}")));
            }
            if health_ok(&transport).await {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.start_kill();
                return Err(ModelError::Timeout(cfg.startup_timeout.as_millis() as u64));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        tracing::info!(alias = %cfg.alias, port, ms = started.elapsed().as_millis() as u64, "llama-server ready");
        Ok(LlamaSidecar { child, alias: cfg.alias.clone(), port, transport, started })
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn transport(&self) -> Arc<HttpTransport> {
        self.transport.clone()
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    pub async fn healthy(&self) -> bool {
        health_ok(&self.transport).await
    }

    /// `true` while the process has not exited.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn stop(mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}

async fn health_ok(t: &HttpTransport) -> bool {
    matches!(t.get_json("/health").await, Ok(v) if v["status"] == "ok")
}

fn launch_err(message: String) -> ModelError {
    ModelError::Provider { provider: "llama-server".into(), message }
}

fn free_port() -> Result<u16, ModelError> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| launch_err(format!("no free loopback port: {e}")))
}

/// Runs the processes a [`crate::ModelManager`] plan asks for.
pub struct SidecarPool {
    configs: HashMap<String, SidecarConfig>,
    running: HashMap<String, LlamaSidecar>,
    routing: Arc<RoutingTransport>,
}

impl Default for SidecarPool {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarPool {
    pub fn new() -> Self {
        Self { configs: HashMap::new(), running: HashMap::new(), routing: Arc::new(RoutingTransport::new()) }
    }

    /// Register how to run model `id` (the same id used in `ModelSpec`).
    pub fn register(&mut self, id: impl Into<String>, cfg: SidecarConfig) {
        self.configs.insert(id.into(), cfg);
    }

    /// Transport that dispatches by model alias to whichever process serves it.
    pub fn transport(&self) -> Arc<RoutingTransport> {
        self.routing.clone()
    }

    pub fn running_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.running.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    /// Execute a plan: unloads first (to free VRAM), then the load.
    pub async fn apply(&mut self, plan: &LoadPlan) -> Result<(), ModelError> {
        for id in &plan.unload {
            if let Some(s) = self.running.remove(id) {
                self.routing.remove(s.alias());
                s.stop().await;
            }
        }
        if let Some(id) = &plan.load {
            if self.running.contains_key(id) {
                return Ok(());
            }
            let cfg = self
                .configs
                .get(id)
                .ok_or_else(|| ModelError::NoRoute(format!("no sidecar config registered for model `{id}`")))?
                .clone();
            let s = LlamaSidecar::start(&cfg).await?;
            self.routing.set(s.alias(), s.transport());
            self.running.insert(id.clone(), s);
        }
        Ok(())
    }

    /// Drop processes that died (crash, OOM) so the manager can reload them.
    pub fn reap(&mut self) -> Vec<String> {
        let mut dead: Vec<String> = vec![];
        for (id, s) in self.running.iter_mut() {
            if !s.is_running() {
                dead.push(id.clone());
            }
        }
        for id in &dead {
            if let Some(s) = self.running.remove(id) {
                self.routing.remove(s.alias());
            }
        }
        dead
    }

    pub async fn stop_all(&mut self) {
        for (_, s) in self.running.drain() {
            self.routing.remove(s.alias());
            s.stop().await;
        }
    }
}

/// Default model files under a models directory, per ADR-003's 8 GB preset.
pub fn preset_paths(models_dir: &Path) -> HashMap<&'static str, PathBuf> {
    HashMap::from([
        ("fast", models_dir.join("qwen3-4b-instruct-q4_k_m.gguf")),
        ("smart", models_dir.join("qwen3-8b-instruct-q4_k_m.gguf")),
        ("embed", models_dir.join("embeddinggemma-300m-Q8_0.gguf")),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_include_security_relevant_flags() {
        let cfg = SidecarConfig::embedding("/bin/llama-server", "/m.gguf", "emb");
        let a = cfg.args(1234).join(" ");
        assert!(a.contains("--host 127.0.0.1"), "must bind loopback only");
        assert!(a.contains("--port 1234"));
        assert!(a.contains("--log-disable"), "prompts must not be logged");
        assert!(a.contains("--embeddings --pooling mean"));
        assert!(a.contains("--alias emb"));
        let cfg = SidecarConfig::chat("/bin/llama-server", "/m.gguf", "chat");
        assert!(!cfg.args(1).join(" ").contains("--embeddings"));
    }

    #[tokio::test]
    async fn missing_binary_or_model_fails_fast() {
        let cfg = SidecarConfig::chat("/nonexistent/llama-server", "/nonexistent.gguf", "x");
        let err = LlamaSidecar::start(&cfg).await.err().unwrap();
        assert!(matches!(err, ModelError::Provider { .. }));
        let mut pool = SidecarPool::new();
        let err = pool.apply(&LoadPlan { unload: vec![], load: Some("ghost".into()) }).await.err().unwrap();
        assert!(matches!(err, ModelError::NoRoute(_)));
    }
}
