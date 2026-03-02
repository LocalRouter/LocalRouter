//! Tauri commands for AI coding agents management.

use lr_coding_agents::manager::CodingAgentManager;
use lr_config::{CodingAgentType, ConfigManager, PermissionState};
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};

/// Detected agent info returned to the frontend.
/// An agent is implicitly enabled when installed — no explicit enable flag.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentInfo {
    pub agent_type: CodingAgentType,
    pub display_name: String,
    pub binary_name: String,
    pub installed: bool,
}

/// Session info returned to the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingSessionInfo {
    pub session_id: String,
    pub agent_type: CodingAgentType,
    pub client_id: String,
    pub working_directory: String,
    pub display_text: String,
    pub status: String,
    pub created_at: String,
}

/// List all coding agents with their detection status
#[tauri::command]
pub async fn list_coding_agents(
    _config_manager: State<'_, ConfigManager>,
) -> Result<Vec<CodingAgentInfo>, String> {
    let installed = CodingAgentManager::detect_installed_agents();

    let agents: Vec<CodingAgentInfo> = CodingAgentType::all()
        .iter()
        .map(|agent_type| CodingAgentInfo {
            agent_type: *agent_type,
            display_name: agent_type.display_name().to_string(),
            binary_name: agent_type.binary_name().to_string(),
            installed: installed.contains(agent_type),
        })
        .collect();

    Ok(agents)
}

/// List active coding sessions
#[tauri::command]
pub async fn list_coding_sessions(
    manager: State<'_, Arc<CodingAgentManager>>,
) -> Result<Vec<CodingSessionInfo>, String> {
    let sessions = manager.list_all_sessions().await;
    Ok(sessions
        .into_iter()
        .map(|s| CodingSessionInfo {
            session_id: s.session_id,
            agent_type: CodingAgentType::ClaudeCode, // TODO: store agent_type in SessionSummary
            client_id: String::new(),
            working_directory: s.working_directory,
            display_text: s.display_text,
            status: s.status.to_string(),
            created_at: s.timestamp.to_rfc3339(),
        })
        .collect())
}

/// End a coding session
#[tauri::command]
pub async fn end_coding_session(
    session_id: String,
    manager: State<'_, Arc<CodingAgentManager>>,
) -> Result<(), String> {
    manager
        .end_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get max concurrent coding agent sessions
#[tauri::command]
pub async fn get_max_coding_sessions(
    config_manager: State<'_, ConfigManager>,
) -> Result<usize, String> {
    Ok(config_manager.get().coding_agents.max_concurrent_sessions)
}

/// Set max concurrent coding agent sessions (0 = unlimited)
#[tauri::command]
pub async fn set_max_coding_sessions(
    max_sessions: usize,
    config_manager: State<'_, ConfigManager>,
    coding_agent_manager: State<'_, Arc<CodingAgentManager>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.coding_agents.max_concurrent_sessions = max_sessions;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Hot-reload into the running manager
    coding_agent_manager.set_max_concurrent_sessions(max_sessions);

    Ok(())
}

/// Set client coding agent permission (Allow/Ask/Off)
#[tauri::command]
pub async fn set_client_coding_agent_permission(
    client_id: String,
    permission: PermissionState,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.coding_agent_permission = permission.clone();
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Set client coding agent type
#[tauri::command]
pub async fn set_client_coding_agent_type(
    client_id: String,
    agent_type: Option<CodingAgentType>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.coding_agent_type = agent_type;
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}
