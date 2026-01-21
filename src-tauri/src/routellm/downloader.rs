//! Model downloader from HuggingFace

#![allow(dead_code)]

use crate::config::RouteLLMDownloadStatus;
use crate::routellm::errors::{RouteLLMError, RouteLLMResult};
use hf_hub::api::tokio::Api;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tracing::{info, warn, debug};

// Global download lock to prevent concurrent downloads
static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

/// Download RouteLLM models from HuggingFace
///
/// Downloads SafeTensors models directly from HuggingFace Hub.
/// Repository: routellm/bert_gpt4_augmented
///
/// # Arguments
/// * `model_path` - Directory where model.safetensors will be saved
/// * `tokenizer_path` - Directory where tokenizer files will be saved
/// * `app_handle` - Optional Tauri app handle for progress events
pub async fn download_models(
    model_path: &Path,
    tokenizer_path: &Path,
    app_handle: Option<AppHandle>,
) -> RouteLLMResult<()> {
    // Try to acquire download lock (non-blocking check)
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err(RouteLLMError::DownloadFailed(
            "Another download is already in progress. Please wait for it to complete.".to_string()
        ));
    }
    let _lock = lock_result.unwrap();

    info!("Starting RouteLLM model download from HuggingFace");
    info!("  Model dir: {:?}", model_path);
    info!("  Tokenizer dir: {:?}", tokenizer_path);

    // Use temporary directories for atomic download
    let temp_model_path = model_path
        .parent()
        .ok_or_else(|| RouteLLMError::DownloadFailed("Model path has no parent directory".to_string()))?
        .join("model.tmp");
    let temp_tokenizer_path = tokenizer_path
        .parent()
        .ok_or_else(|| RouteLLMError::DownloadFailed("Tokenizer path has no parent directory".to_string()))?
        .join("tokenizer.tmp");

    // Remove old temp directories if they exist
    if temp_model_path.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&temp_model_path).await {
            warn!("Failed to remove old temp model directory: {}", e);
        }
    }
    if temp_tokenizer_path.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&temp_tokenizer_path).await {
            warn!("Failed to remove old temp tokenizer directory: {}", e);
        }
    }

    // Create temporary directories
    tokio::fs::create_dir_all(&temp_model_path).await.map_err(|e| {
        RouteLLMError::DownloadFailed(format!("Failed to create temp model directory: {}", e))
    })?;
    tokio::fs::create_dir_all(&temp_tokenizer_path).await.map_err(|e| {
        RouteLLMError::DownloadFailed(format!("Failed to create temp tokenizer directory: {}", e))
    })?;

    // Emit initial progress event
    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "routellm-download-progress",
            DownloadProgress {
                current_file: Some("model.safetensors".to_string()),
                progress: 0.0,
                total_bytes: 440_000_000, // ~440 MB
                downloaded_bytes: 0,
            },
        );
    }

    // Initialize HuggingFace API
    info!("Initializing HuggingFace API");
    let api = Api::new().map_err(|e| {
        RouteLLMError::DownloadFailed(format!("Failed to initialize HuggingFace API: {}", e))
    })?;

    let repo = api.model("routellm/bert_gpt4_augmented".to_string());

    // Download model.safetensors to temporary location
    info!("Downloading model.safetensors from HuggingFace...");
    let downloaded_model = repo.get("model.safetensors").await.map_err(|e| {
        let error_msg = format!("Model download failed: {}. Please check your internet connection.", e);
        warn!("{}", error_msg);

        // Emit failure event
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "routellm-download-failed",
                DownloadError {
                    error: error_msg.clone(),
                },
            );
        }

        RouteLLMError::DownloadFailed(error_msg)
    })?;

    let temp_model_file = temp_model_path.join("model.safetensors");
    debug!("Copying model.safetensors to temp location: {:?}", temp_model_file);
    tokio::fs::copy(&downloaded_model, &temp_model_file)
        .await
        .map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to copy model file: {}", e))
        })?;

    info!("Model downloaded to temporary location");

    // Emit progress (70% complete)
    if let Some(ref handle) = app_handle {
        let _ = handle.emit(
            "routellm-download-progress",
            DownloadProgress {
                current_file: Some("tokenizer.json".to_string()),
                progress: 0.7,
                total_bytes: 440_000_000,
                downloaded_bytes: 308_000_000,
            },
        );
    }

    // Download tokenizer files to temporary location
    info!("Downloading tokenizer files...");
    let tokenizer_files = vec![
        "tokenizer.json",
        "tokenizer_config.json",
        "sentencepiece.bpe.model",  // XLM-RoBERTa uses SentencePiece
        "special_tokens_map.json",
        "config.json", // Model config
    ];

    for (idx, file) in tokenizer_files.iter().enumerate() {
        debug!("Downloading {}", file);

        let downloaded_file = repo.get(file).await.map_err(|e| {
            let error_msg = format!("Failed to download {}: {}", file, e);
            warn!("{}", error_msg);

            if let Some(ref handle) = app_handle {
                let _ = handle.emit(
                    "routellm-download-failed",
                    DownloadError {
                        error: error_msg.clone(),
                    },
                );
            }

            RouteLLMError::DownloadFailed(error_msg)
        })?;

        let dest_file = temp_tokenizer_path.join(file);
        tokio::fs::copy(&downloaded_file, &dest_file)
            .await
            .map_err(|e| {
                RouteLLMError::DownloadFailed(format!("Failed to copy {}: {}", file, e))
            })?;

        // Update progress
        let progress = 0.7 + (0.3 * (idx + 1) as f32 / tokenizer_files.len() as f32);
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "routellm-download-progress",
                DownloadProgress {
                    current_file: Some(file.to_string()),
                    progress,
                    total_bytes: 440_000_000,
                    downloaded_bytes: (440_000_000 as f32 * progress) as u64,
                },
            );
        }
    }

    info!("Tokenizer files downloaded to temporary location");

    // Verify model loads correctly before moving to final location
    info!("Verifying downloaded model...");
    let verification_result = verify_model_loads(&temp_model_path, &temp_tokenizer_path).await;

    if let Err(e) = verification_result {
        let error_msg = format!("Model verification failed: {}. The downloaded model appears corrupted.", e);
        warn!("{}", error_msg);

        // Clean up temp directories
        tokio::fs::remove_dir_all(&temp_model_path).await.ok();
        tokio::fs::remove_dir_all(&temp_tokenizer_path).await.ok();

        // Emit failure event
        if let Some(ref handle) = app_handle {
            let _ = handle.emit(
                "routellm-download-failed",
                DownloadError {
                    error: error_msg.clone(),
                },
            );
        }

        return Err(RouteLLMError::DownloadFailed(error_msg));
    }

    info!("Model verification successful! Moving to final location...");

    // Atomically move temp directories to final locations
    // Remove old directories if they exist
    if model_path.exists() {
        tokio::fs::remove_dir_all(model_path).await.map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to remove old model directory: {}", e))
        })?;
    }
    if tokenizer_path.exists() {
        tokio::fs::remove_dir_all(tokenizer_path).await.map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to remove old tokenizer directory: {}", e))
        })?;
    }

    // Move temp directories to final locations
    tokio::fs::rename(&temp_model_path, model_path).await.map_err(|e| {
        RouteLLMError::DownloadFailed(format!("Failed to move model to final location: {}", e))
    })?;
    tokio::fs::rename(&temp_tokenizer_path, tokenizer_path).await.map_err(|e| {
        RouteLLMError::DownloadFailed(format!("Failed to move tokenizer to final location: {}", e))
    })?;

    info!("Models moved to final location successfully");

    // Emit completion
    if let Some(ref handle) = app_handle {
        let _ = handle.emit("routellm-download-complete", ());
    }

    info!("RouteLLM models downloaded and verified successfully from HuggingFace");
    Ok(())
}

