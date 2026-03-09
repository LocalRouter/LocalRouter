//! Model downloader from HuggingFace for LLMLingua-2

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

/// Get the HuggingFace repo ID for a model size
pub fn repo_id_for_model(model_size: &str) -> &'static str {
    match model_size {
        "xlm-roberta" => "microsoft/llmlingua-2-xlm-roberta-large-meetingbank",
        _ => "microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank",
    }
}

/// Download LLMLingua-2 model from HuggingFace
pub async fn download_model(
    model_path: &Path,
    tokenizer_path: &Path,
    model_size: &str,
    app_handle: Option<AppHandle>,
) -> Result<(), String> {
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err("Another download is already in progress".to_string());
    }
    let _lock = lock_result.unwrap();

    let repo_id = repo_id_for_model(model_size);
    info!("Downloading LLMLingua-2 model from HuggingFace");
    info!("  Repo: {}", repo_id);
    info!("  Model dir: {:?}", model_path);
    info!("  Tokenizer dir: {:?}", tokenizer_path);

    // Create directories
    tokio::fs::create_dir_all(model_path)
        .await
        .map_err(|e| format!("Failed to create model directory: {}", e))?;
    tokio::fs::create_dir_all(tokenizer_path)
        .await
        .map_err(|e| format!("Failed to create tokenizer directory: {}", e))?;

    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "compression-download-progress",
            serde_json::json!({ "progress": 0.0, "current_file": "model.safetensors" }),
        );
    }

    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {}", e))?;
    let repo = api.model(repo_id.to_string());

    // Download model.safetensors (BERT ~709 MB, XLM-RoBERTa ~2.2 GB)
    info!("Downloading model.safetensors...");
    let model_downloaded = download_with_retry(&repo, "model.safetensors").await?;
    let dest = model_path.join("model.safetensors");
    tokio::fs::copy(&model_downloaded, &dest)
        .await
        .map_err(|e| format!("Failed to copy model: {}", e))?;

    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "compression-download-progress",
            serde_json::json!({ "progress": 0.7, "current_file": "tokenizer.json" }),
        );
    }

    // Download tokenizer files (BERT has vocab.txt, XLM-RoBERTa embeds vocab in tokenizer.json)
    let tokenizer_files: Vec<&str> = if model_size == "xlm-roberta" {
        vec![
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
        ]
    } else {
        vec![
            "tokenizer.json",
            "tokenizer_config.json",
            "vocab.txt",
            "special_tokens_map.json",
        ]
    };
    for (idx, file) in tokenizer_files.iter().enumerate() {
        debug!("Downloading {}", file);
        let downloaded = download_with_retry(&repo, file).await?;
        let dest = tokenizer_path.join(file);
        tokio::fs::copy(&downloaded, &dest)
            .await
            .map_err(|e| format!("Failed to copy {}: {}", file, e))?;

        let progress = 0.7 + 0.3 * (idx + 1) as f32 / tokenizer_files.len() as f32;
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "compression-download-progress",
                serde_json::json!({ "progress": progress, "current_file": file }),
            );
        }
    }

    if let Some(ref handle) = app_handle {
        let _ = handle.emit("compression-download-complete", ());
    }

    info!("LLMLingua-2 model downloaded successfully");
    Ok(())
}

/// Check if model files are present
pub fn is_downloaded(model_path: &Path, tokenizer_path: &Path) -> bool {
    model_path.join("model.safetensors").exists() && tokenizer_path.join("tokenizer.json").exists()
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
