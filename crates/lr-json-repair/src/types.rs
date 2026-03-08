use serde::{Deserialize, Serialize};

/// Describes a single repair action that was performed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RepairAction {
    /// Stripped markdown code fences (```json ... ```)
    StrippedMarkdownFences,
    /// Stripped leading/trailing prose around JSON
    StrippedProse,
    /// Fixed JSON syntax errors (trailing commas, unescaped chars, missing brackets, etc.)
    SyntaxRepaired,
    /// Coerced a value type to match schema (e.g., string "42" → number 42)
    TypeCoerced {
        path: String,
        from: String,
        to: String,
    },
    /// Removed a field not present in schema
    ExtraFieldRemoved { path: String },
    /// Added a missing required field with its default value
    DefaultAdded { path: String },
    /// Normalized an enum value's casing
    EnumNormalized {
        path: String,
        from: String,
        to: String,
    },
}

/// Result of a JSON repair operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    /// The original content before repair
    pub original: String,
    /// The repaired content
    pub repaired: String,
    /// Whether any modifications were made
    pub was_modified: bool,
    /// List of repair actions that were performed
    pub repairs: Vec<RepairAction>,
}

/// Configuration for JSON repair operations.
#[derive(Debug, Clone)]
pub struct RepairOptions {
    /// Fix JSON syntax errors (trailing commas, unescaped chars, missing brackets)
    pub syntax_repair: bool,
    /// Coerce values to match schema types
    pub schema_coercion: bool,
    /// Remove fields not present in schema (requires additionalProperties: false)
    pub strip_extra_fields: bool,
    /// Add default values for missing required fields
    pub add_defaults: bool,
    /// Normalize enum values (case-insensitive matching)
    pub normalize_enums: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            syntax_repair: true,
            schema_coercion: false,
            strip_extra_fields: false,
            add_defaults: false,
            normalize_enums: true,
        }
    }
}
