//! MCP tool definitions for skills
//!
//! Generates per-skill namespaced McpTool definitions with deferred loading.
//! Only `get_info` tools are initially visible; run/read tools appear after
//! the client calls `get_info` for each skill.

use super::executor::ScriptExecutor;
use super::manager::SkillManager;
use super::types::{sanitize_name, sanitize_tool_segment, SkillDefinition};
use lr_config::SkillsAccess;
use lr_types::McpTool;
use serde_json::json;
use std::collections::HashSet;

/// Result of handling a skill tool call.
///
/// `InfoLoaded` signals the gateway to update session state and invalidate tools cache.
pub enum SkillToolResult {
    /// Normal response (run, read, async status)
    Response(serde_json::Value),
    /// get_info was called — gateway should mark skill as loaded and invalidate cache
    InfoLoaded {
        skill_name: String,
        response: serde_json::Value,
    },
}

/// Parsed skill tool name
enum SkillToolParsed {
    GetInfo {
        skill_name: String,
    },
    Run {
        skill_name: String,
        script_file: String,
    },
    RunAsync {
        skill_name: String,
        script_file: String,
    },
    Read {
        skill_name: String,
        resource_file: String,
    },
    GetAsyncStatus,
}

// ---------------------------------------------------------------------------
// Tool builders
// ---------------------------------------------------------------------------

