//! JSON repair and schema coercion for LLM responses.
//!
//! Provides two capabilities:
//! - **Syntax repair**: Fix malformed JSON (trailing commas, unescaped chars,
//!   missing brackets, markdown wrappers) using the `jsonrepair` crate.
//! - **Schema coercion**: Fix valid JSON that doesn't match the expected schema
//!   (type coercion, enum normalization, extra field removal, default insertion).

mod schema_coerce;
pub mod streaming;
mod syntax_repair;
pub mod types;

pub use streaming::StreamingSyntaxRepairer;
pub use types::{RepairAction, RepairOptions, RepairResult};

use serde_json::Value;
use tracing::info;

/// Repair JSON content, optionally coercing it to match a schema.
///
/// # Arguments
/// * `content` - The raw JSON content string to repair
/// * `schema` - Optional JSON schema to coerce values against
/// * `options` - Repair configuration options
///
/// # Returns
/// A `RepairResult` containing the repaired content and details of what was changed.
pub fn repair_content(
    content: &str,
    schema: Option<&Value>,
    options: &RepairOptions,
) -> RepairResult {
    let mut all_actions = Vec::new();
    let mut working = content.to_string();

    // Step 1: Syntax repair
    if options.syntax_repair {
        let (repaired, _modified, actions) = syntax_repair::repair_syntax(&working);
        all_actions.extend(actions);
        working = repaired;
    }

    // Step 2: Schema coercion (only if we have a schema and the content is valid JSON)
    if let Some(schema) = schema {
        if options.schema_coercion
            || options.strip_extra_fields
            || options.add_defaults
            || options.normalize_enums
        {
            if let Ok(parsed) = serde_json::from_str::<Value>(&working) {
                let (coerced, actions) = schema_coerce::coerce_to_schema(
                    &parsed,
                    schema,
                    options.strip_extra_fields,
                    options.add_defaults,
                    options.normalize_enums,
                );

                if !actions.is_empty() {
                    all_actions.extend(actions);
                    // Re-serialize with consistent formatting
                    if let Ok(serialized) = serde_json::to_string(&coerced) {
                        working = serialized;
                    }
                }
            }
        }
    }

    let was_modified = working != content;
    if was_modified {
        info!(
            repairs = all_actions.len(),
            "JSON content repaired with {} action(s)",
            all_actions.len()
        );
    }

    RepairResult {
        original: content.to_string(),
        repaired: working,
        was_modified,
        repairs: all_actions,
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
        // Should have markdown stripping + syntax repair + type coercion
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
        let content = r#"{"name": "John",}"#;
        let options = RepairOptions {
            syntax_repair: false,
            ..Default::default()
        };
        let result = repair_content(content, None, &options);
        // Syntax repair disabled, content unchanged
        assert!(!result.was_modified);
    }
}
