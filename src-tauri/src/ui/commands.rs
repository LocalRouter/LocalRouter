//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.

use crate::api_keys::ApiKeyManager;
use crate::config::{ConfigManager, ModelSelection, RouterConfig};
use serde::{Deserialize, Serialize};
use tauri::State;

/// API key information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub model_selection: ModelSelection,
    pub enabled: bool,
    pub created_at: String,
}

/// List all API keys
#[tauri::command]
pub async fn list_api_keys(key_manager: State<'_, ApiKeyManager>) -> Result<Vec<ApiKeyInfo>, String> {
    let keys = key_manager.list_keys();
    Ok(keys
        .into_iter()
        .map(|k| ApiKeyInfo {
            id: k.id,
            name: k.name,
            model_selection: k.model_selection,
            enabled: k.enabled,
            created_at: k.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new API key
#[tauri::command]
pub async fn create_api_key(
    name: Option<String>,
    model_selection: ModelSelection,
    key_manager: State<'_, ApiKeyManager>,
) -> Result<(String, ApiKeyInfo), String> {
    let (key, config) = key_manager
        .create_key(name, model_selection)
        .await
        .map_err(|e| e.to_string())?;

    Ok((
        key,
        ApiKeyInfo {
            id: config.id,
            name: config.name,
            model_selection: config.model_selection,
            enabled: config.enabled,
            created_at: config.created_at.to_rfc3339(),
        },
    ))
}

/// List all routers
#[tauri::command]
pub async fn list_routers(config_manager: State<'_, ConfigManager>) -> Result<Vec<RouterConfig>, String> {
    let config = config_manager.get();
    Ok(config.routers)
}

/// Get current configuration
#[tauri::command]
pub async fn get_config(config_manager: State<'_, ConfigManager>) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(config).map_err(|e| e.to_string())
}
