//! Sensitivity-aware routing (ADR-007).
//!
//! Rules, evaluated in order and deterministically:
//!
//! 1. `Secret` → refused. Secrets must be redacted before reaching the gateway.
//! 2. `offline_mode` → only local providers are candidates.
//! 3. `Private` → local only.
//! 4. `Personal` → local; cloud only if the class is allowed by config *and*
//!    the request carries an explicit `cloud_opt_in`.
//! 5. `Public` → local preferred; cloud if allowed and either the caller asked
//!    for it (`cloud_opt_in`) or no local provider serves the tier.
//!
//! The router never looks at message content.

use std::sync::Arc;

use core_types::{Locality, ModelTier, Sensitivity};
use serde::{Deserialize, Serialize};

use crate::{ModelError, ModelProvider, ModelRequest, ModelResponse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterConfig {
    pub offline_mode: bool,
    /// Mirrors `PolicyConfig::cloud_allowed_classes`.
    pub cloud_allowed_classes: Vec<Sensitivity>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self { offline_mode: false, cloud_allowed_classes: vec![Sensitivity::Public] }
    }
}

/// Why a particular provider was chosen. Recorded into `model_calls`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub provider: String,
    pub locality: Locality,
    pub reason: &'static str,
}

pub struct Router {
    providers: Vec<Arc<dyn ModelProvider>>,
    config: RouterConfig,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        Self { providers: vec![], config }
    }

    pub fn with_provider(mut self, provider: Arc<dyn ModelProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    pub fn set_offline(&mut self, offline: bool) {
        self.config.offline_mode = offline;
    }

    fn first_with(&self, locality: Locality, tier: ModelTier) -> Option<&Arc<dyn ModelProvider>> {
        self.providers.iter().find(|p| {
            let info = p.info();
            info.locality == locality && info.supports(tier)
        })
    }

    /// Choose a provider without calling it.
    pub fn route(&self, request: &ModelRequest) -> Result<(Arc<dyn ModelProvider>, RouteDecision), ModelError> {
        if request.sensitivity == Sensitivity::Secret {
            return Err(ModelError::Refused("secret data must never reach a model".into()));
        }

        let local = self.first_with(Locality::Local, request.tier);
        let cloud_permitted = !self.config.offline_mode
            && request.sensitivity.cloud_eligible(&self.config.cloud_allowed_classes)
            && match request.sensitivity {
                Sensitivity::Public => true,
                Sensitivity::Personal => request.cloud_opt_in,
                _ => false,
            };
        let cloud = if cloud_permitted { self.first_with(Locality::Cloud, request.tier) } else { None };

        let decided = match (local, cloud) {
            (Some(l), Some(c)) => {
                if request.cloud_opt_in {
                    (c, "cloud: user opt-in")
                } else {
                    (l, "local preferred")
                }
            }
            (Some(l), None) => (l, if self.config.offline_mode { "local: offline mode" } else { "local only" }),
            (None, Some(c)) => (c, "cloud: no local provider for tier"),
            (None, None) => {
                let why = if self.config.offline_mode {
                    "offline mode and no local provider for tier"
                } else if request.sensitivity >= Sensitivity::Private {
                    "private data and no local provider for tier"
                } else {
                    "no provider for tier"
                };
                return Err(ModelError::NoRoute(format!("{why} {:?}", request.tier)));
            }
        };

        let info = decided.0.info();
        Ok((decided.0.clone(), RouteDecision { provider: info.name, locality: info.locality, reason: decided.1 }))
    }

    pub async fn chat(&self, request: &ModelRequest) -> Result<(ModelResponse, RouteDecision), ModelError> {
        let (provider, decision) = self.route(request)?;
        let response = provider.chat(request).await?;
        Ok((response, decision))
    }

    /// Embeddings are always computed locally: the embedder is small and the
    /// texts are page content whose class is not known cheaply per chunk.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
        let provider = self
            .first_with(Locality::Local, ModelTier::Embed)
            .ok_or_else(|| ModelError::NoRoute("no local embedding provider".into()))?;
        provider.embed(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ProviderInfo};

    struct Stub {
        info: ProviderInfo,
    }

    #[async_trait::async_trait]
    impl ModelProvider for Stub {
        fn info(&self) -> ProviderInfo {
            self.info.clone()
        }
        async fn chat(&self, _r: &ModelRequest) -> Result<ModelResponse, ModelError> {
            Ok(ModelResponse {
                content: "ok".into(),
                tool_calls: vec![],
                usage: Default::default(),
                provider: self.info.name.clone(),
                model: "stub".into(),
                locality: self.info.locality,
            })
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
    }

    fn local() -> Arc<dyn ModelProvider> {
        Arc::new(Stub {
            info: ProviderInfo {
                name: "llama-server".into(),
                locality: Locality::Local,
                tiers: vec![ModelTier::Fast, ModelTier::Smart, ModelTier::Embed],
            },
        })
    }

    fn cloud() -> Arc<dyn ModelProvider> {
        Arc::new(Stub {
            info: ProviderInfo {
                name: "openai-compatible".into(),
                locality: Locality::Cloud,
                tiers: vec![ModelTier::Fast, ModelTier::Smart, ModelTier::Vision],
            },
        })
    }

    fn req(tier: ModelTier, s: Sensitivity, opt_in: bool) -> ModelRequest {
        let mut r = ModelRequest::new(tier, s, vec![Message::user("hi")]);
        r.cloud_opt_in = opt_in;
        r
    }

    #[test]
    fn secret_is_refused_everywhere() {
        let router = Router::new(RouterConfig::default()).with_provider(local()).with_provider(cloud());
        let err = router.route(&req(ModelTier::Fast, Sensitivity::Secret, true)).err().expect("expected error");
        assert!(matches!(err, ModelError::Refused(_)));
    }

    #[test]
    fn private_never_goes_to_cloud_even_when_config_lists_it() {
        let cfg = RouterConfig {
            offline_mode: false,
            cloud_allowed_classes: vec![Sensitivity::Public, Sensitivity::Personal, Sensitivity::Private],
        };
        let router = Router::new(cfg).with_provider(cloud()).with_provider(local());
        let (_, d) = router.route(&req(ModelTier::Smart, Sensitivity::Private, true)).unwrap();
        assert_eq!(d.locality, Locality::Local);

        // Vision is cloud-only in this setup → Private request has no route.
        let err = router.route(&req(ModelTier::Vision, Sensitivity::Private, true)).err().expect("expected error");
        assert!(matches!(err, ModelError::NoRoute(_)));
    }

    #[test]
    fn personal_requires_config_and_opt_in() {
        let cfg = RouterConfig {
            offline_mode: false,
            cloud_allowed_classes: vec![Sensitivity::Public, Sensitivity::Personal],
        };
        let router = Router::new(cfg).with_provider(local()).with_provider(cloud());
        let (_, d) = router.route(&req(ModelTier::Smart, Sensitivity::Personal, false)).unwrap();
        assert_eq!(d.locality, Locality::Local);
        let (_, d) = router.route(&req(ModelTier::Smart, Sensitivity::Personal, true)).unwrap();
        assert_eq!(d.locality, Locality::Cloud);

        let strict = Router::new(RouterConfig::default()).with_provider(local()).with_provider(cloud());
        let (_, d) = strict.route(&req(ModelTier::Smart, Sensitivity::Personal, true)).unwrap();
        assert_eq!(d.locality, Locality::Local, "opt-in alone is not enough");
    }

    #[test]
    fn public_prefers_local_and_falls_back_to_cloud_for_missing_tier() {
        let router = Router::new(RouterConfig::default()).with_provider(local()).with_provider(cloud());
        let (_, d) = router.route(&req(ModelTier::Fast, Sensitivity::Public, false)).unwrap();
        assert_eq!(d.locality, Locality::Local);
        let (_, d) = router.route(&req(ModelTier::Vision, Sensitivity::Public, false)).unwrap();
        assert_eq!(d.locality, Locality::Cloud);
        assert_eq!(d.reason, "cloud: no local provider for tier");
    }

    #[test]
    fn offline_mode_blocks_cloud_completely() {
        let cfg = RouterConfig { offline_mode: true, cloud_allowed_classes: vec![Sensitivity::Public] };
        let router = Router::new(cfg).with_provider(local()).with_provider(cloud());
        let (_, d) = router.route(&req(ModelTier::Fast, Sensitivity::Public, true)).unwrap();
        assert_eq!(d.locality, Locality::Local);
        assert!(router.route(&req(ModelTier::Vision, Sensitivity::Public, true)).is_err());
    }

    #[tokio::test]
    async fn embed_is_local_only() {
        let router = Router::new(RouterConfig::default()).with_provider(cloud());
        assert!(router.embed(&["a".into()]).await.is_err());
        let router = router.with_provider(local());
        assert_eq!(router.embed(&["a".into(), "b".into()]).await.unwrap().len(), 2);
    }
}
