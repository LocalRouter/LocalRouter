//! Known client tool lists for indexing eligibility defaults.
//!
//! Each client template (Claude Code, Codex, etc.) exposes a set of tools
//! to the LLM. Some produce valuable indexed content (Read, Grep) while
//! others are action-only (Write, Bash). This module provides sensible
//! defaults per template.

use super::types::IndexingState;
use serde::Serialize;

/// A known tool entry with its default indexing state.
#[derive(Debug, Clone, Serialize)]
pub struct KnownToolEntry {
    /// Tool name as it appears in MCP tool calls
    pub name: &'static str,
    /// Default indexing state (Enable = index responses, Disable = skip)
    pub default_state: IndexingState,
    /// Whether this tool is indexable at all. False = action tool, shown
    /// disabled in the picker (can never be indexed regardless of state).
    pub indexable: bool,
}

/// Get the known tool list for a given client template ID.
/// Returns an empty vec for unknown templates (tools discovered at runtime).
pub fn known_tools_for_template(template_id: &str) -> Vec<KnownToolEntry> {
    match template_id {
        "claude-code" => claude_code_tools(),
        "codex" => codex_tools(),
        "aider" => aider_tools(),
        "goose" => goose_tools(),
        // Cursor, OpenCode, Droid, OpenClaw — discovered at runtime
        _ => Vec::new(),
    }
}

fn claude_code_tools() -> Vec<KnownToolEntry> {
    vec![
        // Indexable tools (produce useful content)
        KnownToolEntry { name: "Read", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "Glob", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "Grep", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "WebFetch", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "WebSearch", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "LSP", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "TaskOutput", default_state: IndexingState::Enable, indexable: true },
        // Action tools (not indexable)
        KnownToolEntry { name: "Write", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "Edit", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "Bash", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "Agent", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "NotebookEdit", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "AskUserQuestion", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "Skill", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TaskCreate", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TaskGet", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TaskList", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TaskStop", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TaskUpdate", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "TodoWrite", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "CronCreate", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "CronDelete", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "CronList", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "EnterPlanMode", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "ExitPlanMode", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "EnterWorktree", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "ExitWorktree", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "ToolSearch", default_state: IndexingState::Disable, indexable: false },
    ]
}

fn codex_tools() -> Vec<KnownToolEntry> {
    vec![
        KnownToolEntry { name: "read_file", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "file_read", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "list_directory", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "shell", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "write_file", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "file_write", default_state: IndexingState::Disable, indexable: false },
    ]
}

fn aider_tools() -> Vec<KnownToolEntry> {
    vec![
        KnownToolEntry { name: "read_file", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "run_command", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "write_file", default_state: IndexingState::Disable, indexable: false },
    ]
}

fn goose_tools() -> Vec<KnownToolEntry> {
    vec![
        KnownToolEntry { name: "read_file", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "search", default_state: IndexingState::Enable, indexable: true },
        KnownToolEntry { name: "shell", default_state: IndexingState::Disable, indexable: false },
        KnownToolEntry { name: "write_file", default_state: IndexingState::Disable, indexable: false },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_code_tools() {
        let tools = known_tools_for_template("claude-code");
        assert!(!tools.is_empty());
        // Read should be indexable and enabled
        let read = tools.iter().find(|t| t.name == "Read").unwrap();
        assert!(read.indexable);
        assert_eq!(read.default_state, IndexingState::Enable);
        // Bash should not be indexable
        let bash = tools.iter().find(|t| t.name == "Bash").unwrap();
        assert!(!bash.indexable);
        assert_eq!(bash.default_state, IndexingState::Disable);
    }

    #[test]
    fn test_unknown_template_returns_empty() {
        assert!(known_tools_for_template("unknown").is_empty());
        assert!(known_tools_for_template("cursor").is_empty());
    }

    #[test]
    fn test_codex_tools() {
        let tools = known_tools_for_template("codex");
        assert!(!tools.is_empty());
        let read = tools.iter().find(|t| t.name == "read_file").unwrap();
        assert!(read.indexable);
    }
}
