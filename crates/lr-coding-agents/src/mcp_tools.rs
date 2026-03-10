//! MCP tool generation and dispatch for coding agents.
//!
//! Exposes 6 unified tools: `coding_agent_start`, `coding_agent_say`,
//! `coding_agent_status`, `coding_agent_respond`, `coding_agent_interrupt`, `coding_agent_list`.
//! The selected agent type is resolved from the client's session, not from the tool name.

use crate::manager::CodingAgentManager;
use crate::types::*;
use lr_config::{CodingAgentType, CodingPermissionMode, PermissionState};
use lr_types::mcp_types::McpTool;
use serde_json::{json, Value};

/// Unified tool name prefix
const TOOL_PREFIX: &str = "coding_agent_";

/// All valid tool suffixes
const TOOL_SUFFIXES: &[&str] = &["start", "say", "status", "respond", "interrupt", "list"];

/// Check if a tool name belongs to the coding agent system
pub fn is_coding_agent_tool(tool_name: &str) -> bool {
    if let Some(suffix) = tool_name.strip_prefix(TOOL_PREFIX) {
        TOOL_SUFFIXES.contains(&suffix)
    } else {
        false
    }
}

/// Extract the action suffix from a tool name (e.g., "start", "say", "status")
fn action_from_tool(tool_name: &str) -> Option<&str> {
    let suffix = tool_name.strip_prefix(TOOL_PREFIX)?;
    if TOOL_SUFFIXES.contains(&suffix) {
        Some(suffix)
    } else {
        None
    }
}

/// Build MCP tools for the selected coding agent
pub fn build_coding_agent_tools(
    manager: &CodingAgentManager,
    permission: &PermissionState,
    agent_type: Option<CodingAgentType>,
) -> Vec<McpTool> {
    if !permission.is_enabled() {
        return Vec::new();
    }

    let Some(agent_type) = agent_type else {
        return Vec::new();
    };

    if !manager.is_agent_enabled(agent_type) {
        return Vec::new();
    }

    build_tools_for_agent(agent_type)
}

