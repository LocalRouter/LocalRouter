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

/// Extract the agent type from a tool name.
/// Matches on `{prefix}_` boundary to avoid false positives
/// (e.g., `copilot_start` must not match a hypothetical `cop` prefix).
pub fn agent_type_from_tool(tool_name: &str) -> Option<CodingAgentType> {
    // Sort by prefix length descending so longer prefixes match first
    // (e.g., "qwen_code" before "qwen" if such overlap existed)
    let mut agents = CodingAgentType::all().to_vec();
    agents.sort_by_key(|b| std::cmp::Reverse(b.tool_prefix().len()));

    agents.into_iter().find(|agent| {
        let prefix_with_sep = format!("{}_", agent.tool_prefix());
        tool_name.starts_with(&prefix_with_sep)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::{CodingAgentConfig, CodingAgentsConfig, PermissionState};

    fn test_manager() -> CodingAgentManager {
        let config = CodingAgentsConfig {
            agents: vec![
                CodingAgentConfig {
                    agent_type: CodingAgentType::ClaudeCode,
                    working_directory: None,
                    model_id: None,
                    permission_mode: CodingPermissionMode::Supervised,
                    env: Default::default(),
                    binary_path: None,
                },
                CodingAgentConfig {
                    agent_type: CodingAgentType::Codex,
                    working_directory: None,
                    model_id: None,
                    permission_mode: CodingPermissionMode::Auto,
                    env: Default::default(),
                    binary_path: None,
                },
                CodingAgentConfig {
                    agent_type: CodingAgentType::GeminiCli,
                    working_directory: None,
                    model_id: None,
                    permission_mode: CodingPermissionMode::Supervised,
                    env: Default::default(),
                    binary_path: None,
                },
            ],
            default_working_directory: None,
            max_concurrent_sessions: 10,
            output_buffer_size: 100,
        };
        CodingAgentManager::new(config)
    }

    // ── is_coding_agent_tool ──

    #[test]
    fn test_is_coding_agent_tool_valid() {
        assert!(is_coding_agent_tool("claude_code_start"));
        assert!(is_coding_agent_tool("claude_code_say"));
        assert!(is_coding_agent_tool("claude_code_status"));
        assert!(is_coding_agent_tool("claude_code_respond"));
        assert!(is_coding_agent_tool("claude_code_interrupt"));
        assert!(is_coding_agent_tool("claude_code_list"));
        assert!(is_coding_agent_tool("codex_start"));
        assert!(is_coding_agent_tool("gemini_cli_start"));
        assert!(is_coding_agent_tool("amp_say"));
        assert!(is_coding_agent_tool("aider_status"));
    }

    #[test]
    fn test_is_coding_agent_tool_invalid() {
        assert!(!is_coding_agent_tool("random_tool"));
        assert!(!is_coding_agent_tool("skill_something_run"));
        assert!(!is_coding_agent_tool("claude_code_unknown"));
        assert!(!is_coding_agent_tool(""));
    }

    // ── agent_type_from_tool ──

    #[test]
    fn test_agent_type_from_tool_valid() {
        assert_eq!(
            agent_type_from_tool("claude_code_start"),
            Some(CodingAgentType::ClaudeCode)
        );
        assert_eq!(
            agent_type_from_tool("codex_say"),
            Some(CodingAgentType::Codex)
        );
        assert_eq!(
            agent_type_from_tool("gemini_cli_status"),
            Some(CodingAgentType::GeminiCli)
        );
        assert_eq!(
            agent_type_from_tool("amp_respond"),
            Some(CodingAgentType::Amp)
        );
        assert_eq!(
            agent_type_from_tool("aider_list"),
            Some(CodingAgentType::Aider)
        );
        assert_eq!(
            agent_type_from_tool("copilot_interrupt"),
            Some(CodingAgentType::Copilot)
        );
    }

    #[test]
    fn test_agent_type_from_tool_invalid() {
        assert_eq!(agent_type_from_tool("random_start"), None);
        assert_eq!(agent_type_from_tool(""), None);
    }

    #[test]
    fn test_agent_type_from_tool_boundary_match() {
        // "qwen_code_start" should match "qwen_code", not be confused
        assert_eq!(
            agent_type_from_tool("qwen_code_start"),
            Some(CodingAgentType::QwenCode)
        );
        // "copilot_start" must not match anything shorter
        assert_eq!(
            agent_type_from_tool("copilot_start"),
            Some(CodingAgentType::Copilot)
        );
    }

    // ── action_from_tool ──

    #[test]
    fn test_action_from_tool() {
        assert_eq!(action_from_tool("claude_code_start"), Some("start"));
        assert_eq!(action_from_tool("codex_say"), Some("say"));
        assert_eq!(action_from_tool("amp_status"), Some("status"));
        assert_eq!(action_from_tool("aider_respond"), Some("respond"));
        assert_eq!(action_from_tool("copilot_interrupt"), Some("interrupt"));
        assert_eq!(action_from_tool("droid_list"), Some("list"));
        assert_eq!(action_from_tool("unknown_tool"), None);
    }

    // ── build_coding_agent_tools ──

    #[test]
    fn test_build_coding_agent_tools_all_allowed() {
        let manager = test_manager();
        let permissions = CodingAgentsPermissions {
            global: PermissionState::Allow,
            agents: Default::default(),
        };

        let tools = build_coding_agent_tools(&manager, &permissions);
        // Each installed agent gets 6 tools
        let installed_count = CodingAgentManager::detect_installed_agents().len();
        assert_eq!(tools.len(), installed_count * 6);

        // Every tool should have a valid prefix from an installed agent
        for tool in &tools {
            assert!(is_coding_agent_tool(&tool.name), "Invalid tool name: {}", tool.name);
        }
    }

    #[test]
    fn test_build_coding_agent_tools_no_access() {
        let manager = test_manager();
        let permissions = CodingAgentsPermissions {
            global: PermissionState::Off,
            agents: Default::default(),
        };

        let tools = build_coding_agent_tools(&manager, &permissions);
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_coding_agent_tools_per_agent_override() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();

        // Turn off all installed agents via per-agent override
        let mut agent_overrides = std::collections::HashMap::new();
        for at in &installed {
            agent_overrides.insert(at.tool_prefix().to_string(), PermissionState::Off);
        }

        let permissions = CodingAgentsPermissions {
            global: PermissionState::Allow,
            agents: agent_overrides,
        };

        let tools = build_coding_agent_tools(&manager, &permissions);
        // All agents overridden to Off => no tools
        assert!(tools.is_empty());
    }

    #[test]
    fn test_build_tools_for_agent_generates_six_tools() {
        let tools = build_tools_for_agent(CodingAgentType::ClaudeCode);
        assert_eq!(tools.len(), 6);

        let expected_names = vec![
            "claude_code_start",
            "claude_code_say",
            "claude_code_status",
            "claude_code_respond",
            "claude_code_interrupt",
            "claude_code_list",
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
        let start = tools.iter().find(|t| t.name == "claude_code_start").unwrap();
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
        let result = handle_coding_agent_tool_call("unknown_start", &json!({}), &manager, "c1")
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
            let tool_name = format!("{}_start", agent_type.tool_prefix());
            let result = handle_coding_agent_tool_call(
                &tool_name,
                &json!({"prompt": "test"}),
                &manager,
                "c1",
            )
            .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not enabled"));
        }
        // If all agents are installed, this test is a no-op (fine for CI)
    }

    #[tokio::test]
    async fn test_handle_tool_call_list_empty() {
        let manager = test_manager();
        let installed = CodingAgentManager::detect_installed_agents();
        if installed.is_empty() {
            return; // Skip if no agents installed
        }
        let tool_name = format!("{}_list", installed[0].tool_prefix());
        let result = handle_coding_agent_tool_call(&tool_name, &json!({}), &manager, "c1").await;
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
        let tool_name = format!("{}_status", installed[0].tool_prefix());
        let result = handle_coding_agent_tool_call(
            &tool_name,
            &json!({"sessionId": "nonexistent"}),
            &manager,
            "c1",
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
        let tool_name = format!("{}_start", installed[0].tool_prefix());
        let result =
            handle_coding_agent_tool_call(&tool_name, &json!({}), &manager, "c1").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("prompt"));
    }
}