/// Verify that the downloaded model can be loaded
///
/// This runs a quick test to ensure the model files are not corrupted
async fn verify_model_loads(
    model_path: &Path,
    tokenizer_path: &Path,
) -> RouteLLMResult<()> {
    use crate::routellm::candle_router::CandleRouter;

    info!("Testing model loading from temp location...");

    let model_path_buf = model_path.to_path_buf();
    let tokenizer_path_buf = tokenizer_path.to_path_buf();

    // Run model loading in blocking task since it's CPU-intensive
    tokio::task::spawn_blocking(move || {
        // Try to load the model
        let router = CandleRouter::new(&model_path_buf, &tokenizer_path_buf)?;

        // Run a test prediction to ensure everything works
        let test_prompt = "test";
        let _score = router.calculate_strong_win_rate(test_prompt)?;

        info!("Model verification test passed!");
        Ok::<(), RouteLLMError>(())
    })
    .await
    .map_err(|e| RouteLLMError::DownloadFailed(format!("Verification task failed: {}", e)))??;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
struct DownloadProgress {
    current_file: Option<String>,
    progress: f32,
    total_bytes: u64,
    downloaded_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DownloadError {
    error: String,
}

/// Get download status
///
/// Checks if SafeTensors model files are downloaded
/// Note: After first load, the original model.safetensors is deleted and only
/// model.patched.safetensors remains (to save 1GB disk space)
pub fn get_download_status(
    model_path: &Path,
    tokenizer_path: &Path,
) -> crate::config::RouteLLMDownloadStatus {
    use crate::config::RouteLLMDownloadState;

    let model_file = model_path.join("model.safetensors");
    let patched_model_file = model_path.join("model.patched.safetensors");
    let tokenizer_file = tokenizer_path.join("tokenizer.json");

    // Check for either original or patched model file
    let model_exists = model_file.exists() || patched_model_file.exists();

    if model_exists && tokenizer_file.exists() {
        RouteLLMDownloadStatus {
            state: RouteLLMDownloadState::Downloaded,
            progress: 1.0,
            current_file: None,
            total_bytes: 440_000_000, // ~440 MB (SafeTensors)
            downloaded_bytes: 440_000_000,
            error: None,
        }
    } else {
        RouteLLMDownloadStatus {
            state: RouteLLMDownloadState::NotDownloaded,
            progress: 0.0,
            current_file: None,
            total_bytes: 440_000_000,
            downloaded_bytes: 0,
            error: None,
        }
    }
}
