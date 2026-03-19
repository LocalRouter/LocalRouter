//! MCP tool definitions for skills
//!
//! Exposes a single `skill_read` meta-tool that takes a skill name
//! parameter. This follows a progressive-disclosure pattern: the skill
//! catalog (names + descriptions) is listed in the welcome message, and
//! the LLM calls `skill_read(name)` to load full instructions.
//!
//! Skill files (scripts, references, assets) are readable via
//! `resource_read(name="<skill>/<path>")`.

use super::manager::SkillManager;
use super::types::SkillDefinition;
use lr_config::SkillsPermissions;
use lr_types::McpTool;
use serde_json::json;

/// Default meta-tool name for skill reading.
pub const SKILL_META_TOOL_NAME: &str = "SkillRead";

/// Default internal tool name for reading skill files (not exposed to LLM).
/// Used by the orchestrator's `resource_read` to route skill file reads
/// through the gateway.
pub const SKILL_READ_FILE_TOOL_NAME: &str = "SkillReadFile";

/// Result of handling a skill tool call.
pub enum SkillToolResult {
    /// skill_read response
    Response(serde_json::Value),
}

// ---------------------------------------------------------------------------
// Tool builder
// ---------------------------------------------------------------------------

/// Build the single skill-read meta-tool.
///
/// The tool accepts a `name` parameter (any string). The catalog of available
/// skills is listed in the welcome message, NOT embedded in the tool definition.
fn build_meta_tool(tool_name: &str, resource_read_name: &str) -> McpTool {
    McpTool {
        name: tool_name.to_string(),
        description: Some(format!(
            "Read a skill's full instructions, metadata, and file listing. \
             Skill names are listed in the welcome message. \
             If skills are hidden due to compression, use ctx_search to discover them. \
             Skill files (scripts, references, assets) can be read with {}.",
            resource_read_name,
        )),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to read"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool list builder
// ---------------------------------------------------------------------------

/// Generate skill MCP tools for a client's allowed skills.
///
/// Returns a single skill-read meta-tool if there are accessible skills.
/// The skill catalog is NOT embedded in the tool — it's in the welcome message.
///
/// `tool_name` is the configured name for the meta-tool (e.g. "SkillRead").
/// `resource_read_name` is the name referenced in descriptions (e.g. "ResourceRead").
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    tool_name: &str,
    resource_read_name: &str,
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

    vec![build_meta_tool(tool_name, resource_read_name)]
}

/// Build the skill catalog text for inclusion in the welcome message.
///
/// Returns a formatted listing of available skills with names, descriptions,
/// and file counts. When `compress` is true and there are many skills,
/// the listing is truncated with a ctx_search hint.
///
/// `tool_name` is the configured skill-read tool name (e.g. "SkillRead").
/// `resource_read_name` is the name referenced in descriptions (e.g. "ResourceRead").
pub fn build_skill_catalog(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    context_management_enabled: bool,
    tool_name: &str,
    resource_read_name: &str,
) -> Option<String> {
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return None;
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.resolve_skill(&s.metadata.name).is_enabled())
        .collect();

    if accessible.is_empty() {
        return None;
    }

    let mut text = String::from("Available skills:\n");

    // Compression thresholds
    const FULL_THRESHOLD: usize = 20;
    const NAMES_ONLY_THRESHOLD: usize = 50;

    if context_management_enabled && accessible.len() > NAMES_ONLY_THRESHOLD {
        // Phase 3: Show top 10 names + count hint
        for skill in accessible.iter().take(10) {
            text.push_str(&format!("- `{}`\n", skill.metadata.name));
        }
        text.push_str(&format!(
            "... and {} more — use ctx_search(source=\"catalog:skills\") to discover all skills\n",
            accessible.len() - 10
        ));
    } else if context_management_enabled && accessible.len() > FULL_THRESHOLD {
        // Phase 2: Names only + ctx_search hint
        for skill in &accessible {
            text.push_str(&format!("- `{}`\n", skill.metadata.name));
        }
        text.push_str(
            "Use ctx_search(source=\"catalog:skills\") for skill descriptions and details.\n",
        );
    } else {
        // Phase 1: Full listing with name + description + file counts
        for skill in &accessible {
            let desc = skill
                .metadata
                .description
                .as_deref()
                .unwrap_or("No description");
            let file_count = skill.scripts.len() + skill.references.len() + skill.assets.len();
            if file_count > 0 {
                text.push_str(&format!(
                    "- `{}`: {} ({} files)\n",
                    skill.metadata.name, desc, file_count
                ));
            } else {
                text.push_str(&format!("- `{}`: {}\n", skill.metadata.name, desc));
            }
        }
    }

    text.push_str(&format!(
        "Call {}(name) to load full instructions.\n",
        tool_name
    ));
    text.push_str(&format!(
        "Read skill files with {}(name=\"<skill>/<path>\").\n",
        resource_read_name
    ));

    Some(text)
}

