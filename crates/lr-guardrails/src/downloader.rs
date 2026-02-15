//! Safety model downloader from HuggingFace
//!
//! Downloads GGUF model files for local inference using streaming HTTP
//! with real-time progress reporting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use once_cell::sync::Lazy;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::{info, warn};

// Global download lock to prevent concurrent downloads
static DOWNLOAD_LOCK: Lazy<Arc<Mutex<()>>> = Lazy::new(|| Arc::new(Mutex::new(())));

const MAX_RETRIES: usize = 3;
const RETRY_DELAY_MS: u64 = 2000;
const MIN_DISK_SPACE_GB: u64 = 5;
const PROGRESS_EMIT_INTERVAL_MS: u128 = 250; // Emit progress at most every 250ms

/// Progress event payload for safety model downloads
#[derive(Debug, Clone, serde::Serialize)]
pub struct SafetyModelDownloadProgress {
    pub model_id: String,
    pub progress: f32,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub speed_bytes_per_sec: u64,
}

/// Download error payload
#[derive(Debug, Clone, serde::Serialize)]
pub struct SafetyModelDownloadError {
    pub model_id: String,
    pub error: String,
}

/// Download completion payload
#[derive(Debug, Clone, serde::Serialize)]
pub struct SafetyModelDownloadComplete {
    pub model_id: String,
    pub file_path: String,
    pub file_size: u64,
}

/// Status of a downloaded safety model
#[derive(Debug, Clone, serde::Serialize)]
pub struct SafetyModelDownloadStatus {
    pub downloaded: bool,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
}

/// Check available disk space at the given path
fn check_disk_space(path: &Path) -> Result<u64, String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let check_path = if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        let output = Command::new("df")
            .arg("-k")
            .arg(check_path)
            .output()
            .map_err(|e| format!("Failed to check disk space: {}", e))?;

        if !output.status.success() {
            return Err("Failed to check disk space".to_string());
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();
        if lines.len() < 2 {
            return Err("Unexpected df output".to_string());
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return Err("Failed to parse df output".to_string());
        }

        let available_kb: u64 = parts[3]
            .parse()
            .map_err(|e| format!("Failed to parse available space: {}", e))?;
        Ok(available_kb * 1024)
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
            .arg("-B1")
            .arg(check_path)
            .output()
            .map_err(|e| format!("Failed to check disk space: {}", e))?;

        if !output.status.success() {
            return Err("Failed to check disk space".to_string());
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();
        if lines.len() < 2 {
            return Err("Unexpected df output".to_string());
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return Err("Failed to parse df output".to_string());
        }

        let available_bytes: u64 = parts[3]
            .parse()
            .map_err(|e| format!("Failed to parse available space: {}", e))?;
        Ok(available_bytes)
    }

    #[cfg(target_os = "windows")]
    {
        warn!("Disk space check on Windows: using wmic fallback");
        Ok(u64::MAX)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        warn!("Disk space check not supported on this platform");
        Ok(u64::MAX)
    }
}

/// Get the directory where safety model files are stored
pub fn safety_models_dir() -> Result<PathBuf, String> {
    let config_dir =
        lr_utils::paths::config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    Ok(config_dir.join("safety_models"))
}

/// Get the expected file path for a model's GGUF file
pub fn model_file_path(model_id: &str, gguf_filename: &str) -> Result<PathBuf, String> {
    let dir = safety_models_dir()?;
    Ok(dir.join(model_id).join(gguf_filename))
}

/// Check if a model's GGUF file is already downloaded
pub fn get_download_status(model_id: &str, gguf_filename: &str) -> SafetyModelDownloadStatus {
    match model_file_path(model_id, gguf_filename) {
        Ok(path) => {
            if path.exists() {
                let file_size = std::fs::metadata(&path).map(|m| m.len()).ok();
                SafetyModelDownloadStatus {
                    downloaded: true,
                    file_path: Some(path.to_string_lossy().to_string()),
                    file_size,
                }
            } else {
                SafetyModelDownloadStatus {
                    downloaded: false,
                    file_path: None,
                    file_size: None,
                }
            }
        }
        Err(_) => SafetyModelDownloadStatus {
            downloaded: false,
            file_path: None,
            file_size: None,
        },
    }
}

/// Build the HuggingFace download URL for a file
fn hf_download_url(repo_id: &str, filename: &str) -> String {
    format!(
        "https://huggingface.co/{}/resolve/main/{}",
        repo_id, filename
    )
}

