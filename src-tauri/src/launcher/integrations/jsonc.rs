//! Minimal JSONC (JSON-with-comments) support.
//!
//! VS Code and Zed both store their settings as JSONC: standard JSON plus `//`
//! and `/* */` comments, which `serde_json` rejects. We only need to *read*
//! those files well enough to merge a couple of keys, so this strips comments
//! before parsing rather than implementing a full JSONC round-trip.
//!
//! Consequence, surfaced to the user wherever this is used: rewriting such a
//! file **drops the user's comments**. The write path always takes a backup
//! first, and the shipped default settings for these editors are mostly
//! comments, so this is a real cost — never rewrite one of these files without
//! saying so.

/// Remove `//` line comments and `/* */` block comments, leaving string
/// literals (which may legitimately contain `//`, e.g. a URL) untouched.
///
/// Comment bodies are replaced by nothing, but newlines inside block comments
/// are preserved so byte offsets in error messages stay roughly meaningful.
pub fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    // Line comment: consume to end of line, keep the newline.
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next(); // consume '*'
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        if c == '\n' {
                            out.push('\n');
                        }
                        prev = c;
                    }
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }

    out
}

/// Parse a JSONC document into a `serde_json::Value`.
///
/// An empty (or whitespace/comment-only) document parses as an empty object,
/// which is what an editor with no settings yet effectively has.
pub fn parse(input: &str) -> Result<serde_json::Value, String> {
    let stripped = strip_comments(input);
    if stripped.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    // Trailing commas are legal in JSONC and common in hand-edited settings.
    let cleaned = strip_trailing_commas(&stripped);
    serde_json::from_str(&cleaned).map_err(|e| format!("Failed to parse settings JSON: {e}"))
}

/// Remove trailing commas before `}` or `]`, outside of string literals.
fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;

    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            // Look ahead past whitespace for a closing brace/bracket.
            let next = chars[i + 1..]
                .iter()
                .find(|ch| !ch.is_whitespace())
                .copied();
            if matches!(next, Some('}') | Some(']')) {
                continue; // drop the trailing comma
            }
        }
        out.push(c);
    }

    out
}

/// Whether the document contains any comment that a rewrite would discard.
/// Used to decide whether to warn the user.
pub fn has_comments(input: &str) -> bool {
    strip_comments(input).trim() != input.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_comments() {
        let src = "{\n // a comment\n \"a\": 1, /* inline */ \"b\": 2\n}";
        let v = parse(src).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn preserves_double_slash_inside_strings() {
        let src = r#"{"url": "http://example.com/x"}"#;
        let v = parse(src).unwrap();
        assert_eq!(v["url"], "http://example.com/x");
    }

    #[test]
    fn preserves_escaped_quote_inside_strings() {
        let src = r#"{"a": "he said \" // not a comment"}"#;
        let v = parse(src).unwrap();
        assert_eq!(v["a"], r#"he said " // not a comment"#);
    }

    #[test]
    fn handles_trailing_commas() {
        let src = "{\n \"a\": 1,\n \"b\": [1, 2,],\n}";
        let v = parse(src).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"][1], 2);
    }

    #[test]
    fn does_not_drop_comma_inside_string() {
        let src = r#"{"a": "x,", "b": 1}"#;
        let v = parse(src).unwrap();
        assert_eq!(v["a"], "x,");
        assert_eq!(v["b"], 1);
    }

    #[test]
    fn empty_or_comment_only_parses_as_empty_object() {
        assert_eq!(parse("").unwrap(), serde_json::json!({}));
        assert_eq!(parse("// nothing here\n").unwrap(), serde_json::json!({}));
    }

    #[test]
    fn detects_comments_for_the_data_loss_warning() {
        assert!(has_comments("{\n// hi\n\"a\":1}"));
        assert!(!has_comments(r#"{"a":1}"#));
        // A `//` inside a string is not a comment.
        assert!(!has_comments(r#"{"u":"http://x"}"#));
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse("{ not json").is_err());
    }

    // --- adversarial inputs: a mistake here corrupts a user's editor settings

    #[test]
    fn string_ending_in_escaped_backslash_does_not_swallow_the_next_token() {
        // "a\\" closes the string; the following // really is a comment.
        let src = r#"{"a": "x\\", // c
 "b": 2}"#;
        let v = parse(src).unwrap();
        assert_eq!(v["a"], r"x\");
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn unterminated_block_comment_does_not_panic() {
        // Truncated file: must be a clean error, never a panic or a
        // half-parsed document we'd then write back.
        let _ = parse("{\n/* never closed\n\"a\": 1}");
    }

    #[test]
    fn trailing_slash_at_end_of_input_is_preserved_not_panicked_on() {
        let _ = parse("{\"a\": 1}/");
    }

    #[test]
    fn comma_then_whitespace_inside_a_string_is_not_stripped() {
        let src = "{\"a\": \"x,   \", \"b\": 1}";
        let v = parse(src).unwrap();
        assert_eq!(v["a"], "x,   ");
    }

    #[test]
    fn comma_at_very_end_of_input_does_not_index_out_of_bounds() {
        // strip_trailing_commas looks ahead past the comma — the last
        // character must not panic.
        let _ = parse("{\"a\": 1,");
        let _ = parse(",");
    }

    #[test]
    fn multibyte_content_is_handled_by_char_not_byte_offsets() {
        let src = "{\"emoji\": \"🎉,\", /* 日本語 */ \"b\": 2}";
        let v = parse(src).unwrap();
        assert_eq!(v["emoji"], "🎉,");
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn round_trip_of_a_realistic_settings_file_keeps_every_value() {
        let src = r#"{
  // editor
  "editor.fontSize": 13,
  "editor.rulers": [80, 120,],
  "workbench.colorTheme": "Default Dark+",
  /* proxy is off */
  "http.proxyStrictSSL": true,
  "terminal.integrated.env.osx": { "PATH": "/usr/local/bin://x" },
}"#;
        let v = parse(src).unwrap();
        assert_eq!(v["editor.fontSize"], 13);
        assert_eq!(v["editor.rulers"][1], 120);
        assert_eq!(v["workbench.colorTheme"], "Default Dark+");
        assert_eq!(v["http.proxyStrictSSL"], true);
        assert_eq!(
            v["terminal.integrated.env.osx"]["PATH"],
            "/usr/local/bin://x"
        );
    }
}
