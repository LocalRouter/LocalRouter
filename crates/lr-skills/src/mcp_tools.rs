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

/// Legacy internal tool name for reading skill files.
/// Kept for backwards compatibility during config migration.
/// New code should use SkillRead with the `path` parameter instead.
#[deprecated(note = "Use SkillRead with path parameter instead")]
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
/// The tool accepts a `name` parameter. Available skill names are listed
/// in the parameter description for direct discoverability.
fn build_meta_tool(tool_name: &str, skill_names: &[&str]) -> McpTool {
    let name_desc = if skill_names.is_empty() {
        "Skill name".to_string()
    } else {
        format!("Skill name. Available: {}", skill_names.join(", "))
    };

    McpTool {
        name: tool_name.to_string(),
        description: Some(
            "Read a skill's full instructions, metadata, and file listing. \
             Pass 'path' to read a specific skill file instead."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": name_desc
                },
                "path": {
                    "type": "string",
                    "description": "Optional: relative file path within the skill (e.g. 'scripts/run.sh'). Omit to get full instructions."
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
/// Available skill names are listed in the `name` parameter description.
///
/// `tool_name` is the configured name for the meta-tool (e.g. "SkillRead").
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    tool_name: &str,
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

    let skill_names: Vec<&str> = accessible
        .iter()
        .map(|s| s.metadata.name.as_str())
        .collect();

    vec![build_meta_tool(tool_name, &skill_names)]
}

/// Build the skill catalog text for inclusion in the welcome message.
///
/// Returns a formatted listing of available skills with names, descriptions,
/// and file counts. When `compress` is true and there are many skills,
/// the listing is truncated with a search hint.
///
/// `tool_name` is the configured skill-read tool name (e.g. "SkillRead").
/// `search_tool_name` is the configured search tool name (e.g. "IndexSearch").
pub fn build_skill_catalog(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    context_management_enabled: bool,
    tool_name: &str,
    search_tool_name: &str,
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
            "... and {} more — use {}(source=\"catalog:skills\") to discover all skills\n",
            accessible.len() - 10,
            search_tool_name,
        ));
    } else if context_management_enabled && accessible.len() > FULL_THRESHOLD {
        // Phase 2: Names only + search hint
        for skill in &accessible {
            text.push_str(&format!("- `{}`\n", skill.metadata.name));
        }
        text.push_str(&format!(
            "Use {}(source=\"catalog:skills\") for skill descriptions and details.\n",
            search_tool_name,
        ));
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
        "Read skill files with {}(name=\"<skill>\", path=\"<relative-path>\").\n",
        tool_name
    ));

    Some(text)
}

// ---------------------------------------------------------------------------
// Catalog indexing
// ---------------------------------------------------------------------------

/// Build index entries for skills (name + description + tags + file listing).
///
/// Returns `Vec<(label, content)>` where label is `"catalog:skills/{name}"`.
/// Used by the gateway to index skills into the FTS5 ContentStore so they
/// are discoverable via `IndexSearch(source="catalog:skills")`.
pub fn build_skill_index_entries(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Vec<(String, String)> {
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.resolve_skill(&s.metadata.name).is_enabled())
        .collect();

    let mut entries = Vec::with_capacity(accessible.len());
    for skill in &accessible {
        let label = format!("catalog:skills/{}", skill.metadata.name);
        let mut content = format!("# {}\n", skill.metadata.name);

        if let Some(desc) = &skill.metadata.description {
            content.push_str(&format!("{}\n", desc));
        }

        if !skill.metadata.tags.is_empty() {
            content.push_str(&format!("Tags: {}\n", skill.metadata.tags.join(", ")));
        }

        let file_count = skill.scripts.len() + skill.references.len() + skill.assets.len();
        if file_count > 0 {
            content.push_str(&format!("Files: {}\n", file_count));
            for script in &skill.scripts {
                content.push_str(&format!("- {}\n", script));
            }
            for reference in &skill.references {
                content.push_str(&format!("- {}\n", reference));
            }
            for asset in &skill.assets {
                content.push_str(&format!("- {}\n", asset));
            }
        }

        entries.push((label, content));
    }

    entries
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
/// When an exact name match fails, attempts fuzzy matching (case-insensitive,
/// normalized, Levenshtein) and returns the matched skill with a correction note.
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

    // Check if a specific file path was requested
    let path = arguments.get("path").and_then(|v| v.as_str());

    // Verify the client has any skill access at all
    let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
    if !has_any_access {
        return Err("No skill access".to_string());
    }

    // If path is provided, delegate to read_skill_file
    if let Some(subpath) = path {
        let content = read_skill_file(
            skill_name,
            subpath,
            skill_manager,
            permissions,
            configured_tool_name,
            configured_tool_name, // same tool for both now
        )?;
        let response = json!({
            "content": [{ "type": "text", "text": content }]
        });
        return Ok(Some(SkillToolResult::Response(response)));
    }

    // Try exact match first (fast path)
    if let Some(skill) = skill_manager.get(skill_name) {
        if !permissions.resolve_skill(skill_name).is_enabled() {
            return Err(format!("Skill '{}' is not permitted", skill_name));
        }
        if !skill.enabled {
            return Err(format!("Skill '{}' is disabled", skill_name));
        }
        let response = build_skill_read_response(&skill, configured_tool_name);
        return Ok(Some(SkillToolResult::Response(response)));
    }

    // Exact match failed — try fuzzy matching
    match skill_manager.find_closest(skill_name) {
        Some((skill, match_kind)) => {
            let resolved_name = &skill.metadata.name;

            // Check permissions on the resolved name
            if !permissions.resolve_skill(resolved_name).is_enabled() {
                return Err(not_found_error(skill_name, skill_manager, permissions));
            }

            let mut response = build_skill_read_response(&skill, configured_tool_name);
            prepend_correction_note(&mut response, skill_name, resolved_name, &match_kind);
            Ok(Some(SkillToolResult::Response(response)))
        }
        None => Err(not_found_error(skill_name, skill_manager, permissions)),
    }
}

