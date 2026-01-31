//! MCP tool definitions for skills
//!
//! Generates McpTool definitions that get merged into the gateway's tools/list response.

use super::executor::ScriptExecutor;
use super::manager::SkillManager;
use super::types::SkillDefinition;
use lr_config::SkillsAccess;
use lr_mcp::protocol::McpTool;
use serde_json::json;

/// Build a show-skill tool for a specific skill
fn build_show_skill_tool(skill: &SkillDefinition) -> McpTool {
    let name = format!("show-skill_{}", skill.metadata.name);
    let description = skill
        .metadata
        .description
        .clone()
        .unwrap_or_else(|| format!("Show instructions for skill '{}'", skill.metadata.name));

    McpTool {
        name,
        description: Some(format!(
            "Show full instructions and files for skill '{}'. {}",
            skill.metadata.name, description
        )),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

/// Build the shared get-skill-resource tool
fn build_get_resource_tool() -> McpTool {
    McpTool {
        name: "get-skill-resource".to_string(),
        description: Some(
            "Get the content of a reference file, asset, or other resource from a skill directory."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "skill_name": {
                    "type": "string",
                    "description": "Name of the skill"
                },
                "resource": {
                    "type": "string",
                    "description": "Relative path to the resource file (e.g., 'references/api.md', 'assets/logo.png')"
                }
            },
            "required": ["skill_name", "resource"],
            "additionalProperties": false
        }),
    }
}

/// Build the shared run-skill-script tool
fn build_run_script_tool() -> McpTool {
    McpTool {
        name: "run-skill-script".to_string(),
        description: Some(
            "Execute a script from a skill's scripts/ directory. Supports sync and async execution."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "skill_name": {
                    "type": "string",
                    "description": "Name of the skill"
                },
                "script": {
                    "type": "string",
                    "description": "Relative path to the script (e.g., 'scripts/run.sh')"
                },
                "command": {
                    "type": "string",
                    "description": "Command interpreter to use (auto-detected from extension if omitted). Examples: 'python3', 'bash', 'node'"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 10, max: 20 for sync, 3600 for async)"
                },
                "async": {
                    "type": "boolean",
                    "description": "If true, run asynchronously and return a PID for status polling (default: false)"
                },
                "tail": {
                    "type": "integer",
                    "description": "Number of output lines to return (default: 30)"
                }
            },
            "required": ["skill_name", "script"],
            "additionalProperties": false
        }),
    }
}

/// Build the shared get-skill-script-run tool
fn build_get_script_run_tool() -> McpTool {
    McpTool {
        name: "get-skill-script-run".to_string(),
        description: Some(
            "Get the status and output of a previously started async script execution.".to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "integer",
                    "description": "Process ID returned by run-skill-script with async=true"
                },
                "tail": {
                    "type": "integer",
                    "description": "Number of output lines to return (default: 30)"
                }
            },
            "required": ["pid"],
            "additionalProperties": false
        }),
    }
}

/// Generate all skill MCP tools for a client's allowed skills
///
/// Three-tier access control:
/// 1. Discovered — skill exists in manager's list
/// 2. Globally enabled — skill.enabled == true
/// 3. Client-allowed — client's SkillsAccess matches by source_path
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    access: &SkillsAccess,
) -> Vec<McpTool> {
    if !access.has_any_access() {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();

    // Filter: enabled AND client-allowed
    let allowed: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && access.can_access_by_source(&s.source_path))
        .collect();

    if allowed.is_empty() {
        return Vec::new();
    }

    let mut tools = Vec::new();

    // Add a show-skill tool for each allowed skill
    for skill in &allowed {
        tools.push(build_show_skill_tool(skill));
    }

    // Add shared tools (only once, not per-skill)
    tools.push(build_get_resource_tool());
    tools.push(build_run_script_tool());
    tools.push(build_get_script_run_tool());

    tools
}

/// Check if a skill name is allowed under the given access control
pub fn is_skill_allowed(
    skill_manager: &SkillManager,
    skill_name: &str,
    access: &SkillsAccess,
) -> bool {
    if !access.has_any_access() {
        return false;
    }
    match skill_manager.get(skill_name) {
        Some(skill) => skill.enabled && access.can_access_by_source(&skill.source_path),
        None => false,
    }
}

