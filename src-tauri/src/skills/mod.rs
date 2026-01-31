//! Skills module - AgentSkills.io specification support
//!
//! Skills are directories containing SKILL.md files with YAML frontmatter
//! and markdown instructions, plus optional scripts/, references/, and assets/ directories.
//! Skills are discovered from configured paths, managed per-client, and exposed
//! as MCP tools through the gateway.

pub mod discovery;
pub mod executor;
pub mod manager;
pub mod mcp_tools;
pub mod types;
pub mod watcher;

pub use manager::SkillManager;
pub use types::*;
pub use watcher::SkillWatcher;
