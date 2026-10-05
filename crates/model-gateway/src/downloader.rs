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

/// Progress notification during catalog model download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f32,
    pub complete: bool,
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

/// Download a model from catalog into target models directory with progress reporting.
/// In offline mode or when network fails, returns an error cleanly.
#[cfg(feature = "http")]
pub async fn download_catalog_model<F>(
    models_dir: &Path,
    entry: &ModelCatalogEntry,
    client_opt: Option<&reqwest::Client>,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(ModelDownloadProgress) + Send,
{
    if let Err(e) = std::fs::create_dir_all(models_dir) {
        return Err(format!("Failed to create models directory: {e}"));
    }

    let default_client;
    let client = match client_opt {
        Some(c) => c,
        None => {
            default_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3600))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
            &default_client
        }
    };

    let target_path = models_dir.join(&entry.filename);
    let part_path = models_dir.join(format!("{}.part", entry.filename));

    let res = client
        .get(&entry.download_url)
        .send()
        .await
        .map_err(|e| format!("Network request failed: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("Download failed with HTTP status: {}", res.status()));
    }

    let total_bytes = res.content_length().unwrap_or(entry.size_bytes);
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("Failed to create file: {e}"))?;

    let mut stream = res.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Error reading chunk: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Error writing chunk: {e}"))?;

        downloaded += chunk.len() as u64;
        let pct = if total_bytes > 0 {
            (downloaded as f32 / total_bytes as f32) * 100.0
        } else {
            0.0
        };

        on_progress(ModelDownloadProgress {
            model_id: entry.id.clone(),
            filename: entry.filename.clone(),
            downloaded_bytes: downloaded,
            total_bytes,
            percent: pct,
            complete: false,
        });
    }

    file.flush()
        .await
        .map_err(|e| format!("Error flushing file: {e}"))?;
    drop(file);

    if let Err(e) = std::fs::rename(&part_path, &target_path) {
        return Err(format!("Failed to finalize downloaded file: {e}"));
    }

    on_progress(ModelDownloadProgress {
        model_id: entry.id.clone(),
        filename: entry.filename.clone(),
        downloaded_bytes: downloaded,
        total_bytes,
        percent: 100.0,
        complete: true,
    });

    Ok(target_path)
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
