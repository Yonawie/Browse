use core_types::ModelTier;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A recommended local model in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub name: String,
    pub tier: ModelTier,
    pub filename: String,
    pub download_url: String,
    pub size_bytes: u64,
    pub context_size: u32,
    pub recommended_for: String,
    pub sha256: Option<String>,
}

/// Installation status of a catalog model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelInstallStatus {
    Installed {
        path: String,
        size_bytes: u64,
    },
    NotInstalled {
        expected_size_bytes: u64,
        filename: String,
    },
}

/// Built-in catalog of curated, high-efficiency local models for Browse.
pub fn get_recommended_catalog() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "qwen-2.5-3b-instruct".into(),
            name: "Qwen 2.5 3B Instruct (Q4_K_M)".into(),
            tier: ModelTier::Fast,
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            size_bytes: 2_150_000_000,
            context_size: 8192,
            recommended_for: "Fast local summarization, citation verification, and daily browsing intelligence".into(),
            sha256: None,
        },
        ModelCatalogEntry {
            id: "llama-3.2-3b-instruct".into(),
            name: "Llama 3.2 3B Instruct (Q4_K_M)".into(),
            tier: ModelTier::Fast,
            filename: "llama-3.2-3b-instruct-q4_k_m.gguf".into(),
            download_url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf".into(),
            size_bytes: 2_020_000_000,
            context_size: 8192,
            recommended_for: "Alternative lightweight fast chat and browser automation model".into(),
            sha256: None,
        },
        ModelCatalogEntry {
            id: "bge-small-en-v1.5".into(),
            name: "BGE Small EN v1.5 (Q8_0)".into(),
            tier: ModelTier::Embed,
            filename: "bge-small-en-v1.5-q8_0.gguf".into(),
            download_url: "https://huggingface.co/CompendiumLabs/bge-small-en-v1.5-gguf/resolve/main/bge-small-en-v1.5-q8_0.gguf".into(),
            size_bytes: 35_000_000,
            context_size: 2048,
            recommended_for: "Resident local vector embeddings for zero-telemetry memory search".into(),
            sha256: None,
        },
        ModelCatalogEntry {
            id: "qwen-2.5-7b-instruct".into(),
            name: "Qwen 2.5 7B Instruct (Q4_K_M)".into(),
            tier: ModelTier::Smart,
            filename: "qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            download_url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            size_bytes: 4_680_000_000,
            context_size: 16384,
            recommended_for: "Complex autonomous multi-step agent planning and code refactoring".into(),
            sha256: None,
        },
    ]
}

/// Checks the local filesystem to determine if a catalog model is installed.
pub fn inspect_model_installation(models_dir: &Path, entry: &ModelCatalogEntry) -> ModelInstallStatus {
    let path = models_dir.join(&entry.filename);
    if path.exists() {
        if let Ok(meta) = std::fs::metadata(&path) {
            return ModelInstallStatus::Installed {
                path: path.to_string_lossy().to_string(),
                size_bytes: meta.len(),
            };
        }
    }

    ModelInstallStatus::NotInstalled {
        expected_size_bytes: entry.size_bytes,
        filename: entry.filename.clone(),
    }
}

/// Resolves the default local models directory.
pub fn default_models_directory() -> PathBuf {
    if let Ok(dir) = std::env::var("BROWSE_MODELS_DIR") {
        PathBuf::from(dir)
    } else if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        PathBuf::from(home).join(".browse").join("models")
    } else {
        PathBuf::from("models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_catalog_is_valid() {
        let catalog = get_recommended_catalog();
        assert!(catalog.len() >= 3);
        assert!(catalog.iter().any(|m| m.tier == ModelTier::Fast));
        assert!(catalog.iter().any(|m| m.tier == ModelTier::Embed));
        assert!(catalog.iter().any(|m| m.tier == ModelTier::Smart));
    }

    #[test]
    fn inspects_model_installation_status() {
        let temp_dir = std::env::temp_dir().join("browse_model_test");
        let _ = std::fs::create_dir_all(&temp_dir);

        let entry = ModelCatalogEntry {
            id: "test-model".into(),
            name: "Test Model".into(),
            tier: ModelTier::Fast,
            filename: "test_model.gguf".into(),
            download_url: "https://example.com/test.gguf".into(),
            size_bytes: 1000,
            context_size: 2048,
            recommended_for: "Testing".into(),
            sha256: None,
        };

        let status_before = inspect_model_installation(&temp_dir, &entry);
        assert!(matches!(status_before, ModelInstallStatus::NotInstalled { .. }));

        // Create dummy file
        let file_path = temp_dir.join(&entry.filename);
        std::fs::write(&file_path, b"dummy gguf content").unwrap();

        let status_after = inspect_model_installation(&temp_dir, &entry);
        match status_after {
            ModelInstallStatus::Installed { size_bytes, .. } => {
                assert_eq!(size_bytes, 18);
            }
            ModelInstallStatus::NotInstalled { .. } => panic!("Expected Installed status"),
        }

        let _ = std::fs::remove_file(file_path);
        let _ = std::fs::remove_dir(temp_dir);
    }
}
