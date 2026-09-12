//! VRAM budget accounting for the local runtime (ADR-003 §"Budget").
//!
//! Invariant on the reference hardware (RTX 4060, 8 GiB):
//!
//! ```text
//! resident(fast + embed) + reserve(compositor, 1.5 GiB) + loaded(non-resident) ≤ budget
//! ```
//!
//! Non-resident models (`smart`, `vision`, ASR) are LRU-swapped. The manager is
//! a pure planner: it decides what to load/unload and reports it; the caller
//! performs the actual `llama-server` `/models/load`/`/models/unload` calls.

use std::collections::HashMap;

use core_types::ModelTier;
use serde::{Deserialize, Serialize};

use crate::ModelError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub tier: ModelTier,
    /// Weights + KV cache at the configured context, measured, MiB.
    pub vram_mb: u32,
    /// Resident models are never evicted (the embedder). `fast` is *not*
    /// resident: `smart` is allowed to evict it (ADR-003).
    pub resident: bool,
    /// Loaded at startup (`fast`, `embed`) so the first answer is fast.
    pub warm_on_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub total_vram_mb: u32,
    /// Kept free for the Chromium compositor / GPU rasterization.
    pub reserve_mb: u32,
}

impl BudgetConfig {
    /// RTX 4060 8 GiB reference profile.
    pub const RTX_4060: BudgetConfig = BudgetConfig { total_vram_mb: 8192, reserve_mb: 1536 };

    pub fn usable_mb(&self) -> u32 {
        self.total_vram_mb.saturating_sub(self.reserve_mb)
    }
}

/// What the caller must do to satisfy an `ensure_loaded` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadPlan {
    pub unload: Vec<String>,
    pub load: Option<String>,
}

pub struct ModelManager {
    budget: BudgetConfig,
    specs: HashMap<String, ModelSpec>,
    /// id → last-used tick (monotonic counter).
    loaded: HashMap<String, u64>,
    tick: u64,
}

impl ModelManager {
    pub fn new(budget: BudgetConfig) -> Self {
        Self { budget, specs: HashMap::new(), loaded: HashMap::new(), tick: 0 }
    }

    /// Register a model. Returns an error if resident models alone exceed the budget.
    pub fn register(&mut self, spec: ModelSpec) -> Result<(), ModelError> {
        let resident_total: u32 = self.specs.values().filter(|s| s.resident).map(|s| s.vram_mb).sum::<u32>()
            + if spec.resident { spec.vram_mb } else { 0 };
        if resident_total > self.budget.usable_mb() {
            return Err(ModelError::OutOfBudget { need_mb: resident_total, available_mb: self.budget.usable_mb() });
        }
        self.specs.insert(spec.id.clone(), spec);
        Ok(())
    }

    pub fn loaded_mb(&self) -> u32 {
        self.loaded.keys().filter_map(|id| self.specs.get(id)).map(|s| s.vram_mb).sum()
    }

    pub fn free_mb(&self) -> u32 {
        self.budget.usable_mb().saturating_sub(self.loaded_mb())
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.loaded.contains_key(id)
    }