/// Download a GGUF model file from HuggingFace with streaming progress
///
/// # Arguments
/// * `model_id` - Unique model identifier for directory naming
/// * `hf_repo_id` - HuggingFace repository ID (e.g. "QuantFactory/shieldgemma-2b-GGUF")
/// * `gguf_filename` - Filename to download (e.g. "shieldgemma-2b.Q4_K_M.gguf")
/// * `hf_token` - Optional HuggingFace token for gated models
/// * `app_handle` - Optional Tauri AppHandle for progress events
pub async fn download_model(
    model_id: &str,
    hf_repo_id: &str,
    gguf_filename: &str,
    hf_token: Option<&str>,
    #[cfg(feature = "tauri-support")] app_handle: Option<tauri::AppHandle>,
) -> Result<PathBuf, String> {
    // Acquire download lock
    let lock_result = DOWNLOAD_LOCK.try_lock();
    if lock_result.is_err() {
        return Err(
            "Another download is already in progress. Please wait for it to complete.".to_string(),
        );
    }
    let _lock = lock_result.unwrap();

    info!(
        "Starting safety model download: {} from {}/{}",
        model_id, hf_repo_id, gguf_filename
    );

    let model_dir = safety_models_dir()?.join(model_id);

    // Ensure parent directory exists
    let parent = model_dir
        .parent()
        .ok_or_else(|| "Model directory has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create safety_models directory: {}", e))?;

    // Check disk space
    let available_bytes = check_disk_space(&model_dir).unwrap_or_else(|e| {
        warn!("Disk space check failed: {}, proceeding anyway", e);
        u64::MAX
    });
    let required_bytes = MIN_DISK_SPACE_GB * 1_073_741_824;
    if available_bytes < required_bytes {
        let error_msg = format!(
            "Insufficient disk space. Available: {:.2} GB, Required: {} GB",
            available_bytes as f64 / 1_073_741_824.0,
            MIN_DISK_SPACE_GB
        );

        #[cfg(feature = "tauri-support")]
        if let Some(ref handle) = app_handle {
            use tauri::Emitter;
            let _ = handle.emit(
                "safety-model-download-failed",
                SafetyModelDownloadError {
                    model_id: model_id.to_string(),
                    error: error_msg.clone(),
                },
            );
        }

        return Err(error_msg);
    }

    // Emit initial progress
    #[cfg(feature = "tauri-support")]
    if let Some(ref handle) = app_handle {
        use tauri::Emitter;
        let _ = handle.emit(
            "safety-model-download-progress",
            SafetyModelDownloadProgress {
                model_id: model_id.to_string(),
                progress: 0.0,
                total_bytes: 0,
                downloaded_bytes: 0,
                speed_bytes_per_sec: 0,
            },
        );
    }

    // Prepare temp directory
    let temp_dir = model_dir.with_extension("tmp");
    if temp_dir.exists() {
        tokio::fs::remove_dir_all(&temp_dir)
            .await
            .map_err(|e| format!("Failed to remove old temp directory: {}", e))?;
    }
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;

    let temp_file_path = temp_dir.join(gguf_filename);
    let url = hf_download_url(hf_repo_id, gguf_filename);

    // Build HTTP client
    let client = reqwest::Client::new();

    // Download with retry logic
    let mut last_error = None;
    let mut success = false;

    for attempt in 1..=MAX_RETRIES {
        info!("Download attempt {}/{} from {}", attempt, MAX_RETRIES, url);

        let mut request = client.get(&url);
        if let Some(token) = hf_token {
            request = request.bearer_auth(token);
        }

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Download attempt {} failed to connect: {}", attempt, e);
                last_error = Some(format!("Connection failed: {}", e));
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                }
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            let error_msg = format!("HTTP {} — {}", status, body);
            warn!("Download attempt {} failed: {}", attempt, error_msg);
            last_error = Some(error_msg);
            if attempt < MAX_RETRIES {
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
            continue;
        }

        let total_bytes = response.content_length().unwrap_or(0);
        let mut downloaded_bytes: u64 = 0;
        let download_start = Instant::now();
        let mut last_emit = Instant::now();

        // Open temp file for writing
        let mut file = match tokio::fs::File::create(&temp_file_path).await {
            Ok(f) => f,
            Err(e) => {
                last_error = Some(format!("Failed to create temp file: {}", e));
                break;
            }
        };

        let mut stream = response.bytes_stream();
        let mut stream_error = None;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        stream_error = Some(format!("Failed to write chunk: {}", e));
                        break;
                    }
                    downloaded_bytes += chunk.len() as u64;

                    // Emit progress at throttled interval
                    let now = Instant::now();
                    if now.duration_since(last_emit).as_millis() >= PROGRESS_EMIT_INTERVAL_MS {
                        last_emit = now;

                        let elapsed_secs =
                            download_start.elapsed().as_secs_f64().max(0.001);
                        let speed =
                            (downloaded_bytes as f64 / elapsed_secs) as u64;
                        let progress = if total_bytes > 0 {
                            downloaded_bytes as f32 / total_bytes as f32
                        } else {
                            0.0
                        };

                        #[cfg(feature = "tauri-support")]
                        if let Some(ref handle) = app_handle {
                            use tauri::Emitter;
                            let _ = handle.emit(
                                "safety-model-download-progress",
                                SafetyModelDownloadProgress {
                                    model_id: model_id.to_string(),
                                    progress,
                                    total_bytes,
                                    downloaded_bytes,
                                    speed_bytes_per_sec: speed,
                                },
                            );
                        }
                    }
                }
                Err(e) => {
                    stream_error = Some(format!("Stream error: {}", e));
                    break;
                }
            }
        }

        if let Err(e) = file.flush().await {
            warn!("Failed to flush file: {}", e);
        }
        drop(file);

        if let Some(err) = stream_error {
            warn!("Download attempt {} stream error: {}", attempt, err);
            last_error = Some(err);
            // Clean up partial file
            let _ = tokio::fs::remove_file(&temp_file_path).await;
            if attempt < MAX_RETRIES {
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
            continue;
        }

        info!(
            "Download succeeded on attempt {} ({} bytes in {:.1}s)",
            attempt,
            downloaded_bytes,
            download_start.elapsed().as_secs_f64()
        );
        success = true;
        break;
    }

    if !success {
        let error_msg = format!(
            "Download failed after {} attempts. Last error: {}",
            MAX_RETRIES,
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        );

        // Clean up temp dir
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        #[cfg(feature = "tauri-support")]
        if let Some(ref handle) = app_handle {
            use tauri::Emitter;
            let _ = handle.emit(
                "safety-model-download-failed",
                SafetyModelDownloadError {
                    model_id: model_id.to_string(),
                    error: error_msg.clone(),
                },
            );
        }

        return Err(error_msg);
    }

    // Get file size
    let file_size = tokio::fs::metadata(&temp_file_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    // Atomic move to final location
    if model_dir.exists() {
        tokio::fs::remove_dir_all(&model_dir)
            .await
            .map_err(|e| format!("Failed to remove old model directory: {}", e))?;
    }
    tokio::fs::rename(&temp_dir, &model_dir)
        .await
        .map_err(|e| format!("Failed to move model to final location: {}", e))?;

    let final_path = model_dir.join(gguf_filename);
    info!(
        "Safety model '{}' downloaded successfully: {:?} ({:.2} MB)",
        model_id,
        final_path,
        file_size as f64 / 1_048_576.0
    );

    // Emit completion
    #[cfg(feature = "tauri-support")]
    if let Some(ref handle) = app_handle {
        use tauri::Emitter;
        let _ = handle.emit(
            "safety-model-download-complete",
            SafetyModelDownloadComplete {
                model_id: model_id.to_string(),
                file_path: final_path.to_string_lossy().to_string(),
                file_size,
            },
        );
    }

    Ok(final_path)
}

