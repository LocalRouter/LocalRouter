//! Tauri commands for AI coding agents management.

use lr_coding_agents::manager::CodingAgentManager;
use lr_config::{CodingAgentType, CodingPermissionMode, ConfigManager, PermissionState};
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};

/// Tool definition returned to the frontend (snake_case fields).
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

impl From<lr_types::mcp_types::McpTool> for ToolDefinition {
    fn from(tool: lr_types::mcp_types::McpTool) -> Self {
        Self {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        }
    }
}

/// Detected agent info returned to the frontend.
/// An agent is implicitly enabled when installed — no explicit enable flag.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentInfo {
    pub agent_type: CodingAgentType,
    pub display_name: String,
    pub binary_name: String,
    pub installed: bool,
    pub binary_path: Option<String>,
    pub description: String,
    pub supports_model_selection: bool,
    pub supported_permission_modes: Vec<CodingPermissionMode>,
    pub mcp_tool_prefix: String,
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
        .map(|agent_type| {
            let is_installed = installed.contains(agent_type);
            let binary_path = if is_installed {
                which::which(agent_type.binary_name())
                    .ok()
                    .map(|p| p.display().to_string())
            } else {
                None
            };
            CodingAgentInfo {
                agent_type: *agent_type,
                display_name: agent_type.display_name().to_string(),
                binary_name: agent_type.binary_name().to_string(),
                installed: is_installed,
                binary_path,
                description: agent_type.description().to_string(),
                supports_model_selection: agent_type.supports_model_selection(),
                supported_permission_modes: agent_type.supported_permission_modes(),
                mcp_tool_prefix: agent_type.tool_prefix().to_string(),
            }
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
            agent_type: s.agent_type,
            client_id: s.client_id,
            working_directory: s.working_directory,
            display_text: s.display_text,
            status: s.status.to_string(),
            created_at: s.timestamp.to_rfc3339(),
        })
        .collect())
}

/// Get detailed info for a specific coding session
#[tauri::command]
pub async fn get_coding_session_detail(
    session_id: String,
    manager: State<'_, Arc<CodingAgentManager>>,
) -> Result<lr_coding_agents::types::SessionDetail, String> {
    manager
        .get_session_detail(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get the version of an installed coding agent binary
#[tauri::command]
pub async fn get_coding_agent_version(
    agent_type: CodingAgentType,
) -> Result<Option<String>, String> {
    let binary = agent_type.binary_name();
    let flag = agent_type.version_flag();

    let output = match tokio::process::Command::new(binary)
        .arg(flag)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return Ok(None),
    };

    let text = String::from_utf8_lossy(if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    });

    // Take first non-empty line, trim whitespace
    let version = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string());

    Ok(version)
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

/// Get the MCP tool definitions for a given coding agent type.
/// Returns the 6 unified coding agent tools with their schemas.
#[tauri::command]
pub async fn get_coding_agent_tool_definitions(
    agent_type: CodingAgentType,
) -> Result<Vec<ToolDefinition>, String> {
    Ok(
        lr_coding_agents::mcp_tools::build_tools_for_agent(agent_type)
            .into_iter()
            .map(ToolDefinition::from)
            .collect(),
    )
}

/// Get the context-mode tool definitions (ctx_search + indexing tools).
#[tauri::command]
pub async fn get_context_mode_tool_definitions(
    indexing_tools_enabled: bool,
) -> Result<Vec<ToolDefinition>, String> {
    Ok(
        lr_mcp::gateway::context_mode::build_fallback_tools(indexing_tools_enabled)
            .into_iter()
            .map(ToolDefinition::from)
            .collect(),
    )
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
