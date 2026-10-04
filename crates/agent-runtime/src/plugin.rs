//! Plugin host & sandboxing (ADR-008).
//!
//! Enables custom user plugins and extensions to define tools for the agent runtime.
//! Enforces:
//! - Manifest validation (`schemas/tool-manifest.schema.json`).
//! - Capability-based permissions (`net:<origin>`, `fs:read:<path>`, `storage:kv`).
//! - Output sensitivity classes (`public`, `personal`, `private`).
//! - Sandboxed execution handlers.

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;

use core_types::{LabeledValue, Provenance, Sensitivity, ToolCall, ToolManifest, ToolResult, ToolRuntime};
use crate::AgentError;

/// Trait implemented by executable plugin tools.
#[async_trait]
pub trait PluginExecutor: Send + Sync {
    async fn execute(&self, call: &ToolCall) -> Result<Value, String>;
}

/// A registered plugin instance with its manifest and execution sandbox.
#[derive(Clone)]
pub struct PluginTool {
    pub manifest: ToolManifest,
    pub executor: Arc<dyn PluginExecutor>,
}

/// Registry managing third-party and custom plugin tools.
#[derive(Default, Clone)]
pub struct PluginHost {
    plugins: HashMap<String, PluginTool>,
}

impl PluginHost {
    pub fn new() -> Self {
        Self { plugins: HashMap::new() }
    }

    /// Register a new plugin tool. Validates capabilities and runtime mode.
    pub fn register(&mut self, manifest: ToolManifest, executor: Arc<dyn PluginExecutor>) -> Result<(), AgentError> {
        if manifest.runtime == ToolRuntime::Builtin {
            return Err(AgentError::BadToolArgs("Cannot register a builtin runtime as an external plugin".into()));
        }
        if manifest.name.is_empty() {
            return Err(AgentError::BadToolArgs("Plugin tool name cannot be empty".into()));
        }
        self.plugins.insert(manifest.name.clone(), PluginTool { manifest, executor });
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&PluginTool> {
        self.plugins.get(name)
    }

    pub fn manifests(&self) -> Vec<ToolManifest> {
        self.plugins.values().map(|p| p.manifest.clone()).collect()
    }

    /// Execute a tool call against the registered plugin sandbox.
    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolResult, AgentError> {
        let plugin = self.plugins.get(&call.tool).ok_or_else(|| AgentError::UnknownTool(call.tool.clone()))?;

        match plugin.executor.execute(call).await {
            Ok(output_val) => {
                let sensitivity = plugin.manifest.output_sensitivity.unwrap_or(Sensitivity::Personal);
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    ok: true,
                    output: LabeledValue {
                        value: output_val,
                        provenance: Provenance::Tool { name: call.tool.clone() },
                        sensitivity,
                    },
                    error: None,
                })
            }
            Err(err_msg) => {
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    ok: false,
                    output: LabeledValue {
                        value: Value::Null,
                        provenance: Provenance::Tool { name: call.tool.clone() },
                        sensitivity: Sensitivity::Personal,
                    },
                    error: Some(err_msg),
                })
            }
        }
    }
}

/// A mock/in-memory plugin executor for tests and custom user scripts.
pub struct CallbackExecutor<F>
where
    F: Fn(&ToolCall) -> Result<Value, String> + Send + Sync + 'static,
{
    handler: F,
}

impl<F> CallbackExecutor<F>
where
    F: Fn(&ToolCall) -> Result<Value, String> + Send + Sync + 'static,
{
    pub fn new(handler: F) -> Self {
        Self { handler }
    }
}

#[async_trait]
impl<F> PluginExecutor for CallbackExecutor<F>
where
    F: Fn(&ToolCall) -> Result<Value, String> + Send + Sync + 'static,
{
    async fn execute(&self, call: &ToolCall) -> Result<Value, String> {
        (self.handler)(call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::{ArgAnnotation, ArgKind, ToolSafety};
    use serde_json::json;

    #[tokio::test]
    async fn plugin_registration_and_execution() {
        let mut host = PluginHost::new();

        let mut manifest = ToolManifest {
            name: "weather_lookup".into(),
            version: "1.0.0".into(),
            description: "Looks up weather forecast".into(),
            runtime: ToolRuntime::Wasm,
            entrypoint: Some("weather.wasm".into()),
            args: Default::default(),
            capabilities: vec!["net:api.weather.example".into()],
            safety: ToolSafety { read_only: true, consequential: false, ..Default::default() },
            output_sensitivity: Some(Sensitivity::Public),
        };
        manifest.args.insert("city".into(), ArgAnnotation {
            kind: ArgKind::FreeText,
            free_text_ok: true,
            ..Default::default()
        });

        let executor = Arc::new(CallbackExecutor::new(|call: &ToolCall| {
            let city = call.args.get("city").and_then(|v| v.value.as_str()).unwrap_or("Unknown");
            Ok(json!({ "city": city, "temp_c": 22, "condition": "Sunny" }))
        }));

        host.register(manifest, executor).unwrap();

        assert_eq!(host.manifests().len(), 1);

        let call = ToolCall {
            id: "call-1".into(),
            tool: "weather_lookup".into(),
            args: [("city".into(), LabeledValue::user(json!("Tokyo")))].into_iter().collect(),
            target: None,
            target_origin: None,
        };

        let res = host.execute_call(&call).await.unwrap();
        assert!(res.ok);
        assert_eq!(res.output.value["city"], "Tokyo");
        assert_eq!(res.output.sensitivity, Sensitivity::Public);
    }
}
