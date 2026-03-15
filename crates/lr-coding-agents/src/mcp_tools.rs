//! MCP tool generation and dispatch for coding agents.
//!
//! Exposes 4 unified tools with configurable prefix:
//! `{prefix}Start`, `{prefix}Say`, `{prefix}Status`, `{prefix}List`
//!
//! Default prefix: "Agent" → tools: AgentStart, AgentSay, AgentStatus, AgentList

use crate::manager::CodingAgentManager;
use crate::types::*;
use lr_config::{CodingAgentType, CodingPermissionMode, PermissionState};
use lr_types::mcp_types::McpTool;
use serde_json::{json, Value};

/// All valid tool suffixes (lowercase form)
const TOOL_SUFFIXES: &[&str] = &["start", "say", "status", "list"];

/// Build a tool name from prefix + suffix.
///
/// If prefix ends with a non-alphanumeric char, suffix stays lowercase:
///   "agent_" + "start" = "agent_start"
/// If prefix ends with an alphanumeric char, suffix is PascalCase:
///   "Agent" + "start" = "AgentStart"
pub fn tool_name(prefix: &str, suffix: &str) -> String {
    match prefix.chars().last() {
        Some(c) if c.is_alphanumeric() => {
            let mut capitalized = suffix.to_string();
            if let Some(first) = capitalized.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            format!("{}{}", prefix, capitalized)
        }
        _ => {
            format!("{}{}", prefix, suffix)
        }
    }
}

/// Return all tool names for the configured prefix.
pub fn all_tool_names(prefix: &str) -> Vec<String> {
    TOOL_SUFFIXES.iter().map(|s| tool_name(prefix, s)).collect()
}

/// Check if a tool name belongs to the coding agent system
pub fn is_coding_agent_tool(name: &str, prefix: &str) -> bool {
    action_from_tool(name, prefix).is_some()
}

/// Extract the action suffix from a tool name (e.g., "start", "say", "status").
///
/// Avoids heap allocations by checking prefix match + suffix extraction directly
/// instead of building tool_name strings for comparison.
fn action_from_tool(name: &str, prefix: &str) -> Option<&'static str> {
    let rest = name.strip_prefix(prefix)?;

    let alphanumeric_prefix = prefix.chars().last().map_or(false, |c| c.is_alphanumeric());

    for suffix in TOOL_SUFFIXES {
        if alphanumeric_prefix {
            // PascalCase: compare case-insensitively (all suffixes are ASCII lowercase)
            if rest.eq_ignore_ascii_case(suffix)
                && rest.starts_with(|c: char| c.is_ascii_uppercase())
            {
                return Some(suffix);
            }
        } else if rest == *suffix {
            return Some(suffix);
        }
    }
    None
}

/// Build MCP tools for the selected coding agent
pub fn build_coding_agent_tools(
    manager: &CodingAgentManager,
    permission: &PermissionState,
    agent_type: Option<CodingAgentType>,
    prefix: &str,
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

    build_tools_for_agent(agent_type, prefix)
}

