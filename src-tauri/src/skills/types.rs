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
            source_path: def.source_path.clone(),
            script_count: def.scripts.len(),
            reference_count: def.references.len(),
            asset_count: def.assets.len(),
            enabled: def.enabled,
        }
    }
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
