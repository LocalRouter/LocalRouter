use crate::types::RepairAction;
use tracing::debug;

/// Strip markdown code fences from content.
/// Handles ```json, ```, and ` ``` ` patterns.
fn strip_markdown_fences(content: &str) -> Option<String> {
    let trimmed = content.trim();

    // Match ```json ... ``` or ``` ... ```
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Skip optional language tag (json, JSON, etc.)
        let rest = if let Some(after_tag) = rest.strip_prefix("json") {
            after_tag
        } else if let Some(after_tag) = rest.strip_prefix("JSON") {
            after_tag
        } else {
            rest
        };

        // Skip newline after opening fence
        let rest = rest.strip_prefix('\n').unwrap_or(rest);

        // Remove closing fence
        if let Some(inner) = rest.strip_suffix("```") {
            let inner = inner.strip_suffix('\n').unwrap_or(inner);
            return Some(inner.to_string());
        }
    }

    None
}

/// Strip leading/trailing prose around JSON.
/// Finds the first `{` or `[` and last `}` or `]` and extracts that substring.
fn strip_prose(content: &str) -> Option<String> {
    let trimmed = content.trim();

    // Only attempt if content doesn't start with JSON
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return None;
    }

    // Find first JSON-starting character
    let start = trimmed.find(['{', '[']);
    if let Some(start_idx) = start {
        let bracket = trimmed.as_bytes()[start_idx];
        let closing = if bracket == b'{' { '}' } else { ']' };

        // Find last occurrence of the matching closing bracket
        if let Some(end_idx) = trimmed.rfind(closing) {
            if end_idx > start_idx {
                return Some(trimmed[start_idx..=end_idx].to_string());
            }
        }
    }

    None
}

/// Repair JSON syntax errors using the jsonrepair crate.
/// Returns (repaired_content, was_modified, actions).
pub fn repair_syntax(content: &str) -> (String, bool, Vec<RepairAction>) {
    let mut actions = Vec::new();
    let mut working = content.to_string();

    // Step 1: Strip markdown fences
    if let Some(stripped) = strip_markdown_fences(&working) {
        actions.push(RepairAction::StrippedMarkdownFences);
        working = stripped;
    }

    // Step 2: Strip prose
    if let Some(stripped) = strip_prose(&working) {
        actions.push(RepairAction::StrippedProse);
        working = stripped;
    }

    // Step 3: Run jsonrepair
    match jsonrepair::repair_to_string(&working, &jsonrepair::Options::default()) {
        Ok(repaired) => {
            if repaired != working {
                actions.push(RepairAction::SyntaxRepaired);
                working = repaired;
            }
        }
        Err(e) => {
            debug!("jsonrepair failed, returning best effort: {}", e);
            // jsonrepair failed - return what we have
        }
    }

    let was_modified = working != content;
    (working, was_modified, actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_json_unchanged() {
        let input = r#"{"name": "John", "age": 30}"#;
        let (result, modified, actions) = repair_syntax(input);
        assert_eq!(result, input);
        assert!(!modified);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_trailing_comma_object() {
        let input = r#"{"name": "John", "age": 30,}"#;
        let (result, modified, _actions) = repair_syntax(input);
        assert!(modified);
        // Should parse as valid JSON
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_trailing_comma_array() {
        let input = r#"[1, 2, 3,]"#;
        let (result, modified, _actions) = repair_syntax(input);
        assert!(modified);
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_markdown_fences_json() {
        let input = "```json\n{\"name\": \"John\"}\n```";
        let (result, modified, actions) = repair_syntax(input);
        assert!(modified);
        assert!(actions.contains(&RepairAction::StrippedMarkdownFences));
        assert_eq!(result, r#"{"name": "John"}"#);
    }

    #[test]
    fn test_markdown_fences_no_lang() {
        let input = "```\n{\"key\": \"value\"}\n```";
        let (result, modified, actions) = repair_syntax(input);
        assert!(modified);
        assert!(actions.contains(&RepairAction::StrippedMarkdownFences));
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_prose_around_json() {
        let input = "Here is the JSON:\n{\"name\": \"John\"}\nHope that helps!";
        let (result, modified, actions) = repair_syntax(input);
        assert!(modified);
        assert!(actions.contains(&RepairAction::StrippedProse));
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_missing_closing_bracket() {
        let input = r#"{"name": "John", "age": 30"#;
        let (result, modified, _actions) = repair_syntax(input);
        assert!(modified);
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_single_quotes() {
        let input = "{'name': 'John'}";
        let (result, modified, _actions) = repair_syntax(input);
        assert!(modified);
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_unquoted_keys() {
        let input = "{name: \"John\"}";
        let (result, modified, _actions) = repair_syntax(input);
        assert!(modified);
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_markdown_with_trailing_comma() {
        let input = "```json\n{\"name\": \"John\", \"age\": 30,}\n```";
        let (result, modified, actions) = repair_syntax(input);
        assert!(modified);
        assert!(actions.contains(&RepairAction::StrippedMarkdownFences));
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_empty_object() {
        let input = "{}";
        let (result, modified, _) = repair_syntax(input);
        assert_eq!(result, "{}");
        assert!(!modified);
    }

    #[test]
    fn test_empty_array() {
        let input = "[]";
        let (result, modified, _) = repair_syntax(input);
        assert_eq!(result, "[]");
        assert!(!modified);
    }

    #[test]
    fn test_nested_objects() {
        let input = r#"{"user": {"name": "John", "address": {"city": "NYC",},},}"#;
        let (result, modified, _) = repair_syntax(input);
        assert!(modified);
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }
}