/// Read a skill file (script, reference, or asset) by relative path.
///
/// The `subpath` is relative to the skill directory, e.g. `scripts/build.sh`.
/// Returns the file content as text, or an error if the file doesn't exist
/// or is not part of the skill's known files.
///
/// When an exact skill name match fails, attempts fuzzy matching and prepends
/// a correction note to the returned content.
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

    // Resolve skill: exact match first, then fuzzy fallback
    let (skill, correction_note) = if let Some(skill) = skill_manager.get(skill_name) {
        if !permissions.resolve_skill(skill_name).is_enabled() {
            return Err(format!("Skill '{}' is not permitted", skill_name));
        }
        if !skill.enabled {
            return Err(format!("Skill '{}' is disabled", skill_name));
        }
        (skill, None)
    } else {
        // Fuzzy fallback
        match skill_manager.find_closest(skill_name) {
            Some((skill, match_kind)) if !matches!(match_kind, crate::fuzzy::MatchKind::Exact) => {
                let resolved = &skill.metadata.name;
                if !permissions.resolve_skill(resolved).is_enabled() {
                    return Err(not_found_error(skill_name, skill_manager, permissions));
                }
                let note = format!(
                    "Note: No skill named '{}' was found. Reading from skill '{}' instead.\n\n",
                    skill_name, resolved
                );
                (skill, Some(note))
            }
            _ => return Err(not_found_error(skill_name, skill_manager, permissions)),
        }
    };

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
        return if all_files.is_empty() {
            Err(format!(
                "File '{}' is not part of skill '{}'. This skill has no readable files (no scripts/, references/, or assets/ directory).",
                subpath,
                skill.metadata.name,
            ))
        } else {
            Err(format!(
                "File '{}' is not part of skill '{}'. Available files: {}",
                subpath,
                skill.metadata.name,
                all_files.join(", ")
            ))
        };
    }

    // Read from disk
    let file_path = skill.skill_dir.join(subpath);
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read '{}': {}", file_path.display(), e))?;

    match correction_note {
        Some(note) => Ok(format!("{}{}", note, content)),
        None => Ok(content),
    }
}

// ---------------------------------------------------------------------------
// Fuzzy matching helpers
// ---------------------------------------------------------------------------

/// Prepend a correction note to the JSON response when a fuzzy match was used.
fn prepend_correction_note(
    response: &mut serde_json::Value,
    requested: &str,
    resolved: &str,
    match_kind: &crate::fuzzy::MatchKind,
) {
    if matches!(match_kind, crate::fuzzy::MatchKind::Exact) {
        return;
    }

    let note = format!(
        "Note: No skill named '{}' was found. Showing skill '{}' instead.\n\n",
        requested, resolved
    );

    if let Some(content) = response.get_mut("content") {
        if let Some(arr) = content.as_array_mut() {
            if let Some(first) = arr.first_mut() {
                if let Some(text) = first.get_mut("text") {
                    if let Some(s) = text.as_str() {
                        *text = serde_json::Value::String(format!("{}{}", note, s));
                    }
                }
            }
        }
    }
}

/// Build an error message listing available skill names.
fn not_found_error(
    skill_name: &str,
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> String {
    let all_skills = skill_manager.get_all();
    let accessible_names: Vec<&str> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.resolve_skill(&s.metadata.name).is_enabled())
        .map(|s| s.metadata.name.as_str())
        .collect();

    if accessible_names.is_empty() {
        format!("Skill '{}' not found. No skills are available.", skill_name)
    } else {
        format!(
            "Skill '{}' not found. Available skills: {}",
            skill_name,
            accessible_names.join(", ")
        )
    }
}

// ---------------------------------------------------------------------------
// skill_read response builder
// ---------------------------------------------------------------------------

/// Build the response for a skill_read tool call.
///
/// Shows skill file paths with SkillRead(name, path) syntax.
fn build_skill_read_response(
    skill: &SkillDefinition,
    tool_name: &str,
) -> serde_json::Value {
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

    // File listings with SkillRead(name, path) syntax
    if !skill.scripts.is_empty() {
        text.push_str("## Scripts\n\n");
        text.push_str(&format!(
            "Read with `{}(name=\"{}\", path=\"...\")`.\n\n",
            tool_name, skill_name
        ));
        for script in &skill.scripts {
            text.push_str(&format!("- `{}`\n", script));
        }
        text.push('\n');
    }

    if !skill.references.is_empty() {
        text.push_str("## References\n\n");
        text.push_str(&format!(
            "Read with `{}(name=\"{}\", path=\"...\")`.\n\n",
            tool_name, skill_name
        ));
        for reference in &skill.references {
            text.push_str(&format!("- `{}`\n", reference));
        }
        text.push('\n');
    }

    if !skill.assets.is_empty() {
        text.push_str("## Assets\n\n");
        text.push_str(&format!(
            "Read with `{}(name=\"{}\", path=\"...\")`.\n\n",
            tool_name, skill_name
        ));
        for asset in &skill.assets {
            text.push_str(&format!("- `{}`\n", asset));
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
