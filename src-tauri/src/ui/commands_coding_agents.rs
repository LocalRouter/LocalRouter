//! Tauri commands for AI coding agents management.

use lr_coding_agents::manager::CodingAgentManager;
use lr_config::{CodingAgentType, CodingPermissionMode, ConfigManager, PermissionState};
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
    pub tool_prefix: String,
    pub binary_name: String,
    pub installed: bool,
    pub working_directory: Option<String>,
    pub model_id: Option<String>,
    pub permission_mode: CodingPermissionMode,
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

/// List all coding agents with their detection status and configuration
#[tauri::command]
pub async fn list_coding_agents(
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<CodingAgentInfo>, String> {
    let config = config_manager.get();
    let installed = CodingAgentManager::detect_installed_agents();

    let agents: Vec<CodingAgentInfo> = CodingAgentType::all()
        .iter()
        .map(|agent_type| {
            let agent_config = config
                .coding_agents
                .agents
                .iter()
                .find(|a| a.agent_type == *agent_type);

            CodingAgentInfo {
                agent_type: *agent_type,
                display_name: agent_type.display_name().to_string(),
                tool_prefix: agent_type.tool_prefix().to_string(),
                binary_name: agent_type.binary_name().to_string(),
                installed: installed.contains(agent_type),
                working_directory: agent_config.and_then(|c| c.working_directory.clone()),
                model_id: agent_config.and_then(|c| c.model_id.clone()),
                permission_mode: agent_config
                    .map(|c| c.permission_mode)
                    .unwrap_or_default(),
            }
        })
        .collect();

    Ok(agents)
}

/// Update coding agent configuration
#[tauri::command]
pub async fn update_coding_agent_config(
    agent_type: CodingAgentType,
    working_directory: Option<String>,
    model_id: Option<String>,
    permission_mode: Option<CodingPermissionMode>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(agent) = cfg
                .coding_agents
                .agents
                .iter_mut()
                .find(|a| a.agent_type == agent_type)
            {
                if let Some(wd) = working_directory {
                    agent.working_directory = if wd.is_empty() { None } else { Some(wd) };
                }
                if let Some(model) = model_id {
                    agent.model_id = if model.is_empty() { None } else { Some(model) };
                }
                if let Some(mode) = permission_mode {
                    agent.permission_mode = mode;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("coding-agents-changed", ()) {
        tracing::error!("Failed to emit coding-agents-changed event: {}", e);
    }

    Ok(())
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

/// Permission level for coding agents (global or per-agent)
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentsPermissionLevel {
    Global,
    Agent,
}

/// Set client coding agents permission
#[tauri::command]
pub async fn set_client_coding_agents_permission(
    client_id: String,
    level: CodingAgentsPermissionLevel,
    key: Option<String>,
    state: PermissionState,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                match level {
                    CodingAgentsPermissionLevel::Global => {
                        client.coding_agents_permissions.global = state.clone();
                        // Clear all per-agent overrides so they inherit the new global value
                        client.coding_agents_permissions.agents.clear();
                    }
                    CodingAgentsPermissionLevel::Agent => {
                        if let Some(agent_key) = &key {
                            // If the new state matches the global, remove the override (inherit)
                            if state == client.coding_agents_permissions.global {
                                client.coding_agents_permissions.agents.remove(agent_key);
                            } else {
                                client
                                    .coding_agents_permissions
                                    .agents
                                    .insert(agent_key.clone(), state.clone());
                            }
                        }
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}