// ---------------------------------------------------------------------------
// Tool call handler
// ---------------------------------------------------------------------------

/// Check if a tool name matches a skill tool (meta-tool or read-file tool).
pub fn is_skill_tool(
    tool_name: &str,
    configured_tool_name: &str,
    configured_rfile_name: &str,
) -> bool {
    tool_name == configured_tool_name || tool_name == configured_rfile_name
}

/// Handle a skill tool call.
///
/// Returns `Ok(Some(result))` if the tool was the skill meta-tool,
/// `Ok(None)` if it's not a skill tool (should be routed elsewhere).
///
/// `configured_tool_name` is the configured name for the meta-tool (e.g. "SkillRead").
pub async fn handle_skill_tool_call(
    tool_name: &str,
    arguments: &serde_json::Value,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    configured_tool_name: &str,
) -> Result<Option<SkillToolResult>, String> {
    if tool_name != configured_tool_name {
        return Ok(None);
    }

    let skill_name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required 'name' parameter".to_string())?;

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

    let response = build_skill_read_response(&skill);
    Ok(Some(SkillToolResult::Response(response)))
}

/// Read a skill file (script, reference, or asset) by relative path.
///
/// The `subpath` is relative to the skill directory, e.g. `scripts/build.sh`.
/// Returns the file content as text, or an error if the file doesn't exist
/// or is not part of the skill's known files.
///
/// `configured_tool_name` is the configured skill-read meta-tool name, used in error messages.
/// `configured_rfile_name` is the configured read-file tool name, used in error messages.
pub fn read_skill_file(
    skill_name: &str,
    subpath: &str,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    configured_tool_name: &str,
    configured_rfile_name: &str,
) -> Result<String, String> {
    // Verify access
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

    // Block access to SKILL.md — that's only returned by the skill-read meta-tool
    if subpath == "SKILL.md" || subpath == "skill.md" {
        return Err(format!(
            "SKILL.md is not available via {}. Use {} instead.",
            configured_rfile_name, configured_tool_name,
        ));
    }

    // Verify the file is part of the skill's known files
    let all_files: Vec<&str> = skill
        .scripts
        .iter()
        .chain(skill.references.iter())
        .chain(skill.assets.iter())
        .map(|s| s.as_str())
        .collect();

    if !all_files.contains(&subpath) {
        return Err(format!(
            "File '{}' is not part of skill '{}'. Available files: {}",
            subpath,
            skill_name,
            all_files.join(", ")
        ));
    }

    // Read from disk
    let file_path = skill.skill_dir.join(subpath);
    std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read '{}': {}", file_path.display(), e))
}

// ---------------------------------------------------------------------------
// skill_read response builder
// ---------------------------------------------------------------------------

/// Build the response for a skill_read tool call.
///
/// Shows skill file paths as resource_read-compatible names
/// (e.g. `<skill>/scripts/build.sh`) instead of absolute disk paths.
fn build_skill_read_response(skill: &SkillDefinition) -> serde_json::Value {
    let mut text = String::new();
    let skill_name = &skill.metadata.name;

    // Header
    text.push_str(&format!("# Skill: {}\n\n", skill_name));

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

    // File listings with resource_read-compatible paths
    if !skill.scripts.is_empty() {
        text.push_str("## Scripts\n\n");
        text.push_str("Read with `resource_read(name=\"...\")`.\n\n");
        for script in &skill.scripts {
            text.push_str(&format!("- `{}/{}`\n", skill_name, script));
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        text.push_str("Read with `resource_read(name=\"...\")`.\n\n");
        for reference in &skill.references {
            text.push_str(&format!("- `{}/{}`\n", skill_name, reference));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        text.push_str("Read with `resource_read(name=\"...\")`.\n\n");
        for asset in &skill.assets {
            text.push_str(&format!("- `{}/{}`\n", skill_name, asset));
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
