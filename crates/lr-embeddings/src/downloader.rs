//! Model downloader from HuggingFace for all-MiniLM-L6-v2

use hf_hub::api::tokio::Api;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

const DOWNLOAD_TIMEOUT_SECS: u64 = 600;
const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000;

/// HuggingFace repo for the embedding model
pub const REPO_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Files needed for the embedding model
const MODEL_FILES: &[&str] = &["model.safetensors", "tokenizer.json"];

/// Download embedding model from HuggingFace
pub async fn download_model(model_dir: &Path, app_handle: Option<AppHandle>) -> Result<(), String> {
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err("Another embedding download is already in progress".to_string());
    }
    let _lock = lock_result.unwrap();

    info!("Downloading embedding model from HuggingFace");
    info!("  Repo: {}", REPO_ID);
    info!("  Dir: {:?}", model_dir);

    tokio::fs::create_dir_all(model_dir)
        .await
        .map_err(|e| format!("Failed to create model directory: {}", e))?;

    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "embedding-download-progress",
            serde_json::json!({ "progress": 0.0, "current_file": "model.safetensors" }),
        );
    }

    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {}", e))?;
    let repo = api.model(REPO_ID.to_string());

    for (idx, file) in MODEL_FILES.iter().enumerate() {
        info!("Downloading {}...", file);
        let downloaded = download_with_retry(&repo, file).await?;
        let dest = model_dir.join(file);
        tokio::fs::copy(&downloaded, &dest)
            .await
            .map_err(|e| format!("Failed to copy {}: {}", file, e))?;

        let progress = (idx + 1) as f32 / MODEL_FILES.len() as f32;
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "embedding-download-progress",
                serde_json::json!({ "progress": progress, "current_file": file }),
            );
        }
    }

    if let Some(ref handle) = app_handle {
        let _ = handle.emit("embedding-download-complete", ());
    }

    info!("Embedding model downloaded successfully");
    Ok(())
}

/// Check if model files are present
pub fn is_downloaded(model_dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| model_dir.join(f).exists())
}

/// Download a file with retry logic
async fn download_with_retry(
    repo: &hf_hub::api::tokio::ApiRepo,
    filename: &str,
) -> Result<std::path::PathBuf, String> {
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        let result = timeout(
            Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
            repo.get(filename),
        )
        .await;

        match result {
            Ok(Ok(path)) => {
                debug!("Downloaded {} on attempt {}", filename, attempt);
                return Ok(path);
            }
            Ok(Err(e)) => {
                warn!("Download {} attempt {} failed: {}", filename, attempt, e);
                last_error = Some(format!("{}", e));
            }
            Err(_) => {
                warn!("Download {} attempt {} timed out", filename, attempt);
                last_error = Some("Timed out".to_string());
            }
        }

        if attempt < MAX_RETRIES {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    Err(format!(
        "Failed to download {} after {} attempts: {}",
        filename,
        MAX_RETRIES,
        last_error.unwrap_or_else(|| "Unknown".to_string())
    ))
}
