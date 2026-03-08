//! JSON repair and schema coercion for LLM responses.
//!
//! Provides unified streaming repair that handles both:
//! - **Syntax repair**: Fix malformed JSON (trailing commas, unescaped chars,
//!   missing brackets, markdown wrappers, single quotes, unquoted keys,
//!   Python/JS keywords) via a character-at-a-time state machine.
//! - **Schema coercion**: Fix valid JSON that doesn't match the expected schema
//!   (type coercion, enum normalization, extra field removal, default insertion).

pub mod streaming;
pub mod types;

pub use streaming::StreamingJsonRepairer;
pub use types::{RepairAction, RepairOptions, RepairResult};

use serde_json::Value;
use tracing::info;

/// Repair JSON content, optionally coercing it to match a schema.
///
/// Uses the unified `StreamingJsonRepairer` internally — same engine
/// as the streaming path, just fed the entire content at once.
pub fn repair_content(
    content: &str,
    schema: Option<&Value>,
    options: &RepairOptions,
) -> RepairResult {
    let mut repairer = StreamingJsonRepairer::new(schema.cloned(), options.clone());

    let mut repaired = repairer.push(content);
    repaired.push_str(&repairer.finish());

    let actions = repairer.take_actions();
    let was_modified = repaired != content;

    if was_modified {
        info!(
            repairs = actions.len(),
            "JSON content repaired with {} action(s)",
            actions.len()
        );
    }

    RepairResult {
        original: content.to_string(),
        repaired,
        was_modified,
        repairs: actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_repair_syntax_only() {
        let content = r#"{"name": "John", "age": 30,}"#;
        let result = repair_content(content, None, &RepairOptions::default());
        assert!(result.was_modified);
        assert!(serde_json::from_str::<Value>(&result.repaired).is_ok());
    }

    #[test]
    fn test_repair_with_schema_coercion() {
        let content = r#"{"name": "John", "age": "30"}"#;
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            }
        });
        let options = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let result = repair_content(content, Some(&schema), &options);
        assert!(result.was_modified);
        let parsed: Value = serde_json::from_str(&result.repaired).unwrap();
        assert_eq!(parsed["age"], json!(30));
    }

    #[test]
    fn test_repair_markdown_and_coerce() {
        let content = "```json\n{\"name\": \"John\", \"age\": \"30\",}\n```";
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            }
        });
        let options = RepairOptions {
            syntax_repair: true,
            schema_coercion: true,
            ..Default::default()
        };
        let result = repair_content(content, Some(&schema), &options);
        assert!(result.was_modified);
        let parsed: Value = serde_json::from_str(&result.repaired).unwrap();
        assert_eq!(parsed["age"], json!(30));
        assert!(result.repairs.len() >= 2);
    }

    #[test]
    fn test_no_repair_needed() {
        let content = r#"{"name": "John", "age": 30}"#;
        let result = repair_content(content, None, &RepairOptions::default());
        assert!(!result.was_modified);
        assert!(result.repairs.is_empty());
    }

    #[test]
    fn test_full_pipeline() {
        let content = "Here is your data:\n{\"name\": \"John\", \"age\": \"30\", \"extra\": true, \"status\": \"active\",}\nDone!";
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "status": {"type": "string", "enum": ["Active", "Inactive"]}
            },
            "additionalProperties": false
        });
        let options = RepairOptions {
            syntax_repair: true,
            schema_coercion: true,
            strip_extra_fields: true,
            normalize_enums: true,
            ..Default::default()
        };
        let result = repair_content(content, Some(&schema), &options);
        assert!(result.was_modified);
        let parsed: Value = serde_json::from_str(&result.repaired).unwrap();
        assert_eq!(parsed["age"], json!(30));
        assert_eq!(parsed["status"], json!("Active"));
        assert!(parsed.get("extra").is_none());
    }

    #[test]
    fn test_disabled_syntax_repair() {
        let content = r#"{"name": "John"}"#;
        let options = RepairOptions {
            syntax_repair: false,
            ..Default::default()
        };
        let result = repair_content(content, None, &options);
        assert!(!result.was_modified);
    }
}
