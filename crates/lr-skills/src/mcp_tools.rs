//! MCP tool definitions for skills
//!
//! Generates per-skill `get_info` McpTool definitions. Clients use
//! `ctx_execute_file` with absolute paths to run scripts.

use super::manager::SkillManager;
use super::types::{sanitize_name, SkillDefinition};
use lr_config::SkillsPermissions;
use lr_types::McpTool;
use serde_json::json;

/// Result of handling a skill tool call.
pub enum SkillToolResult {
    /// get_info response
    Response(serde_json::Value),
}

/// Parsed skill tool name
enum SkillToolParsed {
    GetInfo { skill_name: String },
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
            "Show full instructions for skill '{}'. {}",
            skill.metadata.name, description
        )),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool list builder
// ---------------------------------------------------------------------------

/// Generate skill MCP tools for a client's allowed skills.
///
/// Only includes `get_info` tools. Clients use `ctx_execute_file` with
/// absolute paths (shown in get_info response) to run scripts.
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Vec<McpTool> {
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();

    all_skills
        .iter()
        .filter(|s| s.enabled && permissions.resolve_skill(&s.metadata.name).is_enabled())
        .map(|s| build_get_info_tool(s))
        .collect()
}

// ---------------------------------------------------------------------------
// Tool name parsing
// ---------------------------------------------------------------------------

/// Parse a tool name into its skill tool variant.
fn parse_skill_tool_name(
    tool_name: &str,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Option<SkillToolParsed> {
    if !tool_name.starts_with("skill_") {
        return None;
    }

    let all_skills = skill_manager.get_all();

    for skill in all_skills.iter() {
        if !skill.enabled || !permissions.resolve_skill(&skill.metadata.name).is_enabled() {
            continue;
        }

        let sname = sanitize_name(&skill.metadata.name);
        let prefix = format!("skill_{}_", sname);

        if let Some(rest) = tool_name.strip_prefix(&prefix) {
            if rest == "get_info" {
                return Some(SkillToolParsed::GetInfo {
                    skill_name: skill.metadata.name.clone(),
                });
            }
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
    _arguments: &serde_json::Value,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Result<Option<SkillToolResult>, String> {
    let parsed = match parse_skill_tool_name(tool_name, skill_manager, permissions) {
        Some(p) => p,
        None => return Ok(None),
    };

    match parsed {
        SkillToolParsed::GetInfo { skill_name } => {
            let skill = skill_manager
                .get(&skill_name)
                .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

            let response = build_get_info_response(&skill);
            Ok(Some(SkillToolResult::Response(response)))
        }
    }
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
            text.push_str(&format!(
                "- `{}/{}`\n",
                skill_dir, script
            ));
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        text.push_str("Read files with `ctx_execute_file(path, language, code)` using `cat`.\n\n");
        for reference in &skill.references {
            text.push_str(&format!(
                "- `{}/{}`\n",
                skill_dir, reference
            ));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        for asset in &skill.assets {
            text.push_str(&format!(
                "- `{}/{}`\n",
                skill_dir, asset
            ));
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
