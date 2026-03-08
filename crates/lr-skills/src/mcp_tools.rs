//! MCP tool definitions for skills
//!
//! Exposes a single `skill_get_info` meta-tool that takes a skill name
//! parameter. This follows the same progressive-disclosure pattern as
//! Claude Code's built-in `Skill` tool, but uses the `skill_` prefix to
//! avoid naming collisions (Claude Code's tool is called `Skill`;
//! ours becomes `mcp__<server>__skill_get_info` after MCP namespacing).
//!
//! Clients use `ctx_execute_file` with absolute paths to run scripts.

use super::manager::SkillManager;
use super::types::SkillDefinition;
use lr_config::SkillsPermissions;
use lr_types::McpTool;
use serde_json::json;

/// The single meta-tool name for skill info retrieval.
pub const SKILL_META_TOOL_NAME: &str = "skill_get_info";

/// Result of handling a skill tool call.
pub enum SkillToolResult {
    /// get_info response
    Response(serde_json::Value),
}

// ---------------------------------------------------------------------------
// Tool builder
// ---------------------------------------------------------------------------

/// Build the single `skill_get_info` meta-tool with an enum of available skills.
///
/// The tool accepts a `skill` parameter whose allowed values are the
/// sanitized names of all enabled & permitted skills for this client.
fn build_meta_tool(accessible_skills: &[&SkillDefinition]) -> McpTool {
    let enum_values: Vec<serde_json::Value> = accessible_skills
        .iter()
        .map(|s| json!(s.metadata.name))
        .collect();

    let skill_descriptions: Vec<String> = accessible_skills
        .iter()
        .map(|s| {
            let desc = s
                .metadata
                .description
                .as_deref()
                .unwrap_or("No description");
            format!("- `{}`: {}", s.metadata.name, desc)
        })
        .collect();

    McpTool {
        name: SKILL_META_TOOL_NAME.to_string(),
        description: Some(format!(
            "Load full instructions for a skill. Available skills:\n{}",
            skill_descriptions.join("\n")
        )),
        input_schema: json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Name of the skill to load",
                    "enum": enum_values
                }
            },
            "required": ["skill"],
            "additionalProperties": false
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool list builder
// ---------------------------------------------------------------------------

/// Generate skill MCP tools for a client's allowed skills.
///
/// Returns a single `skill_get_info` meta-tool if there are accessible skills.
/// Clients use `ctx_execute_file` with absolute paths (shown in the response)
/// to run scripts.
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Vec<McpTool> {
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.resolve_skill(&s.metadata.name).is_enabled())
        .collect();

    if accessible.is_empty() {
        return Vec::new();
    }

    vec![build_meta_tool(&accessible)]
}

// ---------------------------------------------------------------------------
// Tool call handler
// ---------------------------------------------------------------------------

/// Check if a tool name matches the skill meta-tool.
pub fn is_skill_tool(tool_name: &str) -> bool {
    tool_name == SKILL_META_TOOL_NAME
}

/// Handle a skill tool call.
///
/// Returns `Ok(Some(result))` if the tool was the skill meta-tool,
/// `Ok(None)` if it's not a skill tool (should be routed elsewhere).
pub async fn handle_skill_tool_call(
    tool_name: &str,
    arguments: &serde_json::Value,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Result<Option<SkillToolResult>, String> {
    if tool_name != SKILL_META_TOOL_NAME {
        return Ok(None);
    }

    let skill_name = arguments
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required 'skill' parameter".to_string())?;

    // Verify the skill is accessible
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return Err("No skill access".to_string());
    }
    if !permissions.resolve_skill(skill_name).is_enabled() {
        return Err(format!("Skill '{}' is not permitted", skill_name));
    }

    let skill = skill_manager
        .get(skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    if !skill.enabled {
        return Err(format!("Skill '{}' is disabled", skill_name));
    }

    let response = build_get_info_response(&skill);
    Ok(Some(SkillToolResult::Response(response)))
}

// ---------------------------------------------------------------------------
// get_info response builder
// ---------------------------------------------------------------------------

/// Build the response for a get_info tool call.
///
/// Shows absolute paths so clients can use `ctx_execute_file` to run scripts.
fn build_get_info_response(skill: &SkillDefinition) -> serde_json::Value {
    let mut text = String::new();
    let skill_dir = skill.skill_dir.display();

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

    text.push_str(&format!("**Location:** `{}/SKILL.md`\n", skill_dir));
    text.push('\n');

    // File listings with absolute paths
    if !skill.scripts.is_empty() {
        text.push_str("## Scripts\n\n");
        text.push_str("Run scripts with `ctx_execute_file(path, language, code)`.\n\n");
        for script in &skill.scripts {
            text.push_str(&format!("- `{}/{}`\n", skill_dir, script));
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        text.push_str("Read files with `ctx_execute_file(path, language, code)` using `cat`.\n\n");
        for reference in &skill.references {
            text.push_str(&format!("- `{}/{}`\n", skill_dir, reference));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        for asset in &skill.assets {
            text.push_str(&format!("- `{}/{}`\n", skill_dir, asset));
        }
        text.push('\n');
    }

    // Full SKILL.md body
    text.push_str("## Instructions\n\n");
    text.push_str(&format!(
        "> File paths in these instructions are relative to: `{}`\n\n",
        skill_dir
    ));
    text.push_str(&skill.body);

    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}
