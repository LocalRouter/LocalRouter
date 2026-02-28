//! MCP tool generation and dispatch for coding agents.
//!
//! Each enabled agent gets 6 tools: `{prefix}_start`, `{prefix}_say`,
//! `{prefix}_status`, `{prefix}_respond`, `{prefix}_interrupt`, `{prefix}_list`.

use crate::manager::CodingAgentManager;
use crate::types::*;
use lr_config::{CodingAgentType, CodingAgentsPermissions, CodingPermissionMode};
use lr_types::mcp_types::McpTool;
use serde_json::{json, Value};

/// Check if a tool name belongs to a coding agent
pub fn is_coding_agent_tool(tool_name: &str) -> bool {
    CodingAgentType::all().iter().any(|agent| {
        let prefix = agent.tool_prefix();
        tool_name == format!("{}_start", prefix)
            || tool_name == format!("{}_say", prefix)
            || tool_name == format!("{}_status", prefix)
            || tool_name == format!("{}_respond", prefix)
            || tool_name == format!("{}_interrupt", prefix)
            || tool_name == format!("{}_list", prefix)
    })
}

/// Extract the agent type from a tool name
pub fn agent_type_from_tool(tool_name: &str) -> Option<CodingAgentType> {
    CodingAgentType::all().iter().find(|agent| {
        tool_name.starts_with(agent.tool_prefix())
    }).copied()
}

/// Extract the action suffix from a tool name (e.g., "start", "say", "status")
fn action_from_tool(tool_name: &str) -> Option<&str> {
    // Find the last underscore-delimited segment
    let suffixes = ["_start", "_say", "_status", "_respond", "_interrupt", "_list"];
    for suffix in &suffixes {
        if tool_name.ends_with(suffix) {
            return Some(&suffix[1..]); // strip leading underscore
        }
    }
    None
}

/// Build MCP tools for all enabled agents
pub fn build_coding_agent_tools(
    manager: &CodingAgentManager,
    permissions: &CodingAgentsPermissions,
) -> Vec<McpTool> {
    if !permissions.has_any_access() {
        return Vec::new();
    }

    let mut tools = Vec::new();

    for agent_type in CodingAgentType::all() {
        if !manager.is_agent_enabled(*agent_type) {
            continue;
        }

        let perm = permissions.resolve_agent(agent_type.tool_prefix());
        if !perm.is_enabled() {
            continue;
        }

        tools.extend(build_tools_for_agent(*agent_type));
    }

    tools
}

/// Build the 6 tools for a single agent
fn build_tools_for_agent(agent_type: CodingAgentType) -> Vec<McpTool> {
    let prefix = agent_type.tool_prefix();
    let name = agent_type.display_name();

    vec![
        // {agent}_start
        McpTool {
            name: format!("{}_start", prefix),
            description: Some(format!(
                "Start a new {} coding session with an initial prompt",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The initial task/prompt"
                    },
                    "workingDirectory": {
                        "type": "string",
                        "description": "Working directory for the session"
                    },
                    "model": {
                        "type": "string",
                        "description": "Model override (optional, agent default applies)"
                    },
                    "permissionMode": {
                        "type": "string",
                        "enum": ["auto", "supervised", "plan"],
                        "description": "Permission mode. Default: agent's configured mode"
                    }
                },
                "required": ["prompt"]
            }),
        },
        // {agent}_say
        McpTool {
            name: format!("{}_say", prefix),
            description: Some(format!(
                "Send a message to a {} session. Automatically resumes if ended.",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "The session ID"
                    },
                    "message": {
                        "type": "string",
                        "description": "The message to send"
                    },
                    "permissionMode": {
                        "type": "string",
                        "enum": ["auto", "supervised", "plan"],
                        "description": "Switch permission mode (interrupts + resumes if active)"
                    }
                },
                "required": ["sessionId", "message"]
            }),
        },
        // {agent}_status
        McpTool {
            name: format!("{}_status", prefix),
            description: Some(format!(
                "Get current status and recent output of a {} session",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "The session ID"
                    },
                    "outputLines": {
                        "type": "number",
                        "description": "Recent output lines to return (default: 50)"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        // {agent}_respond
        McpTool {
            name: format!("{}_respond", prefix),
            description: Some(format!(
                "Respond to a pending question in a {} session",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "The session ID"
                    },
                    "id": {
                        "type": "string",
                        "description": "Question ID from pendingQuestion.id"
                    },
                    "answers": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "One answer per question (e.g. 'allow', 'deny: too risky')"
                    }
                },
                "required": ["sessionId", "id", "answers"]
            }),
        },
        // {agent}_interrupt
        McpTool {
            name: format!("{}_interrupt", prefix),
            description: Some(format!(
                "Interrupt a running {} session",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "The session ID to interrupt"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        // {agent}_list
        McpTool {
            name: format!("{}_list", prefix),
            description: Some(format!(
                "List all {} sessions for this client",
                name
            )),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "number",
                        "description": "Max sessions to return (default: 50)"
                    }
                }
            }),
        },
    ]
}