/// Build the 4 tools for an agent type.
pub fn build_tools_for_agent(agent_type: CodingAgentType, prefix: &str) -> Vec<McpTool> {
    let name = agent_type.display_name();

    vec![
        // Start
        McpTool {
            name: tool_name(prefix, "start"),
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
        // Say (combined say + interrupt)
        McpTool {
            name: tool_name(prefix, "say"),
            description: Some(format!(
                "Send a message to a {} session. Can interrupt current work and/or resume completed sessions with context preserved.",
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
                        "description": "Message to send. If session is done/error, resumes with context."
                    },
                    "interrupt": {
                        "type": "boolean",
                        "description": "If true, interrupts current work before sending message. If true with no message, just stops the agent."
                    },
                    "permissionMode": {
                        "type": "string",
                        "enum": ["auto", "supervised", "plan"],
                        "description": "Switch permission mode"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        // Status
        McpTool {
            name: tool_name(prefix, "status"),
            description: Some(format!(
                "Get current status and recent output of a {} session. Use wait=true to block until the session reaches a terminal state (done, error, interrupted) instead of polling in a loop.",
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
                        "description": "If true, blocks until the session reaches a terminal state (done, error, interrupted) instead of returning immediately. Default: false"
                    },
                    "timeoutSeconds": {
                        "type": "number",
                        "description": "Max seconds to wait when wait=true (default: 300, max: 600). Ignored when wait=false."
                    }
                },
                "required": ["sessionId"]
            }),
        },
        // List
        McpTool {
            name: tool_name(prefix, "list"),
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
pub async fn handle_coding_agent_tool_call(
    tool_name_str: &str,
    arguments: &Value,
    manager: &CodingAgentManager,
    client_id: &str,
    agent_type: CodingAgentType,
    prefix: &str,
) -> Result<Option<Value>, String> {
    let action = match action_from_tool(tool_name_str, prefix) {
        Some(a) => a,
        None => return Ok(None),
    };

    if !manager.is_agent_enabled(agent_type) {
        return Err(format!("{} is not enabled", agent_type.display_name()));
    }

    match action {
        "start" => handle_start(manager, agent_type, client_id, arguments).await,
        "say" => handle_say(manager, client_id, arguments).await,
        "status" => handle_status(manager, client_id, arguments).await,
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
    client_id: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    let session_id = args["sessionId"]
        .as_str()
        .ok_or("Missing required field: sessionId")?;
    let message = args["message"].as_str();
    let interrupt = args["interrupt"].as_bool().unwrap_or(false);
    let permission_mode = args["permissionMode"]
        .as_str()
        .and_then(parse_permission_mode);

    let result = manager
        .say(session_id, client_id, message, interrupt, permission_mode)
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

    // ── tool_name generation ──

    #[test]
    fn test_tool_name_alphanumeric_prefix() {
        assert_eq!(tool_name("Agent", "start"), "AgentStart");
        assert_eq!(tool_name("Agent", "say"), "AgentSay");
        assert_eq!(tool_name("Agent", "status"), "AgentStatus");
        assert_eq!(tool_name("Agent", "list"), "AgentList");
    }

    #[test]
    fn test_tool_name_non_alphanumeric_prefix() {
        assert_eq!(tool_name("coding_agent_", "start"), "coding_agent_start");
        assert_eq!(tool_name("agent-", "say"), "agent-say");
        assert_eq!(tool_name("my_tool.", "status"), "my_tool.status");
    }

    #[test]
    fn test_tool_name_single_char_prefix() {
        assert_eq!(tool_name("A", "start"), "AStart");
        assert_eq!(tool_name("_", "start"), "_start");
    }

    // ── is_coding_agent_tool ──

    #[test]
    fn test_is_coding_agent_tool_default_prefix() {
        let prefix = "Agent";
        assert!(is_coding_agent_tool("AgentStart", prefix));
        assert!(is_coding_agent_tool("AgentSay", prefix));
        assert!(is_coding_agent_tool("AgentStatus", prefix));
        assert!(is_coding_agent_tool("AgentList", prefix));
        assert!(!is_coding_agent_tool("AgentRespond", prefix));
        assert!(!is_coding_agent_tool("AgentInterrupt", prefix));
        assert!(!is_coding_agent_tool("random_tool", prefix));
    }

    #[test]
    fn test_is_coding_agent_tool_underscore_prefix() {
        let prefix = "coding_agent_";
        assert!(is_coding_agent_tool("coding_agent_start", prefix));
        assert!(is_coding_agent_tool("coding_agent_say", prefix));
        assert!(is_coding_agent_tool("coding_agent_status", prefix));
        assert!(is_coding_agent_tool("coding_agent_list", prefix));
        assert!(!is_coding_agent_tool("coding_agent_respond", prefix));
    }

    // ── all_tool_names ──

    #[test]
    fn test_all_tool_names() {
        let names = all_tool_names("Agent");
        assert_eq!(
            names,
            vec!["AgentStart", "AgentSay", "AgentStatus", "AgentList"]
        );
    }

    // ── action_from_tool ──

    #[test]
    fn test_action_from_tool() {
        assert_eq!(action_from_tool("AgentStart", "Agent"), Some("start"));
        assert_eq!(action_from_tool("AgentSay", "Agent"), Some("say"));
        assert_eq!(action_from_tool("AgentStatus", "Agent"), Some("status"));
        assert_eq!(action_from_tool("AgentList", "Agent"), Some("list"));
        assert_eq!(action_from_tool("unknown_tool", "Agent"), None);
    }

    // ── build_coding_agent_tools ──

    #[test]
    fn test_build_coding_agent_tools_allowed_with_type() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return;
        }

        let tools = build_coding_agent_tools(
            &manager,
            &PermissionState::Allow,
            Some(installed[0]),
            "Agent",
        );
        assert_eq!(tools.len(), 4);

        for tool in &tools {
            assert!(
                is_coding_agent_tool(&tool.name, "Agent"),
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
            "Agent",
        );
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_coding_agent_tools_no_type() {
        let manager = test_manager();
        let tools = build_coding_agent_tools(&manager, &PermissionState::Allow, None, "Agent");
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_tools_generates_four_tools() {
        let tools = build_tools_for_agent(CodingAgentType::ClaudeCode, "Agent");
        assert_eq!(tools.len(), 4);

        let expected_names = vec!["AgentStart", "AgentSay", "AgentStatus", "AgentList"];
        for expected in expected_names {
            assert!(
                tools.iter().any(|t| t.name == expected),
                "Missing tool: {}",
                expected
            );
        }
    }

    #[test]
    fn test_build_tools_start_requires_prompt() {
        let tools = build_tools_for_agent(CodingAgentType::ClaudeCode, "Agent");
        let start = tools.iter().find(|t| t.name == "AgentStart").unwrap();
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
            "Agent",
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_handle_tool_call_unavailable_agent() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();

        let unavailable = CodingAgentType::all()
            .iter()
            .find(|at| !installed.contains(at));

        if let Some(agent_type) = unavailable {
            let result = handle_coding_agent_tool_call(
                "AgentStart",
                &json!({"prompt": "test"}),
                &manager,
                "c1",
                *agent_type,
                "Agent",
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
            "AgentList",
            &json!({}),
            &manager,
            "c1",
            installed[0],
            "Agent",
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
            "AgentStatus",
            &json!({"sessionId": "nonexistent"}),
            &manager,
            "c1",
            installed[0],
            "Agent",
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
            "AgentStart",
            &json!({}),
            &manager,
            "c1",
            installed[0],
            "Agent",
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("prompt"));
    }
}