/// Build a `skill_{sname}_get_info` tool
fn build_get_info_tool(skill: &SkillDefinition) -> McpTool {
    let sname = sanitize_name(&skill.metadata.name);
    let description = skill
        .metadata
        .description
        .clone()
        .unwrap_or_else(|| format!("Show instructions for skill '{}'", skill.metadata.name));

    McpTool {
        name: format!("skill_{}_get_info", sname),
        description: Some(format!(
            "Show full instructions and available tools for skill '{}'. {}",
            skill.metadata.name, description
        )),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

/// Build a `skill_{sname}_run_{sfile}` tool
fn build_run_tool(skill: &SkillDefinition, script: &str) -> McpTool {
    let sname = sanitize_name(&skill.metadata.name);
    let sfile = sanitize_tool_segment(script);

    McpTool {
        name: format!("skill_{}_run_{}", sname, sfile),
        description: Some(format!(
            "Execute script '{}' from skill '{}'.",
            script, skill.metadata.name
        )),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command interpreter to use (auto-detected from extension if omitted). Examples: 'python3', 'bash', 'node'"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 10, max: 20)"
                },
                "tail": {
                    "type": "integer",
                    "description": "Number of output lines to return (default: 30)"
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Build a `skill_{sname}_run_async_{sfile}` tool
fn build_run_async_tool(skill: &SkillDefinition, script: &str) -> McpTool {
    let sname = sanitize_name(&skill.metadata.name);
    let sfile = sanitize_tool_segment(script);

    McpTool {
        name: format!("skill_{}_run_async_{}", sname, sfile),
        description: Some(format!(
            "Execute script '{}' from skill '{}' asynchronously. Returns a PID for status polling.",
            script, skill.metadata.name
        )),
        input_schema: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command interpreter to use (auto-detected from extension if omitted). Examples: 'python3', 'bash', 'node'"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 10, max: 3600)"
                },
                "tail": {
                    "type": "integer",
                    "description": "Number of output lines to return (default: 30)"
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Build a `skill_{sname}_read_{sfile}` tool
fn build_read_tool(skill: &SkillDefinition, resource: &str) -> McpTool {
    let sname = sanitize_name(&skill.metadata.name);
    let sfile = sanitize_tool_segment(resource);

    McpTool {
        name: format!("skill_{}_read_{}", sname, sfile),
        description: Some(format!(
            "Read resource '{}' from skill '{}'.",
            resource, skill.metadata.name
        )),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

/// Build the `skill_get_async_status` tool (shared across all skills)
fn build_get_async_status_tool() -> McpTool {
    McpTool {
        name: "skill_get_async_status".to_string(),
        description: Some(
            "Get the status and output of a previously started async script execution.".to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "integer",
                    "description": "Process ID returned by an async run tool"
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

// ---------------------------------------------------------------------------
// Tool list builder
// ---------------------------------------------------------------------------

/// Generate all skill MCP tools for a client's allowed skills.
///
/// - Always includes `get_info` tools for all allowed skills.
/// - Only includes run/read/run_async tools for skills in `info_loaded`.
/// - Includes `skill_get_async_status` only when `async_enabled` and any skill loaded.
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    access: &SkillsAccess,
    info_loaded: &HashSet<String>,
    async_enabled: bool,
) -> Vec<McpTool> {
    if !access.has_any_access() {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();

    // Filter: enabled AND client-allowed
    let allowed: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && access.can_access_by_name(&s.metadata.name))
        .collect();

    if allowed.is_empty() {
        return Vec::new();
    }

    let mut tools = Vec::new();
    let mut any_loaded = false;

    for skill in &allowed {
        // Always add get_info
        tools.push(build_get_info_tool(skill));

        // Only add run/read tools if info was loaded for this skill
        if info_loaded.contains(&skill.metadata.name) {
            any_loaded = true;

            for script in &skill.scripts {
                tools.push(build_run_tool(skill, script));
                if async_enabled {
                    tools.push(build_run_async_tool(skill, script));
                }
            }

            for reference in &skill.references {
                tools.push(build_read_tool(skill, reference));
            }

            for asset in &skill.assets {
                tools.push(build_read_tool(skill, asset));
            }
        }
    }

    // Add shared async status tool if async is enabled and at least one skill is loaded
    if async_enabled && any_loaded {
        tools.push(build_get_async_status_tool());
    }

    tools
}

// ---------------------------------------------------------------------------
// Tool name parsing
// ---------------------------------------------------------------------------

/// Parse a tool name into its skill tool variant.
///
/// Iterates allowed skills to match `skill_{sanitized_name}_` prefix, then
/// determines the action suffix. Returns `None` if not a skill tool.
fn parse_skill_tool_name(
    tool_name: &str,
    skill_manager: &SkillManager,
    access: &SkillsAccess,
) -> Option<SkillToolParsed> {
    // Global async status tool
    if tool_name == "skill_get_async_status" {
        return Some(SkillToolParsed::GetAsyncStatus);
    }

    if !tool_name.starts_with("skill_") {
        return None;
    }

    let all_skills = skill_manager.get_all();

    // Try to match against each allowed skill
    for skill in all_skills.iter() {
        if !skill.enabled || !access.can_access_by_name(&skill.metadata.name) {
            continue;
        }

        let sname = sanitize_name(&skill.metadata.name);
        let prefix = format!("skill_{}_", sname);

        if let Some(rest) = tool_name.strip_prefix(&prefix) {
            // get_info
            if rest == "get_info" {
                return Some(SkillToolParsed::GetInfo {
                    skill_name: skill.metadata.name.clone(),
                });
            }

            // run_async_{sfile} — check before run_ since run_async starts with run_
            if let Some(sfile) = rest.strip_prefix("run_async_") {
                // Reverse-map sanitized file name to actual script path
                if let Some(script) = reverse_map_file(sfile, &skill.scripts) {
                    return Some(SkillToolParsed::RunAsync {
                        skill_name: skill.metadata.name.clone(),
                        script_file: script,
                    });
                }
            }

            // run_{sfile}
            if let Some(sfile) = rest.strip_prefix("run_") {
                if let Some(script) = reverse_map_file(sfile, &skill.scripts) {
                    return Some(SkillToolParsed::Run {
                        skill_name: skill.metadata.name.clone(),
                        script_file: script,
                    });
                }
            }

            // read_{sfile}
            if let Some(sfile) = rest.strip_prefix("read_") {
                // Check references, then assets
                if let Some(resource) = reverse_map_file(sfile, &skill.references) {
                    return Some(SkillToolParsed::Read {
                        skill_name: skill.metadata.name.clone(),
                        resource_file: resource,
                    });
                }
                if let Some(resource) = reverse_map_file(sfile, &skill.assets) {
                    return Some(SkillToolParsed::Read {
                        skill_name: skill.metadata.name.clone(),
                        resource_file: resource,
                    });
                }
            }
        }
    }

    None
}

/// Reverse-map a sanitized file segment back to the original file path.
///
/// Compares against a list of known file paths, returning the first match.
fn reverse_map_file(sanitized: &str, files: &[String]) -> Option<String> {
    for file in files {
        if sanitize_tool_segment(file) == sanitized {
            return Some(file.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tool call handler
// ---------------------------------------------------------------------------

/// Check if a tool name matches a skill tool pattern.
pub fn is_skill_tool(tool_name: &str) -> bool {
    tool_name.starts_with("skill_")
}

/// Handle a skill tool call.
///
/// Returns `Ok(Some(result))` if the tool was a skill tool,
/// `Ok(None)` if it's not a skill tool (should be routed elsewhere).
pub async fn handle_skill_tool_call(
    tool_name: &str,
    arguments: &serde_json::Value,
    skill_manager: &SkillManager,
    script_executor: &ScriptExecutor,
    access: &SkillsAccess,
    info_loaded: &HashSet<String>,
    async_enabled: bool,
) -> Result<Option<SkillToolResult>, String> {
    let parsed = match parse_skill_tool_name(tool_name, skill_manager, access) {
        Some(p) => p,
        None => return Ok(None),
    };

    match parsed {
        SkillToolParsed::GetInfo { skill_name } => {
            let skill = skill_manager
                .get(&skill_name)
                .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

            let response = build_get_info_response(&skill, async_enabled);
            Ok(Some(SkillToolResult::InfoLoaded {
                skill_name,
                response,
            }))
        }

        SkillToolParsed::Run {
            skill_name,
            script_file,
        } => {
            if !info_loaded.contains(&skill_name) {
                let sname = sanitize_name(&skill_name);
                return Err(format!(
                    "Call skill_{}_get_info first to unlock run/read tools for this skill.",
                    sname
                ));
            }

            let command = arguments.get("command").and_then(|v| v.as_str());
            let timeout = arguments.get("timeout").and_then(|v| v.as_u64());
            let tail = arguments
                .get("tail")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let skill_dir = skill_manager
                .get_skill_dir(&skill_name)
                .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

            let result = script_executor
                .run_sync(&skill_dir, &script_file, command, timeout, tail)
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

            Ok(Some(SkillToolResult::Response(json!({
                "content": [{
                    "type": "text",
                    "text": text.trim()
                }],
                "exit_code": result.exit_code,
                "timed_out": result.timed_out
            }))))
        }

        SkillToolParsed::RunAsync {
            skill_name,
            script_file,
        } => {
            if !async_enabled {
                return Err("Async script execution is not enabled.".to_string());
            }
            if !info_loaded.contains(&skill_name) {
                let sname = sanitize_name(&skill_name);
                return Err(format!(
                    "Call skill_{}_get_info first to unlock run/read tools for this skill.",
                    sname
                ));
            }

            let command = arguments.get("command").and_then(|v| v.as_str());
            let timeout = arguments.get("timeout").and_then(|v| v.as_u64());

            let skill_dir = skill_manager
                .get_skill_dir(&skill_name)
                .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

            let pid = script_executor
                .run_async(&skill_dir, &script_file, command, timeout)
                .await?;

            Ok(Some(SkillToolResult::Response(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Script started asynchronously. PID: {}\nUse skill_get_async_status with this PID to check status.", pid)
                }],
                "pid": pid
            }))))
        }

        SkillToolParsed::Read {
            skill_name,
            resource_file,
        } => {
            if !info_loaded.contains(&skill_name) {
                let sname = sanitize_name(&skill_name);
                return Err(format!(
                    "Call skill_{}_get_info first to unlock run/read tools for this skill.",
                    sname
                ));
            }

            let content = skill_manager.get_resource(&skill_name, &resource_file)?;

            Ok(Some(SkillToolResult::Response(json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            }))))
        }

        SkillToolParsed::GetAsyncStatus => {
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

            Ok(Some(SkillToolResult::Response(json!({
                "content": [{
                    "type": "text",
                    "text": text.trim()
                }],
                "running": status.running,
                "exit_code": status.exit_code,
                "timed_out": status.timed_out
            }))))
        }
    }
}

// ---------------------------------------------------------------------------
// get_info response builder
// ---------------------------------------------------------------------------

/// Build the response for a get_info tool call.
fn build_get_info_response(skill: &SkillDefinition, async_enabled: bool) -> serde_json::Value {
    let sname = sanitize_name(&skill.metadata.name);
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

    // File listings with new tool names
    if !skill.scripts.is_empty() {
        text.push_str("## Scripts\n\n");
        for script in &skill.scripts {
            let sfile = sanitize_tool_segment(script);
            text.push_str(&format!(
                "- `{}` — run with `skill_{}_run_{}`\n",
                script, sname, sfile
            ));
            if async_enabled {
                text.push_str(&format!(
                    "  - async: `skill_{}_run_async_{}`\n",
                    sname, sfile
                ));
            }
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        for reference in &skill.references {
            let sfile = sanitize_tool_segment(reference);
            text.push_str(&format!(
                "- `{}` — read with `skill_{}_read_{}`\n",
                reference, sname, sfile
            ));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        for asset in &skill.assets {
            let sfile = sanitize_tool_segment(asset);
            text.push_str(&format!(
                "- `{}` — read with `skill_{}_read_{}`\n",
                asset, sname, sfile
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