/// Build the 6 tools with unified `coding_agent_` prefix.
/// Public so the UI can display tool definitions without starting an agent.
pub fn build_tools_for_agent(agent_type: CodingAgentType) -> Vec<McpTool> {
    let name = agent_type.display_name();

    vec![
        // coding_agent_start
        McpTool {
            name: format!("{}start", TOOL_PREFIX),
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
                        "description": "Working directory for the session. If omitted, a temporary directory is created."
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
        // coding_agent_say
        McpTool {
            name: format!("{}say", TOOL_PREFIX),
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
        // coding_agent_status
        McpTool {
            name: format!("{}status", TOOL_PREFIX),
            description: Some(format!(
                "Get current status and recent output of a {} session. Use wait=true to block until the session needs attention (done, awaiting_input, error, interrupted) instead of polling in a loop.",
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
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "If true, blocks until the session needs attention (done, awaiting_input, error, interrupted) instead of returning immediately. Default: false"
                    },
                    "timeoutSeconds": {
                        "type": "number",
                        "description": "Max seconds to wait when wait=true (default: 300, max: 600). Ignored when wait=false."
                    }
                },
                "required": ["sessionId"]
            }),
        },
        // coding_agent_respond
        McpTool {
            name: format!("{}respond", TOOL_PREFIX),
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
        // coding_agent_interrupt
        McpTool {
            name: format!("{}interrupt", TOOL_PREFIX),
            description: Some(format!("Interrupt a running {} session", name)),
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
        // coding_agent_list
        McpTool {
            name: format!("{}list", TOOL_PREFIX),
            description: Some(format!("List all {} sessions for this client", name)),
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
/// The `agent_type` is resolved from the client's session, not from the tool name.
/// Returns `Ok(Some(value))` on success, `Ok(None)` if tool not found,
/// or `Err` on error.
pub async fn handle_coding_agent_tool_call(
    tool_name: &str,
    arguments: &Value,
    manager: &CodingAgentManager,
    client_id: &str,
    agent_type: CodingAgentType,
) -> Result<Option<Value>, String> {
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
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    let model = args["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let permission_mode = args["permissionMode"]
        .as_str()
        .and_then(parse_permission_mode);

    let result = manager
        .start_session(
            agent_type,
            client_id,
            prompt,
            working_dir,
            model,
            permission_mode,
        )
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
    let wait = args["wait"].as_bool().unwrap_or(false);

    let result = if wait {
        let timeout_secs = args["timeoutSeconds"].as_u64().unwrap_or(300).min(600);
        let timeout = std::time::Duration::from_secs(timeout_secs);
        manager
            .wait_for_non_active(session_id, client_id, timeout, output_lines)
            .await
            .map_err(|e| e.to_mcp_error())?
    } else {
        manager
            .status(session_id, client_id, output_lines)
            .await
            .map_err(|e| e.to_mcp_error())?
    };

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
    let question_id = args["id"].as_str().ok_or("Missing required field: id")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::CodingAgentsConfig;

    fn test_manager() -> CodingAgentManager {
        let config = CodingAgentsConfig::default();
        CodingAgentManager::new(config)
    }

    // ── is_coding_agent_tool ──

    #[test]
    fn test_is_coding_agent_tool_valid() {
        assert!(is_coding_agent_tool("coding_agent_start"));
        assert!(is_coding_agent_tool("coding_agent_say"));
        assert!(is_coding_agent_tool("coding_agent_status"));
        assert!(is_coding_agent_tool("coding_agent_respond"));
        assert!(is_coding_agent_tool("coding_agent_interrupt"));
        assert!(is_coding_agent_tool("coding_agent_list"));
    }

    #[test]
    fn test_is_coding_agent_tool_invalid() {
        assert!(!is_coding_agent_tool("random_tool"));
        assert!(!is_coding_agent_tool("skill_something_run"));
        assert!(!is_coding_agent_tool("coding_agent_unknown"));
        assert!(!is_coding_agent_tool("claude_code_start"));
        assert!(!is_coding_agent_tool(""));
    }

    // ── action_from_tool ──

    #[test]
    fn test_action_from_tool() {
        assert_eq!(action_from_tool("coding_agent_start"), Some("start"));
        assert_eq!(action_from_tool("coding_agent_say"), Some("say"));
        assert_eq!(action_from_tool("coding_agent_status"), Some("status"));
        assert_eq!(action_from_tool("coding_agent_respond"), Some("respond"));
        assert_eq!(
            action_from_tool("coding_agent_interrupt"),
            Some("interrupt")
        );
        assert_eq!(action_from_tool("coding_agent_list"), Some("list"));
        assert_eq!(action_from_tool("unknown_tool"), None);
    }

    // ── build_coding_agent_tools ──

    #[test]
    fn test_build_coding_agent_tools_allowed_with_type() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return;
        }

        let tools = build_coding_agent_tools(&manager, &PermissionState::Allow, Some(installed[0]));
        assert_eq!(tools.len(), 6);

        for tool in &tools {
            assert!(
                is_coding_agent_tool(&tool.name),
                "Invalid tool name: {}",
                tool.name
            );
        }
    }

    #[test]
    fn test_build_coding_agent_tools_off() {
        let manager = test_manager();
        let tools = build_coding_agent_tools(
            &manager,
            &PermissionState::Off,
            Some(CodingAgentType::ClaudeCode),
        );
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_coding_agent_tools_no_type() {
        let manager = test_manager();
        let tools = build_coding_agent_tools(&manager, &PermissionState::Allow, None);
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_tools_for_agent_generates_six_tools() {
        let tools = build_tools_for_agent(CodingAgentType::ClaudeCode);
        assert_eq!(tools.len(), 6);

        let expected_names = vec![
            "coding_agent_start",
            "coding_agent_say",
            "coding_agent_status",
            "coding_agent_respond",
            "coding_agent_interrupt",
            "coding_agent_list",
        ];

        for expected in expected_names {
            assert!(
                tools.iter().any(|t| t.name == expected),
                "Missing tool: {}",
                expected
            );
        }
    }

    #[test]
    fn test_build_tools_for_agent_has_descriptions() {
        let tools = build_tools_for_agent(CodingAgentType::Codex);
        for tool in &tools {
            assert!(
                tool.description.is_some(),
                "Tool {} has no description",
                tool.name
            );
            assert!(
                tool.description.as_ref().unwrap().contains("Codex"),
                "Tool {} description should mention agent name",
                tool.name
            );
        }
    }

    #[test]
    fn test_build_tools_start_requires_prompt() {
        let tools = build_tools_for_agent(CodingAgentType::ClaudeCode);
        let start = tools
            .iter()
            .find(|t| t.name == "coding_agent_start")
            .unwrap();
        let required = start.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "prompt"));
    }

    // ── parse_permission_mode ──

    #[test]
    fn test_parse_permission_mode() {
        assert_eq!(
            parse_permission_mode("auto"),
            Some(CodingPermissionMode::Auto)
        );
        assert_eq!(
            parse_permission_mode("supervised"),
            Some(CodingPermissionMode::Supervised)
        );
        assert_eq!(
            parse_permission_mode("plan"),
            Some(CodingPermissionMode::Plan)
        );
        assert_eq!(parse_permission_mode("unknown"), None);
        assert_eq!(parse_permission_mode(""), None);
    }

    // ── handle_coding_agent_tool_call ──

    #[tokio::test]
    async fn test_handle_tool_call_unknown_tool() {
        let manager = test_manager();
        let result = handle_coding_agent_tool_call(
            "unknown_start",
            &json!({}),
            &manager,
            "c1",
            CodingAgentType::ClaudeCode,
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // None = not a coding agent tool
    }

    #[tokio::test]
    async fn test_handle_tool_call_unavailable_agent() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();

        // Find an agent that is NOT installed
        let unavailable = CodingAgentType::all()
            .iter()
            .find(|at| !installed.contains(at));

        if let Some(agent_type) = unavailable {
            let result = handle_coding_agent_tool_call(
                "coding_agent_start",
                &json!({"prompt": "test"}),
                &manager,
                "c1",
                *agent_type,
            )
            .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not enabled"));
        }
    }

    #[tokio::test]
    async fn test_handle_tool_call_list_empty() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return;
        }
        let result = handle_coding_agent_tool_call(
            "coding_agent_list",
            &json!({}),
            &manager,
            "c1",
            installed[0],
        )
        .await;
        assert!(result.is_ok());
        let value = result.unwrap().unwrap();
        let sessions = value["sessions"].as_array().unwrap();
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn test_handle_tool_call_status_missing_session() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return;
        }
        let result = handle_coding_agent_tool_call(
            "coding_agent_status",
            &json!({"sessionId": "nonexistent"}),
            &manager,
            "c1",
            installed[0],
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_tool_call_start_missing_prompt() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return;
        }
        let result = handle_coding_agent_tool_call(
            "coding_agent_start",
            &json!({}),
            &manager,
            "c1",
            installed[0],
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("prompt"));
    }
}
