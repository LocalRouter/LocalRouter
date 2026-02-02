//! Skill type definitions

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Metadata from SKILL.md YAML frontmatter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name (required)
    pub name: String,

    /// Version string
    #[serde(default)]
    pub version: Option<String>,

    /// Short description
    #[serde(default)]
    pub description: Option<String>,

    /// Author information
    #[serde(default)]
    pub author: Option<String>,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Additional metadata fields
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

/// A fully-parsed skill definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    /// Parsed metadata from frontmatter
    pub metadata: SkillMetadata,

    /// Raw markdown body (after frontmatter)
    pub body: String,

    /// Absolute path to the skill directory
    pub skill_dir: PathBuf,

    /// Path of the source that led to discovery (scan dir, zip, or direct path)
    pub source_path: String,

    /// List of scripts in scripts/ directory (relative paths)
    pub scripts: Vec<String>,

    /// List of reference files in references/ directory (relative paths)
    pub references: Vec<String>,

    /// List of asset files in assets/ directory (relative paths)
    pub assets: Vec<String>,

    /// Whether this skill is enabled (false if globally disabled)
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// SHA-256 hash of source zip/skill file (None for directory-based skills)
    #[serde(default)]
    pub content_hash: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// Lightweight info for listing skills (no body content)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    /// Skill name
    pub name: String,

    /// Version
    pub version: Option<String>,

    /// Short description
    pub description: Option<String>,

    /// Author
    pub author: Option<String>,

    /// Tags
    pub tags: Vec<String>,

    /// Additional metadata fields from frontmatter
    pub extra: HashMap<String, serde_yaml::Value>,

    /// Source path
    pub source_path: String,

    /// Number of scripts
    pub script_count: usize,

    /// Number of reference files
    pub reference_count: usize,

    /// Number of asset files
    pub asset_count: usize,

    /// Whether this skill is enabled
    pub enabled: bool,
}

impl From<&SkillDefinition> for SkillInfo {
    fn from(def: &SkillDefinition) -> Self {
        Self {
            name: def.metadata.name.clone(),
            version: def.metadata.version.clone(),
            description: def.metadata.description.clone(),
            author: def.metadata.author.clone(),
            tags: def.metadata.tags.clone(),
            extra: def.metadata.extra.clone(),
            source_path: def.source_path.clone(),
            script_count: def.scripts.len(),
            reference_count: def.references.len(),
            asset_count: def.assets.len(),
            enabled: def.enabled,
        }
    }
}

/// Sanitize a skill name into a valid tool name segment.
///
/// Lowercases, replaces non-`[a-z0-9_-]` with `_`, collapses consecutive `_`, trims `_` from edges.
pub fn sanitize_name(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() || ch == '-' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    // Collapse consecutive underscores
    let mut collapsed = String::with_capacity(result.len());
    let mut prev_underscore = false;
    for ch in result.chars() {
        if ch == '_' {
            if !prev_underscore {
                collapsed.push('_');
            }
            prev_underscore = true;
        } else {
            collapsed.push(ch);
            prev_underscore = false;
        }
    }
    // Trim underscores from edges
    collapsed.trim_matches('_').to_string()
}

/// Sanitize a file path into a tool name segment.
///
/// Strips known directory prefixes (`scripts/`, `references/`, `assets/`),
/// then sanitizes (dots become underscores, so `build.sh` → `build_sh`).
pub fn sanitize_tool_segment(file_path: &str) -> String {
    let stripped = file_path
        .strip_prefix("scripts/")
        .or_else(|| file_path.strip_prefix("references/"))
        .or_else(|| file_path.strip_prefix("assets/"))
        .unwrap_or(file_path);
    sanitize_name(stripped)
}

/// Result of running a script synchronously
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptRunResult {
    /// Exit code (None if process was killed)
    pub exit_code: Option<i32>,

    /// Last N lines of stdout
    pub stdout: String,

    /// Last N lines of stderr
    pub stderr: String,

    /// Whether the process timed out
    pub timed_out: bool,
}

/// Status of an async script execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncScriptStatus {
    /// Process ID
    pub pid: u32,

    /// Whether the process is still running
    pub running: bool,

    /// Exit code (None if still running or was killed)
    pub exit_code: Option<i32>,

    /// Last N lines of stdout
    pub stdout: String,

    /// Last N lines of stderr
    pub stderr: String,

    /// Whether the process timed out
    pub timed_out: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name_basic() {
        assert_eq!(sanitize_name("hello"), "hello");
        assert_eq!(sanitize_name("Hello World"), "hello_world");
        assert_eq!(sanitize_name("my-skill"), "my-skill");
        assert_eq!(sanitize_name("my_skill"), "my_skill");
    }

    #[test]
    fn test_sanitize_name_special_chars() {
        assert_eq!(sanitize_name("skill@v2!"), "skill_v2");
        assert_eq!(sanitize_name("a..b"), "a_b");
        assert_eq!(sanitize_name("___leading___"), "leading");
        assert_eq!(sanitize_name("trail___"), "trail");
    }

    #[test]
    fn test_sanitize_name_collapse_underscores() {
        assert_eq!(sanitize_name("a   b"), "a_b");
        assert_eq!(sanitize_name("a___b"), "a_b");
        assert_eq!(sanitize_name("a . b"), "a_b");
    }

    #[test]
    fn test_sanitize_tool_segment_strips_prefix() {
        assert_eq!(sanitize_tool_segment("scripts/build.sh"), "build_sh");
        assert_eq!(sanitize_tool_segment("references/api.md"), "api_md");
        assert_eq!(sanitize_tool_segment("assets/logo.png"), "logo_png");
        assert_eq!(sanitize_tool_segment("other/file.txt"), "other_file_txt");
    }

    #[test]
    fn test_sanitize_tool_segment_complex() {
        assert_eq!(
            sanitize_tool_segment("scripts/run-tests.sh"),
            "run-tests_sh"
        );
        assert_eq!(
            sanitize_tool_segment("scripts/My Script.py"),
            "my_script_py"
        );
    }
}