/// Handle a skill tool call
///
/// Returns Ok(Some(response_json)) if the tool was a skill tool,
/// Ok(None) if it's not a skill tool (should be routed elsewhere).
pub async fn handle_skill_tool_call(
    tool_name: &str,
    arguments: &serde_json::Value,
    skill_manager: &SkillManager,
    script_executor: &ScriptExecutor,
    access: &SkillsAccess,
) -> Result<Option<serde_json::Value>, String> {
    // Check if it's a show-skill tool
    if let Some(skill_name) = tool_name.strip_prefix("show-skill_") {
        if !is_skill_allowed(skill_manager, skill_name, access) {
            return Err(format!("Skill '{}' is not allowed for this client", skill_name));
        }

        let skill = skill_manager
            .get(skill_name)
            .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

        return Ok(Some(build_show_skill_response(&skill)));
    }

    match tool_name {
        "get-skill-resource" => {
            let skill_name = arguments
                .get("skill_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'skill_name' argument")?;

            if !is_skill_allowed(skill_manager, skill_name, access) {
                return Err(format!("Skill '{}' is not allowed for this client", skill_name));
            }

            let resource = arguments
                .get("resource")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'resource' argument")?;

            let content = skill_manager.get_resource(skill_name, resource)?;

            Ok(Some(json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            })))
        }

        "run-skill-script" => {
            let skill_name = arguments
                .get("skill_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'skill_name' argument")?;

            if !is_skill_allowed(skill_manager, skill_name, access) {
                return Err(format!("Skill '{}' is not allowed for this client", skill_name));
            }

            let script = arguments
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'script' argument")?;

            let command = arguments.get("command").and_then(|v| v.as_str());
            let timeout = arguments.get("timeout").and_then(|v| v.as_u64());
            let is_async = arguments
                .get("async")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tail = arguments
                .get("tail")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let skill_dir = skill_manager
                .get_skill_dir(skill_name)
                .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

            if is_async {
                let pid = script_executor
                    .run_async(&skill_dir, script, command, timeout)
                    .await?;

                Ok(Some(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Script started asynchronously. PID: {}\nUse get-skill-script-run with this PID to check status.", pid)
                    }],
                    "pid": pid
                })))
            } else {
                let result = script_executor
                    .run_sync(&skill_dir, script, command, timeout, tail)
                    .await?;

                let mut text = String::new();
                if result.timed_out {
                    text.push_str("⚠️ Script timed out\n\n");
                }
                if let Some(code) = result.exit_code {
                    text.push_str(&format!("Exit code: {}\n\n", code));
                }
                if !result.stdout.is_empty() {
                    text.push_str(&format!("--- stdout ---\n{}\n", result.stdout));
                }
                if !result.stderr.is_empty() {
                    text.push_str(&format!("--- stderr ---\n{}\n", result.stderr));
                }

                Ok(Some(json!({
                    "content": [{
                        "type": "text",
                        "text": text.trim()
                    }],
                    "exit_code": result.exit_code,
                    "timed_out": result.timed_out
                })))
            }
        }

        "get-skill-script-run" => {
            let pid = arguments
                .get("pid")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'pid' argument")? as u32;
            let tail = arguments
                .get("tail")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let status = script_executor.get_async_status(pid, tail).await?;

            let mut text = String::new();
            text.push_str(&format!(
                "PID: {} | Status: {}\n",
                status.pid,
                if status.running {
                    "Running"
                } else if status.timed_out {
                    "Timed out"
                } else {
                    "Completed"
                }
            ));
            if let Some(code) = status.exit_code {
                text.push_str(&format!("Exit code: {}\n", code));
            }
            if !status.stdout.is_empty() {
                text.push_str(&format!("\n--- stdout ---\n{}\n", status.stdout));
            }
            if !status.stderr.is_empty() {
                text.push_str(&format!("\n--- stderr ---\n{}\n", status.stderr));
            }

            Ok(Some(json!({
                "content": [{
                    "type": "text",
                    "text": text.trim()
                }],
                "running": status.running,
                "exit_code": status.exit_code,
                "timed_out": status.timed_out
            })))
        }

        _ => Ok(None), // Not a skill tool
    }
}

/// Build the response for show-skill tool
fn build_show_skill_response(skill: &SkillDefinition) -> serde_json::Value {
    let mut text = String::new();

    // Header
    text.push_str(&format!("# Skill: {}\n\n", skill.metadata.name));

    if let Some(desc) = &skill.metadata.description {
        text.push_str(&format!("{}\n\n", desc));
    }

    if let Some(version) = &skill.metadata.version {
        text.push_str(&format!("**Version:** {}\n", version));
    }

    if let Some(author) = &skill.metadata.author {
        text.push_str(&format!("**Author:** {}\n", author));
    }

    if !skill.metadata.tags.is_empty() {
        text.push_str(&format!("**Tags:** {}\n", skill.metadata.tags.join(", ")));
    }

    text.push('\n');

    // File listings with suggested tool calls
    if !skill.scripts.is_empty() {
        text.push_str("## Scripts\n\n");
        for script in &skill.scripts {
            text.push_str(&format!(
                "- `{}` — run with `run-skill-script(skill_name=\"{}\", script=\"{}\")`\n",
                script, skill.metadata.name, script
            ));
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        for reference in &skill.references {
            text.push_str(&format!(
                "- `{}` — read with `get-skill-resource(skill_name=\"{}\", resource=\"{}\")`\n",
                reference, skill.metadata.name, reference
            ));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        for asset in &skill.assets {
            text.push_str(&format!(
                "- `{}` — read with `get-skill-resource(skill_name=\"{}\", resource=\"{}\")`\n",
                asset, skill.metadata.name, asset
            ));
        }
        text.push('\n');
    }

    // Full SKILL.md body
    text.push_str("## Instructions\n\n");
    text.push_str(&skill.body);

    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}
