//! Model downloader from HuggingFace

#![allow(dead_code)]

use crate::errors::{RouteLLMError, RouteLLMResult};
use hf_hub::api::tokio::Api;
use lr_config::RouteLLMDownloadStatus;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

// Global download lock to prevent concurrent downloads
static DOWNLOAD_LOCK: once_cell::sync::Lazy<Arc<Mutex<()>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(())));

// Download configuration constants
const DOWNLOAD_TIMEOUT_SECS: u64 = 600; // 10 minutes
const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000; // 2 seconds between retries
const MIN_DISK_SPACE_GB: u64 = 2; // Require 2 GB free space

/// Check available disk space
fn check_disk_space(path: &Path) -> RouteLLMResult<u64> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        // Get the parent directory or the path itself
        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        let output = Command::new("df")
            .arg("-k") // Output in KB
            .arg(check_path)
            .output()
            .map_err(|e| {
                RouteLLMError::DownloadFailed(format!("Failed to check disk space: {}", e))
            })?;

        if !output.status.success() {
            return Err(RouteLLMError::DownloadFailed(
                "Failed to check disk space".to_string(),
            ));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.len() < 2 {
            return Err(RouteLLMError::DownloadFailed(
                "Unexpected df output".to_string(),
            ));
        }

        // Parse the second line (data line)
        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return Err(RouteLLMError::DownloadFailed(
                "Failed to parse df output".to_string(),
            ));
        }

        // Column 3 is available space in KB
        let available_kb: u64 = parts[3].parse().map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to parse available space: {}", e))
        })?;

        Ok(available_kb * 1024) // Convert to bytes
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        let output = Command::new("df")
            .arg("-B1") // Output in bytes
            .arg(check_path)
            .output()
            .map_err(|e| {
                RouteLLMError::DownloadFailed(format!("Failed to check disk space: {}", e))
            })?;

        if !output.status.success() {
            return Err(RouteLLMError::DownloadFailed(
                "Failed to check disk space".to_string(),
            ));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.len() < 2 {
            return Err(RouteLLMError::DownloadFailed(
                "Unexpected df output".to_string(),
            ));
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return Err(RouteLLMError::DownloadFailed(
                "Failed to parse df output".to_string(),
            ));
        }

        let available_bytes: u64 = parts[3].parse().map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to parse available space: {}", e))
        })?;

        Ok(available_bytes)
    }

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        // Get the parent directory or the path itself
        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        // Get the drive letter from the path (e.g., "C:" from "C:\Users\...")
        let path_str = check_path.to_string_lossy();
        let drive = if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
            &path_str[0..2]
        } else {
            // Default to C: if no drive letter found
            "C:"
        };

        // Use wmic to get free space (works on all Windows versions)
        let output = Command::new("wmic")
            .args([
                "logicaldisk",
                "where",
                &format!("DeviceID='{}'", drive),
                "get",
                "FreeSpace",
            ])
            .output()
            .map_err(|e| {
                RouteLLMError::DownloadFailed(format!("Failed to check disk space: {}", e))
            })?;

        if !output.status.success() {
            warn!("Failed to check disk space on Windows, proceeding anyway");
            return Ok(u64::MAX);
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        // Parse output: "FreeSpace\r\n12345678\r\n"
        let available_bytes: u64 = output_str
            .lines()
            .skip(1) // Skip header
            .next()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(u64::MAX);

        Ok(available_bytes)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        // Unsupported platform - skip check and return a large value
        warn!("Disk space check not supported on this platform");
        Ok(u64::MAX)
    }
}

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
            "Another download is already in progress. Please wait for it to complete.".to_string(),
        ));
    }
    let _lock = lock_result.unwrap();

    info!("Starting RouteLLM model download from HuggingFace");
    info!("  Model dir: {:?}", model_path);
    info!("  Tokenizer dir: {:?}", tokenizer_path);

    // Check available disk space before downloading
    let available_bytes = check_disk_space(model_path)?;
    let available_gb = available_bytes as f64 / 1_073_741_824.0; // Convert to GB
    let required_gb = MIN_DISK_SPACE_GB;

    info!("Available disk space: {:.2} GB", available_gb);

    if available_bytes < (required_gb * 1_073_741_824) {
        let error_msg = format!(
            "Insufficient disk space. Available: {:.2} GB, Required: {} GB",
            available_gb, required_gb
        );
        warn!("{}", error_msg);

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

    // Use temporary directories for atomic download
    let temp_model_path = model_path
        .parent()
        .ok_or_else(|| {
            RouteLLMError::DownloadFailed("Model path has no parent directory".to_string())
        })?
        .join("model.tmp");
    let temp_tokenizer_path = tokenizer_path
        .parent()
        .ok_or_else(|| {
            RouteLLMError::DownloadFailed("Tokenizer path has no parent directory".to_string())
        })?
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
    tokio::fs::create_dir_all(&temp_model_path)
        .await
        .map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to create temp model directory: {}", e))
        })?;
    tokio::fs::create_dir_all(&temp_tokenizer_path)
        .await
        .map_err(|e| {
            RouteLLMError::DownloadFailed(format!(
                "Failed to create temp tokenizer directory: {}",
                e
            ))
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

    // Download model.safetensors to temporary location with retry logic
    info!("Downloading model.safetensors from HuggingFace...");

    let downloaded_model = {
        let mut last_error = None;
        let mut success = None;

        for attempt in 1..=MAX_RETRIES {
            info!("Download attempt {}/{}", attempt, MAX_RETRIES);

            // Wrap in timeout
            let download_future = repo.get("model.safetensors");
            let result = timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS), download_future).await;

            match result {
                Ok(Ok(path)) => {
                    info!("Model download succeeded on attempt {}", attempt);
                    success = Some(path);
                    break;
                }
                Ok(Err(e)) => {
                    warn!("Download attempt {} failed: {}", attempt, e);
                    last_error = Some(format!("{}", e));
                }
                Err(_) => {
                    warn!(
                        "Download attempt {} timed out after {} seconds",
                        attempt, DOWNLOAD_TIMEOUT_SECS
                    );
                    last_error = Some(format!(
                        "Download timed out after {} seconds",
                        DOWNLOAD_TIMEOUT_SECS
                    ));
                }
            }

            // Wait before retrying (unless this was the last attempt)
            if attempt < MAX_RETRIES {
                info!("Waiting {} seconds before retry...", RETRY_DELAY_MS / 1000);
                tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
            }
        }

        match success {
            Some(path) => path,
            None => {
                let error_msg = format!(
                    "Model download failed after {} attempts. Last error: {}. Please check your internet connection.",
                    MAX_RETRIES,
                    last_error.unwrap_or_else(|| "Unknown error".to_string())
                );
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

                return Err(RouteLLMError::DownloadFailed(error_msg));
            }
        }
    };

    let temp_model_file = temp_model_path.join("model.safetensors");
    debug!(
        "Copying model.safetensors to temp location: {:?}",
        temp_model_file
    );
    tokio::fs::copy(&downloaded_model, &temp_model_file)
        .await
        .map_err(|e| RouteLLMError::DownloadFailed(format!("Failed to copy model file: {}", e)))?;

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
    let tokenizer_files = [
        "tokenizer.json",
        "tokenizer_config.json",
        "sentencepiece.bpe.model", // XLM-RoBERTa uses SentencePiece
        "special_tokens_map.json",
        "config.json", // Model config
    ];

    for (idx, file) in tokenizer_files.iter().enumerate() {
        debug!("Downloading {}", file);

        // Download with retry logic
        let downloaded_file = {
            let mut last_error = None;
            let mut success = None;

            for attempt in 1..=MAX_RETRIES {
                debug!("Downloading {} - attempt {}/{}", file, attempt, MAX_RETRIES);

                // Wrap in timeout (shorter for tokenizer files since they're smaller)
                let download_future = repo.get(file);
                let result = timeout(Duration::from_secs(120), download_future).await;

                match result {
                    Ok(Ok(path)) => {
                        debug!("Downloaded {} on attempt {}", file, attempt);
                        success = Some(path);
                        break;
                    }
                    Ok(Err(e)) => {
                        warn!("Failed to download {} (attempt {}): {}", file, attempt, e);
                        last_error = Some(format!("{}", e));
                    }
                    Err(_) => {
                        warn!("Download of {} timed out (attempt {})", file, attempt);
                        last_error = Some("Download timed out".to_string());
                    }
                }

                // Wait before retrying
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                }
            }

            match success {
                Some(path) => path,
                None => {
                    let error_msg = format!(
                        "Failed to download {} after {} attempts: {}",
                        file,
                        MAX_RETRIES,
                        last_error.unwrap_or_else(|| "Unknown error".to_string())
                    );
                    warn!("{}", error_msg);

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
            }
        };

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
        let error_msg = format!(
            "Model verification failed: {}. The downloaded model appears corrupted.",
            e
        );
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
        tokio::fs::remove_dir_all(tokenizer_path)
            .await
            .map_err(|e| {
                RouteLLMError::DownloadFailed(format!(
                    "Failed to remove old tokenizer directory: {}",
                    e
                ))
            })?;
    }

    // Move temp directories to final locations
    tokio::fs::rename(&temp_model_path, model_path)
        .await
        .map_err(|e| {
            RouteLLMError::DownloadFailed(format!("Failed to move model to final location: {}", e))
        })?;
    tokio::fs::rename(&temp_tokenizer_path, tokenizer_path)
        .await
        .map_err(|e| {
            RouteLLMError::DownloadFailed(format!(
                "Failed to move tokenizer to final location: {}",
                e
            ))
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
async fn verify_model_loads(model_path: &Path, tokenizer_path: &Path) -> RouteLLMResult<()> {
    use crate::candle_router::CandleRouter;

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
) -> lr_config::RouteLLMDownloadStatus {
    use lr_config::RouteLLMDownloadState;

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