    pub fn loaded_ids(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.loaded.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    fn touch(&mut self, id: &str) {
        self.tick += 1;
        if let Some(t) = self.loaded.get_mut(id) {
            *t = self.tick;
        }
    }

    /// First registered model for a tier (callers register one per tier).
    pub fn model_for(&self, tier: ModelTier) -> Option<&ModelSpec> {
        let mut candidates: Vec<&ModelSpec> = self.specs.values().filter(|s| s.tier == tier).collect();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        candidates.first().copied()
    }

    /// Plan loading `id`, evicting least-recently-used non-resident models if
    /// needed. Applies the plan to internal state and returns it.
    pub fn ensure_loaded(&mut self, id: &str) -> Result<LoadPlan, ModelError> {
        let spec = self.specs.get(id).cloned().ok_or_else(|| ModelError::NoRoute(format!("unknown model {id}")))?;

        if self.loaded.contains_key(id) {
            self.touch(id);
            return Ok(LoadPlan::default());
        }

        if spec.vram_mb > self.budget.usable_mb() {
            return Err(ModelError::OutOfBudget { need_mb: spec.vram_mb, available_mb: self.budget.usable_mb() });
        }

        // Plan on a scratch copy so a failed load leaves the loaded set untouched.
        let mut scratch = self.loaded.clone();
        let mut plan = LoadPlan::default();
        let used = |l: &HashMap<String, u64>| -> u32 { l.keys().map(|k| self.specs[k].vram_mb).sum() };
        while self.budget.usable_mb().saturating_sub(used(&scratch)) < spec.vram_mb {
            let victim = scratch
                .iter()
                .filter(|(vid, _)| !self.specs[*vid].resident)
                .min_by_key(|(_, tick)| **tick)
                .map(|(vid, _)| vid.clone());
            match victim {
                Some(v) => {
                    scratch.remove(&v);
                    plan.unload.push(v);
                }
                None => {
                    return Err(ModelError::OutOfBudget {
                        need_mb: spec.vram_mb,
                        available_mb: self.budget.usable_mb().saturating_sub(used(&scratch)),
                    });
                }
            }
        }

        self.tick += 1;
        scratch.insert(id.to_string(), self.tick);
        self.loaded = scratch;
        plan.load = Some(id.to_string());
        Ok(plan)
    }

    /// Convenience: ensure the model registered for `tier` is loaded.
    pub fn ensure_tier(&mut self, tier: ModelTier) -> Result<(String, LoadPlan), ModelError> {
        let id = self
            .model_for(tier)
            .map(|s| s.id.clone())
            .ok_or_else(|| ModelError::NoRoute(format!("no model registered for tier {tier:?}")))?;
        let plan = self.ensure_loaded(&id)?;
        Ok((id, plan))
    }

    /// Load every `warm_on_start` model at startup, residents first.
    pub fn warm(&mut self) -> Result<Vec<String>, ModelError> {
        let mut specs: Vec<&ModelSpec> = self.specs.values().filter(|s| s.warm_on_start || s.resident).collect();
        specs.sort_by_key(|s| (!s.resident, s.id.clone()));
        let ids: Vec<String> = specs.into_iter().map(|s| s.id.clone()).collect();
        for id in &ids {
            self.ensure_loaded(id)?;
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, tier: ModelTier, mb: u32, resident: bool, warm: bool) -> ModelSpec {
        ModelSpec { id: id.into(), tier, vram_mb: mb, resident, warm_on_start: warm }
    }

    /// ADR-003 default preset for RTX 4060.
    fn reference_manager() -> ModelManager {
        let mut m = ModelManager::new(BudgetConfig::RTX_4060);
        m.register(spec("qwen3-4b-q4", ModelTier::Fast, 3500, false, true)).unwrap();
        m.register(spec("embeddinggemma-300m", ModelTier::Embed, 400, true, true)).unwrap();
        m.register(spec("qwen3-8b-q4", ModelTier::Smart, 6200, false, false)).unwrap();
        m.register(spec("gemma-4-e4b", ModelTier::Vision, 4200, false, false)).unwrap();
        m
    }

    #[test]
    fn warm_set_fits_the_reference_budget() {
        let mut m = reference_manager();
        let warmed = m.warm().unwrap();
        assert_eq!(warmed, vec!["embeddinggemma-300m".to_string(), "qwen3-4b-q4".to_string()]);
        assert_eq!(m.loaded_mb(), 3900);
        assert!(m.free_mb() >= 2000, "compositor reserve plus headroom");
    }

    #[test]
    fn residents_over_budget_are_rejected_at_registration() {
        let mut m = ModelManager::new(BudgetConfig::RTX_4060);
        m.register(spec("a", ModelTier::Fast, 4000, true, true)).unwrap();
        let err = m.register(spec("b", ModelTier::Embed, 3000, true, true)).unwrap_err();
        assert!(matches!(err, ModelError::OutOfBudget { .. }));
    }

    #[test]
    fn smart_evicts_fast_but_never_the_resident_embedder() {
        let mut m = reference_manager();
        m.warm().unwrap();

        // usable = 8192 - 1536 = 6656; embed 400 + smart 6200 = 6600 fits only after evicting fast.
        let plan = m.ensure_loaded("qwen3-8b-q4").unwrap();
        assert_eq!(plan.unload, vec!["qwen3-4b-q4".to_string()]);
        assert_eq!(m.loaded_ids(), vec!["embeddinggemma-300m", "qwen3-8b-q4"]);

        // A model larger than usable minus residents can never be loaded.
        m.register(spec("huge", ModelTier::Smart, 6500, false, false)).unwrap();
        let err = m.ensure_loaded("huge").unwrap_err();
        assert!(matches!(err, ModelError::OutOfBudget { .. }));
        assert!(m.is_loaded("embeddinggemma-300m"), "residents survive a failed load");
        assert!(m.is_loaded("qwen3-8b-q4"), "a failed load must not evict anything");
    }

    #[test]
    fn lru_swap_between_non_residents() {
        let mut m = ModelManager::new(BudgetConfig { total_vram_mb: 8192, reserve_mb: 1536 });
        m.register(spec("embed", ModelTier::Embed, 400, true, true)).unwrap();
        m.register(spec("fast", ModelTier::Fast, 3500, false, false)).unwrap();
        m.register(spec("smart", ModelTier::Smart, 6000, false, false)).unwrap();
        m.register(spec("vision", ModelTier::Vision, 2500, false, false)).unwrap();
        m.warm().unwrap();

        assert_eq!(m.ensure_loaded("fast").unwrap(), LoadPlan { unload: vec![], load: Some("fast".into()) });
        assert_eq!(m.ensure_loaded("vision").unwrap(), LoadPlan { unload: vec![], load: Some("vision".into()) });
        // 400 + 3500 + 2500 = 6400 ≤ 6656. Now smart (6000) needs both evicted; `fast` is LRU first.
        let plan = m.ensure_loaded("smart").unwrap();
        assert_eq!(plan.unload, vec!["fast".to_string(), "vision".to_string()]);
        assert_eq!(plan.load, Some("smart".into()));
        assert_eq!(m.loaded_ids(), vec!["embed", "smart"]);

        // Touching keeps a model hot: re-request `smart`, then `fast` must evict `smart` anyway (only candidate).
        assert_eq!(m.ensure_loaded("smart").unwrap(), LoadPlan::default());
        let plan = m.ensure_loaded("fast").unwrap();
        assert_eq!(plan.unload, vec!["smart".to_string()]);
    }

    #[test]
    fn ensure_tier_resolves_registered_model() {
        let mut m = reference_manager();
        let (id, plan) = m.ensure_tier(ModelTier::Fast).unwrap();
        assert_eq!(id, "qwen3-4b-q4");
        assert_eq!(plan.load, Some("qwen3-4b-q4".into()));
        assert!(m.ensure_tier(ModelTier::Embed).is_ok());
    }
}
