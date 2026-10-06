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
fn build_meta_tool(tool_name: &str, skill_names: &[&str], style: SkillPathStyle) -> McpTool {
    let name_desc = if skill_names.is_empty() {
        "Skill name".to_string()
    } else {
        format!("Skill name. Available: {}", skill_names.join(", "))
    };

    McpTool {
        name: tool_name.to_string(),
        description: Some(match style {
            SkillPathStyle::Disk => "Read a skill's full instructions, metadata, and files. \
                 The response gives the skill's directory and the absolute path of each \
                 bundled script, so scripts can be run in place with your shell. \
                 Pass 'path' to read a specific skill file instead."
                .to_string(),
            SkillPathStyle::Virtual => {
                "Read a skill's full instructions, metadata, and file listing. \
                 Pass 'path' to read a specific skill file instead."
                    .to_string()
            }
        }),
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
    style: SkillPathStyle,
) -> Vec<McpTool> {
    let has_any_access = permissions.has_any_access();
    if !has_any_access {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.has_any_enabled_for_skill(&s.metadata.name))
        .collect();

    if accessible.is_empty() {
        return Vec::new();
    }

    let skill_names: Vec<&str> = accessible
        .iter()
        .map(|s| s.metadata.name.as_str())
        .collect();

    vec![build_meta_tool(tool_name, &skill_names, style)]
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
    style: SkillPathStyle,
) -> Option<String> {
    let has_any_access = permissions.has_any_access();
    if !has_any_access {
        return None;
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.has_any_enabled_for_skill(&s.metadata.name))
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
    if style == SkillPathStyle::Disk {
        text.push_str(&format!(
            "{}(name) also gives each skill's directory and the absolute paths of its scripts; \
             run scripts from there with your shell.\n",
            tool_name
        ));
    }

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
    style: SkillPathStyle,
) -> Vec<(String, String)> {
    let has_any_access = permissions.has_any_access();
    if !has_any_access {
        return Vec::new();
    }

    let all_skills = skill_manager.get_all();
    let accessible: Vec<&SkillDefinition> = all_skills
        .iter()
        .filter(|s| s.enabled && permissions.has_any_enabled_for_skill(&s.metadata.name))
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

        let dir = skill_dir_path(skill);
        if style == SkillPathStyle::Disk {
            content.push_str(&format!("Location: {}/\n", dir.display()));
        }

        let file_count = skill.scripts.len() + skill.references.len() + skill.assets.len();
        if file_count > 0 {
            content.push_str(&format!("Files: {}\n", file_count));
            for file in skill
                .scripts
                .iter()
                .chain(&skill.references)
                .chain(&skill.assets)
            {
                match style {
                    SkillPathStyle::Disk => {
                        content.push_str(&format!("- {} ({})\n", file, dir.join(file).display()))
                    }
                    SkillPathStyle::Virtual => content.push_str(&format!("- {}\n", file)),
                }
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
    style: SkillPathStyle,
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
    let has_any_access = permissions.has_any_access();
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
            style,
        )?;
        let response = json!({
            "content": [{ "type": "text", "text": content }]
        });
        return Ok(Some(SkillToolResult::Response(response)));
    }

    // Try exact match first (fast path)
    if let Some(skill) = skill_manager.get(skill_name) {
        if !permissions.has_any_enabled_for_skill(skill_name) {
            return Err(format!("Skill '{}' is not permitted", skill_name));
        }
        if !skill.enabled {
            return Err(format!("Skill '{}' is disabled", skill_name));
        }
        let response = build_skill_read_response(&skill, configured_tool_name, style);
        return Ok(Some(SkillToolResult::Response(response)));
    }

    // Exact match failed — try fuzzy matching
    match skill_manager.find_closest(skill_name) {
        Some((skill, match_kind)) => {
            let resolved_name = &skill.metadata.name;

            // Check permissions on the resolved name
            if !permissions.has_any_enabled_for_skill(resolved_name) {
                return Err(not_found_error(skill_name, skill_manager, permissions));
            }

            let mut response = build_skill_read_response(&skill, configured_tool_name, style);
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
    style: SkillPathStyle,
) -> Result<String, String> {
    // Verify access
    let has_any_access = permissions.has_any_access();
    if !has_any_access {
        return Err("No skill access".to_string());
    }

    // Resolve skill: exact match first, then fuzzy fallback
    let (skill, correction_note) = if let Some(skill) = skill_manager.get(skill_name) {
        if !permissions.has_any_enabled_for_skill(skill_name) {
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
                if !permissions.has_any_enabled_for_skill(resolved) {
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

    // Treat ".", "", "/" as directory-listing requests — return the file listing
    if matches!(subpath, "." | "" | "/") {
        let all_files: Vec<&str> = skill
            .scripts
            .iter()
            .chain(skill.references.iter())
            .chain(skill.assets.iter())
            .map(|s| s.as_str())
            .collect();
        if all_files.is_empty() {
            return Ok("This skill has no readable files.".to_string());
        }
        let mut listing = match style {
            SkillPathStyle::Disk => format!(
                "Files in skill '{}' (located at {}/):\n",
                skill.metadata.name,
                skill_dir_path(&skill).display()
            ),
            SkillPathStyle::Virtual => format!("Files in skill '{}':\n", skill.metadata.name),
        };
        for f in &all_files {
            match style {
                SkillPathStyle::Disk => listing.push_str(&format!(
                    "- {} ({})\n",
                    f,
                    skill_dir_path(&skill).join(f).display()
                )),
                SkillPathStyle::Virtual => listing.push_str(&format!("- {}\n", f)),
            }
        }
        let mut prefix = String::new();
        if let Some(note) = correction_note {
            prefix.push_str(&note);
        }
        return if prefix.is_empty() {
            Ok(listing)
        } else {
            Ok(format!("{}{}", prefix, listing))
        };
    }

    // Block access to SKILL.md — that's only returned by the skill-read meta-tool
    if subpath == "SKILL.md" || subpath == "skill.md" {
        return Err(format!(
            "SKILL.md is not available via {}. Use {} instead.",
            configured_rfile_name, configured_tool_name,
        ));
    }

    // Resolve the file path against the skill's known files.
    // Tries: exact match → strip prefix → add prefix → fuzzy match.
    let all_files: Vec<&str> = skill
        .scripts
        .iter()
        .chain(skill.references.iter())
        .chain(skill.assets.iter())
        .map(|s| s.as_str())
        .collect();

    let (resolved_path, path_correction) = resolve_skill_file_path(subpath, &all_files)?;

    // Discovery follows file symlinks, so membership in the known file list
    // alone does not prove that the resolved file stays inside the skill.
    let content = skill_manager.get_resource(&skill.metadata.name, resolved_path)?;

    // Combine skill-name and path correction notes
    let mut prefix = String::new();
    if let Some(note) = correction_note {
        prefix.push_str(&note);
    }
    if let Some(note) = path_correction {
        prefix.push_str(&note);
    }
    if prefix.is_empty() {
        Ok(content)
    } else {
        Ok(format!("{}{}", prefix, content))
    }
}

// ---------------------------------------------------------------------------
// File path resolution
// ---------------------------------------------------------------------------

/// Resolve a requested file path against a skill's known files.
///
/// Tries layers in order:
/// 1. Exact match
/// 2. Strip directory prefix and try bare filename (e.g., `scripts/run.sh` → `run.sh`)
/// 3. Add known prefixes (`scripts/`, `references/`, `assets/`)
/// 4. Fuzzy match using Levenshtein distance
///
/// Returns `(resolved_path, Option<correction_note>)` on success.
fn resolve_skill_file_path<'a>(
    requested: &str,
    all_files: &[&'a str],
) -> Result<(&'a str, Option<String>), String> {
    if all_files.is_empty() {
        return Err(format!(
            "File '{}' not found. This skill has no readable files.",
            requested,
        ));
    }

    // Layer 1: Exact match
    if let Some(&path) = all_files.iter().find(|&&f| f == requested) {
        return Ok((path, None));
    }

    // Layer 2: Strip prefix — e.g., `scripts/sysinfo.sh` → try `sysinfo.sh`
    if let Some(basename) = requested.rsplit('/').next() {
        if basename != requested {
            if let Some(&path) = all_files.iter().find(|&&f| f == basename) {
                return Ok((
                    path,
                    Some(format!(
                        "Note: '{}' was not found. Reading '{}' instead.\n\n",
                        requested, path
                    )),
                ));
            }
        }
    }

    // Layer 3: Add prefix — e.g., `run.sh` → try `scripts/run.sh`, `references/run.sh`, `assets/run.sh`
    for prefix in &["scripts", "references", "assets"] {
        let prefixed = format!("{}/{}", prefix, requested);
        if let Some(&path) = all_files.iter().find(|&&f| f == prefixed) {
            return Ok((
                path,
                Some(format!(
                    "Note: '{}' was not found. Reading '{}' instead.\n\n",
                    requested, path
                )),
            ));
        }
    }

    // Layer 4: Fuzzy match using shared fuzzy matching
    let candidates: Vec<(usize, &str)> =
        all_files.iter().enumerate().map(|(i, &f)| (i, f)).collect();
    if let Some((idx, kind)) = crate::fuzzy::find_best_match(requested, &candidates) {
        if !matches!(kind, crate::fuzzy::MatchKind::Exact) {
            let resolved = all_files[idx];
            return Ok((
                resolved,
                Some(format!(
                    "Note: '{}' was not found. Reading '{}' instead.\n\n",
                    requested, resolved,
                )),
            ));
        }
    }

    // No match found — return error with available files
    Err(format!(
        "File '{}' not found. Available files: {}",
        requested,
        all_files.join(", ")
    ))
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
        .filter(|s| s.enabled && permissions.has_any_enabled_for_skill(&s.metadata.name))
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

/// Placeholder in SKILL.md bodies resolved to the skill's directory, so
/// instructions can reference bundled scripts wherever the skill lives.
pub const SKILL_DIR_PLACEHOLDER: &str = "{{SKILL_DIR}}";

/// How skill file locations are presented to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillPathStyle {
    /// The client runs on this machine with its own shell (direct MCP):
    /// show the skill directory and absolute file paths so bundled scripts
    /// can be executed in place.
    #[default]
    Disk,
    /// The model is driven by LocalRouter (MCP via LLM) and has no shell:
    /// files are reachable only through `SkillRead(name, path)`.
    Virtual,
}

/// Absolute skill directory (symlinks resolved when possible).
fn skill_dir_path(skill: &SkillDefinition) -> std::path::PathBuf {
    std::fs::canonicalize(&skill.skill_dir).unwrap_or_else(|_| skill.skill_dir.clone())
}

/// Resolve skill-directory references in a SKILL.md body.
///
/// `{{SKILL_DIR}}` and the Claude Code install paths `.claude/skills/<name>/`
/// (as `~/`, `$HOME/`, `./` or bare prefixes) point at the skill's own
/// directory. With [`SkillPathStyle::Disk`] they become the absolute
/// directory; with [`SkillPathStyle::Virtual`] they become paths relative to
/// the skill, matching `SkillRead(name, path)`.
pub fn resolve_skill_body(skill: &SkillDefinition, style: SkillPathStyle) -> String {
    let dir_prefix = match style {
        SkillPathStyle::Disk => format!("{}/", skill_dir_path(skill).display()),
        SkillPathStyle::Virtual => String::new(),
    };
    let name = &skill.metadata.name;
    let mut body = skill.body.clone();
    // Longest prefixes first: the bare form is a suffix of the others
    for legacy in [
        format!("${{HOME}}/.claude/skills/{name}/"),
        format!("$HOME/.claude/skills/{name}/"),
        format!("~/.claude/skills/{name}/"),
        format!("./.claude/skills/{name}/"),
        format!(".claude/skills/{name}/"),
    ] {
        body = body.replace(&legacy, &dir_prefix);
    }
    body = body.replace(&format!("{SKILL_DIR_PLACEHOLDER}/"), &dir_prefix);
    let bare_dir = match style {
        SkillPathStyle::Disk => skill_dir_path(skill).display().to_string(),
        SkillPathStyle::Virtual => ".".to_string(),
    };
    body.replace(SKILL_DIR_PLACEHOLDER, &bare_dir)
}

/// Build the response for a skill_read tool call.
///
/// With [`SkillPathStyle::Disk`] the response carries the skill's location
/// and absolute file paths — an agent can't run a script it only has as
/// text. Files are always also readable via `SkillRead(name, path)`.
fn build_skill_read_response(
    skill: &SkillDefinition,
    tool_name: &str,
    style: SkillPathStyle,
) -> serde_json::Value {
    let mut text = String::new();
    let skill_name = &skill.metadata.name;
    let dir = skill_dir_path(skill);

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

    if style == SkillPathStyle::Disk {
        text.push_str(&format!("**Location:** `{}/`\n", dir.display()));
    }

    text.push('\n');

    let read_hint = format!(
        "Read with `{}(name=\"{}\", path=\"...\")`.",
        tool_name, skill_name
    );
    let sections: [(&str, &Vec<String>); 3] = [
        ("Scripts", &skill.scripts),
        ("References", &skill.references),
        ("Assets", &skill.assets),
    ];
    for (title, files) in sections {
        if files.is_empty() {
            continue;
        }
        text.push_str(&format!("## {}\n\n", title));
        match style {
            SkillPathStyle::Disk => {
                if title == "Scripts" {
                    text.push_str("Run scripts from their absolute path with your shell. ");
                }
                text.push_str(&read_hint);
                text.push_str("\n\n");
                for file in files {
                    text.push_str(&format!("- `{}` (`{}`)\n", dir.join(file).display(), file));
                }
            }
            SkillPathStyle::Virtual => {
                text.push_str(&read_hint);
                text.push_str("\n\n");
                for file in files {
                    text.push_str(&format!("- `{}`\n", file));
                }
            }
        }
        text.push('\n');
    }

    // Full SKILL.md body
    text.push_str("## Instructions\n\n");
    if style == SkillPathStyle::Disk {
        text.push_str(&format!(
            "> Relative paths in these instructions are relative to `{}/`.\n\n",
            dir.display()
        ));
    }
    text.push_str(&resolve_skill_body(skill, style));

    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A script-based skill like `ticket-monitor`: a root-level script and
    /// instructions written against Claude Code's install path.
    fn script_skill() -> (
        tempfile::TempDir,
        SkillManager,
        SkillsPermissions,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("ticket-monitor");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: ticket-monitor\ndescription: Watch tickets\n---\n\
             Run `.claude/skills/ticket-monitor/monitor-tickets.sh watch SU-1` in the background.\n\
             Or `~/.claude/skills/ticket-monitor/monitor-tickets.sh once`.\n\
             Or `{{SKILL_DIR}}/monitor-tickets.sh list` from `{{SKILL_DIR}}`.\n\
             See references/usage.md.\n",
        )
        .unwrap();
        std::fs::write(
            skill_dir.join("monitor-tickets.sh"),
            "#!/bin/bash\necho watching",
        )
        .unwrap();
        std::fs::write(skill_dir.join("references/usage.md"), "usage").unwrap();

        let manager = SkillManager::new();
        manager.initial_scan(&[skill_dir.display().to_string()], &[]);
        let permissions = SkillsPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        };
        let canonical = std::fs::canonicalize(&skill_dir).unwrap();
        (dir, manager, permissions, canonical)
    }

    async fn skill_read_text(
        manager: &SkillManager,
        permissions: &SkillsPermissions,
        args: serde_json::Value,
        style: SkillPathStyle,
    ) -> String {
        let Some(SkillToolResult::Response(response)) =
            handle_skill_tool_call("SkillRead", &args, manager, permissions, "SkillRead", style)
                .await
                .unwrap()
        else {
            panic!("expected a SkillRead response");
        };
        response["content"][0]["text"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn skill_read_gives_shell_clients_runnable_paths() {
        let (_tmp, manager, permissions, dir) = script_skill();
        let dir = dir.display().to_string();
        let text = skill_read_text(
            &manager,
            &permissions,
            json!({"name": "ticket-monitor"}),
            SkillPathStyle::Disk,
        )
        .await;

        assert!(text.contains(&format!("**Location:** `{dir}/`")), "{text}");
        assert!(
            text.contains(&format!(
                "- `{dir}/monitor-tickets.sh` (`monitor-tickets.sh`)"
            )),
            "{text}"
        );
        assert!(text.contains(&format!("relative to `{dir}/`")), "{text}");
        // Hardcoded Claude Code paths and the placeholder resolve to the skill
        assert!(
            text.contains(&format!("`{dir}/monitor-tickets.sh watch SU-1`")),
            "{text}"
        );
        assert!(
            text.contains(&format!("`{dir}/monitor-tickets.sh once`")),
            "{text}"
        );
        assert!(
            text.contains(&format!("`{dir}/monitor-tickets.sh list` from `{dir}`")),
            "{text}"
        );
        assert!(!text.contains(".claude/skills"), "{text}");
        assert!(!text.contains(SKILL_DIR_PLACEHOLDER), "{text}");
    }

    #[tokio::test]
    async fn skill_read_without_a_shell_keeps_paths_virtual() {
        let (_tmp, manager, permissions, dir) = script_skill();
        let text = skill_read_text(
            &manager,
            &permissions,
            json!({"name": "ticket-monitor"}),
            SkillPathStyle::Virtual,
        )
        .await;

        assert!(!text.contains(&dir.display().to_string()), "{text}");
        assert!(!text.contains("**Location:**"), "{text}");
        assert!(text.contains("- `monitor-tickets.sh`"), "{text}");
        // Paths become relative to the skill, as SkillRead(name, path) expects
        assert!(text.contains("`monitor-tickets.sh watch SU-1`"), "{text}");
        assert!(
            text.contains("`monitor-tickets.sh list` from `.`"),
            "{text}"
        );
        assert!(!text.contains(".claude/skills"), "{text}");
    }

    #[tokio::test]
    async fn skill_file_reads_stay_byte_exact_and_listings_show_paths() {
        let (_tmp, manager, permissions, dir) = script_skill();
        let script = skill_read_text(
            &manager,
            &permissions,
            json!({"name": "ticket-monitor", "path": "monitor-tickets.sh"}),
            SkillPathStyle::Disk,
        )
        .await;
        assert_eq!(script, "#!/bin/bash\necho watching");

        let listing = skill_read_text(
            &manager,
            &permissions,
            json!({"name": "ticket-monitor", "path": "."}),
            SkillPathStyle::Disk,
        )
        .await;
        assert!(
            listing.contains(&format!(
                "- monitor-tickets.sh ({}/monitor-tickets.sh)",
                dir.display()
            )),
            "{listing}"
        );
    }

    #[test]
    fn catalog_tool_and_index_mention_paths_only_for_shell_clients() {
        let (_tmp, manager, permissions, dir) = script_skill();
        let dir = dir.display().to_string();

        let disk_tool =
            &build_skill_tools(&manager, &permissions, "SkillRead", SkillPathStyle::Disk)[0];
        assert!(disk_tool
            .description
            .as_ref()
            .unwrap()
            .contains("absolute path"));
        let virtual_tool =
            &build_skill_tools(&manager, &permissions, "SkillRead", SkillPathStyle::Virtual)[0];
        assert!(!virtual_tool
            .description
            .as_ref()
            .unwrap()
            .contains("absolute path"));

        let disk_catalog = build_skill_catalog(
            &manager,
            &permissions,
            false,
            "SkillRead",
            "IndexSearch",
            SkillPathStyle::Disk,
        )
        .unwrap();
        assert!(disk_catalog.contains("run scripts from there"));
        let virtual_catalog = build_skill_catalog(
            &manager,
            &permissions,
            false,
            "SkillRead",
            "IndexSearch",
            SkillPathStyle::Virtual,
        )
        .unwrap();
        assert!(!virtual_catalog.contains("run scripts from there"));

        let disk_index = build_skill_index_entries(&manager, &permissions, SkillPathStyle::Disk);
        assert!(disk_index[0].1.contains(&format!("Location: {dir}/")));
        let virtual_index =
            build_skill_index_entries(&manager, &permissions, SkillPathStyle::Virtual);
        assert!(!virtual_index[0].1.contains(&dir));
    }

    #[cfg(unix)]
    #[test]
    fn read_skill_file_rejects_symlinks_outside_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("example");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: example\n---\nExample",
        )
        .unwrap();
        std::fs::write(dir.path().join("private.txt"), "outside data").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("private.txt"),
            skill_dir.join("references/private.txt"),
        )
        .unwrap();
        std::fs::write(skill_dir.join("references/public.txt"), "skill data").unwrap();

        let manager = SkillManager::new();
        manager.initial_scan(&[skill_dir.display().to_string()], &[]);
        let permissions = SkillsPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        };
        let error = read_skill_file(
            "example",
            "references/private.txt",
            &manager,
            &permissions,
            "SkillRead",
            "ReadFile",
            SkillPathStyle::Disk,
        )
        .unwrap_err();
        assert!(error.contains("outside the skill directory"));
        assert_eq!(
            read_skill_file(
                "example",
                "references/public.txt",
                &manager,
                &permissions,
                "SkillRead",
                "ReadFile",
                SkillPathStyle::Disk,
            )
            .unwrap(),
            "skill data"
        );
    }

    #[test]
    fn test_resolve_exact_match() {
        let files = vec!["sysinfo.sh", "scripts/build.sh"];
        let (resolved, note) = resolve_skill_file_path("sysinfo.sh", &files).unwrap();
        assert_eq!(resolved, "sysinfo.sh");
        assert!(note.is_none());
    }

    #[test]
    fn test_resolve_strip_prefix() {
        // LLM guesses scripts/sysinfo.sh but file is at root
        let files = vec!["sysinfo.sh"];
        let (resolved, note) = resolve_skill_file_path("scripts/sysinfo.sh", &files).unwrap();
        assert_eq!(resolved, "sysinfo.sh");
        assert!(note.is_some());
        assert!(note.unwrap().contains("sysinfo.sh"));
    }

    #[test]
    fn test_resolve_add_prefix() {
        // LLM guesses bare run.sh but file is in scripts/
        let files = vec!["scripts/run.sh", "references/doc.md"];
        let (resolved, note) = resolve_skill_file_path("run.sh", &files).unwrap();
        assert_eq!(resolved, "scripts/run.sh");
        assert!(note.is_some());
        assert!(note.unwrap().contains("scripts/run.sh"));
    }

    #[test]
    fn test_resolve_add_references_prefix() {
        let files = vec!["references/doc.md"];
        let (resolved, note) = resolve_skill_file_path("doc.md", &files).unwrap();
        assert_eq!(resolved, "references/doc.md");
        assert!(note.is_some());
    }

    #[test]
    fn test_resolve_fuzzy_match() {
        // Typo: sysinf.sh → sysinfo.sh
        let files = vec!["sysinfo.sh", "helper.py"];
        let (resolved, note) = resolve_skill_file_path("sysinf.sh", &files).unwrap();
        assert_eq!(resolved, "sysinfo.sh");
        assert!(note.is_some());
        assert!(note.unwrap().contains("sysinfo.sh"));
    }

    #[test]
    fn test_resolve_no_match_lists_available() {
        let files = vec!["sysinfo.sh", "helper.py"];
        let err = resolve_skill_file_path("totally-different.rb", &files).unwrap_err();
        assert!(err.contains("sysinfo.sh"));
        assert!(err.contains("helper.py"));
    }

    #[test]
    fn test_resolve_empty_files_error() {
        let files: Vec<&str> = vec![];
        let err = resolve_skill_file_path("anything.sh", &files).unwrap_err();
        assert!(err.contains("no readable files"));
    }
}