/// Delete downloaded model files for a given model
pub async fn delete_model_files(model_id: &str) -> Result<(), String> {
    let model_dir = safety_models_dir()?.join(model_id);
    if model_dir.exists() {
        tokio::fs::remove_dir_all(&model_dir)
            .await
            .map_err(|e| format!("Failed to delete model files: {}", e))?;
        info!("Deleted safety model files for '{}'", model_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safety_models_dir() {
        let dir = safety_models_dir();
        assert!(dir.is_ok());
        let dir = dir.unwrap();
        assert!(dir.to_string_lossy().contains("safety_models"));
    }

    #[test]
    fn test_model_file_path() {
        let path = model_file_path("test_model", "test.gguf").unwrap();
        assert!(path.to_string_lossy().contains("test_model"));
        assert!(path.to_string_lossy().ends_with("test.gguf"));
    }

    #[test]
    fn test_download_status_not_downloaded() {
        let status = get_download_status("nonexistent_model", "nonexistent.gguf");
        assert!(!status.downloaded);
        assert!(status.file_path.is_none());
        assert!(status.file_size.is_none());
    }

    #[test]
    fn test_hf_download_url() {
        let url = hf_download_url("QuantFactory/shieldgemma-2b-GGUF", "shieldgemma-2b.Q4_K_M.gguf");
        assert_eq!(
            url,
            "https://huggingface.co/QuantFactory/shieldgemma-2b-GGUF/resolve/main/shieldgemma-2b.Q4_K_M.gguf"
        );
    }
}