/// Handle a coding agent tool call.
///
/// Returns `Ok(Some(value))` on success, `Ok(None)` if tool not found,
/// or `Err` on error.
pub async fn handle_coding_agent_tool_call(
    tool_name: &str,
    arguments: &Value,
    manager: &CodingAgentManager,
    client_id: &str,
) -> Result<Option<Value>, String> {
    let agent_type = match agent_type_from_tool(tool_name) {
        Some(at) => at,
        None => return Ok(None),
    };

    let action = match action_from_tool(tool_name) {
        Some(a) => a,
        None => return Ok(None),
    };

    if !manager.is_agent_enabled(agent_type) {
        return Err(format!("{} is not enabled", agent_type.display_name()));
    }

    match action {
        "start" => handle_start(manager, agent_type, client_id, arguments).await,
        "say" => handle_say(manager, agent_type, client_id, arguments).await,
        "status" => handle_status(manager, client_id, arguments).await,
        "respond" => handle_respond(manager, client_id, arguments).await,
        "interrupt" => handle_interrupt(manager, client_id, arguments).await,
        "list" => handle_list(manager, agent_type, client_id, arguments).await,
        _ => Ok(None),
    }
}

async fn handle_start(
    manager: &CodingAgentManager,
    agent_type: CodingAgentType,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let prompt = args["prompt"]
        .as_str()
        .ok_or("Missing required field: prompt")?;

    let working_dir = args["workingDirectory"]
        .as_str()
        .map(std::path::PathBuf::from);

    let model = args["model"].as_str().map(String::from);

    let permission_mode = args["permissionMode"]
        .as_str()
        .and_then(parse_permission_mode);

    let result = manager
        .start_session(agent_type, client_id, prompt, working_dir, model, permission_mode)
        .await
        .map_err(|e| e.to_mcp_error())?;

    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

async fn handle_say(
    manager: &CodingAgentManager,
    _agent_type: CodingAgentType,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let session_id = args["sessionId"]
        .as_str()
        .ok_or("Missing required field: sessionId")?;
    let message = args["message"]
        .as_str()
        .ok_or("Missing required field: message")?;
    let permission_mode = args["permissionMode"]
        .as_str()
        .and_then(parse_permission_mode);

    let result = manager
        .say(session_id, client_id, message, permission_mode)
        .await
        .map_err(|e| e.to_mcp_error())?;

    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

async fn handle_status(
    manager: &CodingAgentManager,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let session_id = args["sessionId"]
        .as_str()
        .ok_or("Missing required field: sessionId")?;
    let output_lines = args["outputLines"].as_u64().map(|n| n as usize);

    let result = manager
        .status(session_id, client_id, output_lines)
        .await
        .map_err(|e| e.to_mcp_error())?;

    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

async fn handle_respond(
    manager: &CodingAgentManager,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let session_id = args["sessionId"]
        .as_str()
        .ok_or("Missing required field: sessionId")?;
    let question_id = args["id"]
        .as_str()
        .ok_or("Missing required field: id")?;
    let answers: Vec<String> = args["answers"]
        .as_array()
        .ok_or("Missing required field: answers")?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    let result = manager
        .respond(session_id, client_id, question_id, answers)
        .await
        .map_err(|e| e.to_mcp_error())?;

    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

async fn handle_interrupt(
    manager: &CodingAgentManager,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let session_id = args["sessionId"]
        .as_str()
        .ok_or("Missing required field: sessionId")?;

    let result = manager
        .interrupt(session_id, client_id)
        .await
        .map_err(|e| e.to_mcp_error())?;

    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

async fn handle_list(
    manager: &CodingAgentManager,
    agent_type: CodingAgentType,
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let limit = args["limit"].as_u64().map(|n| n as usize);

    let sessions = manager
        .list_sessions(client_id, Some(agent_type), limit)
        .await;

    let result = ListResponse { sessions };
    Ok(Some(serde_json::to_value(result).unwrap_or_default()))
}

fn parse_permission_mode(s: &str) -> Option<CodingPermissionMode> {
    match s {
        "auto" => Some(CodingPermissionMode::Auto),
        "supervised" => Some(CodingPermissionMode::Supervised),
        "plan" => Some(CodingPermissionMode::Plan),
        _ => None,
    }
}

/// Information about available coding agents for gateway instructions
pub struct CodingAgentInfo {
    pub name: String,
    pub tool_prefix: String,
}

/// Build instruction text about available coding agents
pub fn build_coding_agents_instructions(
    manager: &CodingAgentManager,
    permissions: &CodingAgentsPermissions,
) -> Option<String> {
    let mut agents: Vec<CodingAgentInfo> = Vec::new();

    for agent_type in CodingAgentType::all() {
        if !manager.is_agent_enabled(*agent_type) {
            continue;
        }
        let perm = permissions.resolve_agent(agent_type.tool_prefix());
        if !perm.is_enabled() {
            continue;
        }
        agents.push(CodingAgentInfo {
            name: agent_type.display_name().to_string(),
            tool_prefix: agent_type.tool_prefix().to_string(),
        });
    }

    if agents.is_empty() {
        return None;
    }

    let mut text = String::from("\n\n## AI Coding Agents\n\nYou have access to the following AI coding agents. Each agent can be started with a prompt, and you can interact with it through a session-based API.\n\nAvailable agents:\n");

    for agent in &agents {
        text.push_str(&format!(
            "- **{}**: Use `{}_start` to begin a session, `{}_say` to send messages, `{}_status` to check progress, `{}_respond` to answer questions, `{}_interrupt` to stop, `{}_list` to see sessions.\n",
            agent.name,
            agent.tool_prefix,
            agent.tool_prefix,
            agent.tool_prefix,
            agent.tool_prefix,
            agent.tool_prefix,
            agent.tool_prefix,
        ));
    }

    text.push_str("\nWorkflow: Start a session → poll status → respond to questions → get results.\n");

    Some(text)
}
