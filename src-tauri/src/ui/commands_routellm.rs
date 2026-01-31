//! Tauri commands for RouteLLM intelligent routing

use lr_routellm::{RouteLLMStatus, RouteLLMTestResult};
use lr_server::state::AppState;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, State};
use tracing::info;

/// Get global RouteLLM status
#[tauri::command]
pub async fn routellm_get_status(
    state: State<'_, Arc<AppState>>,
) -> Result<RouteLLMStatus, String> {
    // Check if RouteLLM service is available in Router
    if let Some(service) = state.router.get_routellm_service() {
        Ok(service.get_status().await)
    } else {
        // Service not initialized
        Ok(RouteLLMStatus {
            state: lr_routellm::status::RouteLLMState::NotDownloaded,
            memory_usage_mb: None,
            last_access_secs_ago: None,
        })
    }
}

/// Test prediction (try-it-out)
#[tauri::command]
pub async fn routellm_test_prediction(
    prompt: String,
    threshold: f32,
    state: State<'_, Arc<AppState>>,
) -> Result<RouteLLMTestResult, String> {
    // Validate threshold
    if !threshold.is_finite() {
        return Err("Threshold must be a finite number".to_string());
    }
    if !(0.0..=1.0).contains(&threshold) {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }

    // Validate prompt
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }
    if prompt.len() > 4096 {
        return Err("Prompt is too long (max 4096 characters)".to_string());
    }

    let service = state
        .router
        .get_routellm_service()
        .ok_or_else(|| "RouteLLM service not available".to_string())?;

    let start = Instant::now();
    let (is_strong, win_rate) = service
        .predict_with_threshold(prompt, threshold)
        .await
        .map_err(|e| format!("Prediction failed: {}", e))?;
    let latency_ms = start.elapsed().as_millis() as u64;

    Ok(RouteLLMTestResult {
        is_strong,
        win_rate,
        latency_ms,
    })
}

/// Manually unload models from memory
#[tauri::command]
pub async fn routellm_unload(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let service = state
        .router
        .get_routellm_service()
        .ok_or_else(|| "RouteLLM service not available".to_string())?;

    service.unload().await;
    info!("RouteLLM models unloaded via Tauri command");
    Ok(())
}

/// Download RouteLLM models from HuggingFace
#[tauri::command]
pub async fn routellm_download_models(
    state: State<'_, Arc<AppState>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    info!("Starting RouteLLM model download via Tauri command");

    let service = state
        .router
        .get_routellm_service()
        .ok_or_else(|| "RouteLLM service not available".to_string())?;

    let (model_path, tokenizer_path) = service.get_paths();

    // Use the downloader module (downloads SafeTensors from HuggingFace)
    lr_routellm::downloader::download_models(&model_path, &tokenizer_path, Some(app_handle))
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    Ok(())
}

/// Open the RouteLLM folder in the system file manager
#[tauri::command]
pub async fn open_routellm_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let config_dir = lr_config::paths::config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;
    let routellm_dir = config_dir.join("routellm");

    // Ensure directory exists
    if !routellm_dir.exists() {
        std::fs::create_dir_all(&routellm_dir)
            .map_err(|e| format!("Failed to create routellm directory: {}", e))?;
    }

    // Open in system file manager
    #[allow(deprecated)]
    app.shell()
        .open(routellm_dir.to_string_lossy().as_ref(), None)
        .map_err(|e| format!("Failed to open routellm folder: {}", e))?;

    Ok(())
}

/// Update global RouteLLM settings
#[tauri::command]
pub async fn routellm_update_settings(
    idle_timeout_secs: u64,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Validate idle timeout (reasonable range: 0 = never, max = 24 hours)
    if idle_timeout_secs > 86400 {
        return Err("Idle timeout cannot exceed 24 hours (86400 seconds)".to_string());
    }

    // Update config
    state
        .config_manager
        .update(|cfg| {
            cfg.routellm_settings.idle_timeout_secs = idle_timeout_secs;
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    state
        .config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Update the running service's timeout setting
    if let Some(service) = state.router.get_routellm_service() {
        service.set_idle_timeout(idle_timeout_secs).await;
    }

    info!(
        "RouteLLM settings updated: idle_timeout_secs={}",
        idle_timeout_secs
    );
    Ok(())
}
