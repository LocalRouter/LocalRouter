//! Shared dotenv (`.env`) merging for tools that read env vars from a file.
//!
//! Several tools we integrate with load a dotenv file at startup and export its
//! keys into their own process environment — Codex (`~/.codex/.env`), OpenClaw
//! (`~/.openclaw/.env`), and Aider (`~/.env`). Writing those files is how the
//! HTTPS inspection proxy gets configured permanently for them.
//!
//! The merge is deliberately line-based rather than parse-and-reserialize:
//! these files are hand-edited and routinely hold API keys and comments, so we
//! only touch the lines we own.

/// Merge `updates` (key/value pairs) into an existing `.env` body, preserving
/// every other line and comment verbatim.
///
/// - An existing assignment for a key is replaced **in place**, keeping its
///   position in the file.
/// - `export KEY=…` and leading-whitespace forms are recognized and replaced
///   (normalized to a plain `KEY=value`).
/// - Later duplicate assignments of the same key are dropped: dotenv gives the
///   **last** occurrence precedence, so a stale duplicate below the line we
///   rewrote would silently win.
/// - Keys not present are appended in `updates` order.
///
/// The file's existing line ending style is preserved: a CRLF file stays CRLF,
/// so a version-controlled `~/.env` shows a two-line diff rather than a
/// whole-file rewrite.
///
/// The result always ends with a trailing newline.
pub fn merge_env(existing: &str, updates: &[(&str, &str)]) -> String {
    let newline = detect_newline(existing);
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();

    for (key, value) in updates {
        let assignment = format!("{key}={value}");
        let prefix = format!("{key}=");
        // Match `KEY=`, with optional leading whitespace and `export `.
        let is_key_line = |line: &str| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix("export ")
                .unwrap_or(trimmed)
                .starts_with(&prefix)
        };

        let mut found = false;
        lines.retain_mut(|line| {
            if is_key_line(line) {
                if found {
                    // Drop stale duplicates below the one we rewrote.
                    return false;
                }
                *line = assignment.clone();
                found = true;
            }
            true
        });
        if !found {
            lines.push(assignment);
        }
    }

    join_lines(&lines, newline)
}

/// The line ending the file already uses: CRLF only if it actually appears,
/// otherwise LF (which is also the right default for a new file).
fn detect_newline(existing: &str) -> &'static str {
    if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn join_lines(lines: &[String], newline: &str) -> String {
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

/// Remove assignments for `keys` from a `.env` body, leaving everything else
/// untouched. Used to undo proxy configuration.
pub fn remove_env_keys(existing: &str, keys: &[&str]) -> String {
    let newline = detect_newline(existing);
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    lines.retain(|line| {
        let trimmed = line.trim_start();
        let bare = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        !keys.iter().any(|k| bare.starts_with(&format!("{k}=")))
    });
    join_lines(&lines, newline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_into_empty() {
        let out = merge_env("", &[("A", "1"), ("B", "2")]);
        assert_eq!(out, "A=1\nB=2\n");
    }

    #[test]
    fn preserves_comments_and_other_keys_and_replaces_in_place() {
        let existing = "# header\nOPENAI_API_KEY=sk-123\n\nA=old\n";
        let out = merge_env(existing, &[("A", "new"), ("B", "2")]);
        assert_eq!(out, "# header\nOPENAI_API_KEY=sk-123\n\nA=new\nB=2\n");
    }

    #[test]
    fn replaces_export_prefixed_and_indented_assignments() {
        let out = merge_env("  export A=old\n", &[("A", "new")]);
        assert_eq!(out, "A=new\n");
    }

    #[test]
    fn drops_later_duplicates_because_dotenv_last_wins() {
        let existing = "A=first\nKEEP=yes\nA=second\n";
        let out = merge_env(existing, &[("A", "new")]);
        assert_eq!(out, "A=new\nKEEP=yes\n");
    }

    #[test]
    fn does_not_match_key_prefixes() {
        // `AB=` must not be treated as an assignment of `A`.
        let out = merge_env("AB=keep\n", &[("A", "1")]);
        assert_eq!(out, "AB=keep\nA=1\n");
    }

    #[test]
    fn handles_missing_trailing_newline() {
        let out = merge_env("A=1", &[("B", "2")]);
        assert_eq!(out, "A=1\nB=2\n");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        // A Windows ~/.env rewritten with LF would show up as a whole-file
        // diff, contradicting the "only the proxy keys change" promise.
        let existing = "# c\r\nA=1\r\n";
        let out = merge_env(existing, &[("B", "2")]);
        assert_eq!(out, "# c\r\nA=1\r\nB=2\r\n");
        assert!(!out.contains("\n\n"));
    }

    #[test]
    fn crlf_preserved_on_removal_too() {
        let out = remove_env_keys("# c\r\nA=1\r\nKEEP=2\r\n", &["A"]);
        assert_eq!(out, "# c\r\nKEEP=2\r\n");
    }

    #[test]
    fn lf_files_stay_lf() {
        let out = merge_env("A=1\n", &[("B", "2")]);
        assert_eq!(out, "A=1\nB=2\n");
    }

    #[test]
    fn remove_strips_only_named_keys() {
        let existing = "# c\nA=1\nexport B=2\nKEEP=3\n";
        let out = remove_env_keys(existing, &["A", "B"]);
        assert_eq!(out, "# c\nKEEP=3\n");
    }
}
