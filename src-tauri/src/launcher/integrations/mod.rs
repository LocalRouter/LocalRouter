//! App integration implementations
//!
//! Each integration knows how to detect, configure, and launch a specific app
//! to connect to LocalRouter.

mod aider;
mod claude_code;
mod codex;
mod cursor;
mod droid;
mod goose;
mod openclaw;
mod opencode;

use super::AppIntegration;
use std::path::PathBuf;

/// Locate a binary by name, falling back to the user's login-shell PATH
/// when the process PATH doesn't include it.
///
/// macOS `.app` bundles launched from Finder/Dock inherit a stripped PATH
/// (`/usr/bin:/bin:/usr/sbin:/sbin`) that omits user-installed tools in
/// `/opt/homebrew/bin`, `~/.cargo/bin`, etc. `lr_mcp::manager::shell_env()`
/// resolves the real shell PATH (via `$SHELL -lic 'echo $PATH'`, cached);
/// we use it here so integration detection matches what the user sees in
/// their terminal.
pub(crate) fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(p) = which::which(name) {
        return Some(p);
    }
    let shell_path = lr_mcp::manager::shell_env().get("PATH")?.clone();
    // cwd only matters for relative PATH entries; absolute ones like
    // `/opt/homebrew/bin` resolve regardless. Fall back to root if the
    // real cwd isn't available (unusual, but possible in sandboxed GUI
    // contexts).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    which::which_in(name, Some(&shell_path), cwd).ok()
}

/// All known template IDs that have backend integrations
#[allow(dead_code)]
pub const KNOWN_TEMPLATE_IDS: &[&str] = &[
    "claude-code",
    "codex",
    "opencode",
    "droid",
    "aider",
    "cursor",
    "openclaw",
    "goose",
];

/// Get an integration by template ID
pub fn get_integration(template_id: &str) -> Option<Box<dyn AppIntegration>> {
    match template_id {
        "claude-code" => Some(Box::new(claude_code::ClaudeCodeIntegration)),
        "codex" => Some(Box::new(codex::CodexIntegration)),
        "opencode" => Some(Box::new(opencode::OpenCodeIntegration)),
        "droid" => Some(Box::new(droid::DroidIntegration)),
        "aider" => Some(Box::new(aider::AiderIntegration)),
        "cursor" => Some(Box::new(cursor::CursorIntegration)),
        "openclaw" => Some(Box::new(openclaw::OpenClawIntegration)),
        "goose" => Some(Box::new(goose::GooseIntegration)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_known_templates_resolve() {
        for id in KNOWN_TEMPLATE_IDS {
            let integration = get_integration(id);
            assert!(
                integration.is_some(),
                "template '{}' should resolve to an integration",
                id
            );
        }
    }

    #[test]
    fn test_unknown_template_returns_none() {
        assert!(get_integration("nonexistent").is_none());
        assert!(get_integration("").is_none());
        assert!(get_integration("custom").is_none());
    }

    #[test]
    fn test_integration_names() {
        let expected: Vec<(&str, &str)> = vec![
            ("claude-code", "Claude Code"),
            ("codex", "Codex"),
            ("opencode", "OpenCode"),
            ("droid", "Droid"),
            ("aider", "Aider"),
            ("cursor", "Cursor"),
            ("openclaw", "OpenClaw"),
            ("goose", "Goose"),
        ];

        for (id, name) in expected {
            let integration = get_integration(id).unwrap();
            assert_eq!(integration.name(), name, "name mismatch for '{}'", id);
        }
    }

    #[test]
    fn test_capability_flags() {
        // Apps that support both try_it_out and permanent_config
        for id in &["claude-code", "codex", "aider", "goose"] {
            let integration = get_integration(id).unwrap();
            assert!(
                integration.supports_try_it_out(),
                "'{}' should support try_it_out",
                id
            );
            assert!(
                integration.supports_permanent_config(),
                "'{}' should support permanent_config",
                id
            );
        }

        // Apps that are permanent-config only
        for id in &["opencode", "droid", "openclaw", "cursor"] {
            let integration = get_integration(id).unwrap();
            assert!(
                !integration.supports_try_it_out(),
                "'{}' should NOT support try_it_out",
                id
            );
            assert!(
                integration.supports_permanent_config(),
                "'{}' should support permanent_config",
                id
            );
        }
    }

    #[test]
    fn test_try_it_out_returns_terminal_command() {
        for id in &["claude-code", "codex", "aider", "goose"] {
            let integration = get_integration(id).unwrap();
            let result = integration
                .try_it_out("http://localhost:3625", "test-secret", "test-client")
                .unwrap();
            assert!(result.success, "try_it_out should succeed for '{}'", id);
            assert!(
                result.terminal_command.is_some(),
                "'{}' try_it_out should return a terminal command",
                id
            );
            assert!(
                result.modified_files.is_empty(),
                "'{}' try_it_out should not modify files",
                id
            );
        }
    }

    #[test]
    fn test_try_it_out_not_supported_returns_error() {
        for id in &["opencode", "droid", "openclaw", "cursor"] {
            let integration = get_integration(id).unwrap();
            let result =
                integration.try_it_out("http://localhost:3625", "test-secret", "test-client");
            assert!(
                result.is_err(),
                "'{}' try_it_out should return an error",
                id
            );
        }
    }

    #[test]
    fn test_config_file_integrations_configure_permanent() {
        for id in &[
            "claude-code",
            "codex",
            "opencode",
            "droid",
            "openclaw",
            "cursor",
        ] {
            let integration = get_integration(id).unwrap();
            let result = integration.configure_permanent(
                "http://localhost:3625",
                "test-secret",
                "test-client",
            );
            assert!(
                result.is_ok(),
                "{} configure_permanent should not panic",
                id
            );
            if let Ok(ref launch_result) = result {
                assert!(
                    launch_result.success,
                    "{} configure_permanent should succeed",
                    id
                );
            }
        }
    }

    #[test]
    fn test_cursor_config_path_is_platform_specific() {
        let integration = get_integration("cursor").unwrap();
        let result =
            integration.configure_permanent("http://localhost:3625", "test-secret", "test-client");
        assert!(
            result.is_ok(),
            "cursor configure_permanent should not panic"
        );

        if let Ok(ref launch_result) = result {
            assert!(launch_result.success);
            if !launch_result.modified_files.is_empty() {
                assert!(
                    launch_result.modified_files[0].contains("Cursor"),
                    "cursor config path should contain 'Cursor'"
                );
            }
        }
    }
}
