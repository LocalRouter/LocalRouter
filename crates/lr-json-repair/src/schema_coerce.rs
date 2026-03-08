use crate::types::RepairAction;
use serde_json::Value;
use tracing::debug;

/// Coerce a JSON value to match a JSON schema.
/// Returns (coerced_value, actions).
pub fn coerce_to_schema(
    value: &Value,
    schema: &Value,
    strip_extra_fields: bool,
    add_defaults: bool,
    normalize_enums: bool,
) -> (Value, Vec<RepairAction>) {
    let mut actions = Vec::new();
    let result = coerce_recursive(
        value,
        schema,
        String::new(),
        strip_extra_fields,
        add_defaults,
        normalize_enums,
        &mut actions,
    );
    (result, actions)
}

fn coerce_recursive(
    value: &Value,
    schema: &Value,
    path: String,
    strip_extra_fields: bool,
    add_defaults: bool,
    normalize_enums: bool,
    actions: &mut Vec<RepairAction>,
) -> Value {
    // Handle enum normalization first (before type coercion)
    if normalize_enums {
        if let Some(enum_values) = schema.get("enum").and_then(|e| e.as_array()) {
            if let Some(normalized) = normalize_enum_value(value, enum_values) {
                if &normalized != value {
                    actions.push(RepairAction::EnumNormalized {
                        path: path.clone(),
                        from: value.to_string(),
                        to: normalized.to_string(),
                    });
                    return normalized;
                }
            }
        }
    }

    let schema_type = schema.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match schema_type {
        "object" => coerce_object(
            value,
            schema,
            &path,
            strip_extra_fields,
            add_defaults,
            normalize_enums,
            actions,
        ),
        "array" => coerce_array(
            value,
            schema,
            &path,
            strip_extra_fields,
            add_defaults,
            normalize_enums,
            actions,
        ),
        "string" => coerce_to_string(value, &path, actions),
        "number" | "integer" => coerce_to_number(value, schema_type, &path, actions),
        "boolean" => coerce_to_boolean(value, &path, actions),
        _ => value.clone(),
    }
}

fn coerce_object(
    value: &Value,
    schema: &Value,
    path: &str,
    strip_extra_fields: bool,
    add_defaults: bool,
    normalize_enums: bool,
    actions: &mut Vec<RepairAction>,
) -> Value {
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return value.clone(),
    };

    let properties = schema.get("properties").and_then(|p| p.as_object());

    let additional_properties = schema
        .get("additionalProperties")
        .and_then(|a| a.as_bool())
        .unwrap_or(true);

    let mut result = serde_json::Map::new();

    for (key, val) in obj {
        let field_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", path, key)
        };

        // Check if field is in schema properties
        if let Some(props) = properties {
            if let Some(prop_schema) = props.get(key) {
                // Field is in schema - coerce recursively
                let coerced = coerce_recursive(
                    val,
                    prop_schema,
                    field_path,
                    strip_extra_fields,
                    add_defaults,
                    normalize_enums,
                    actions,
                );
                result.insert(key.clone(), coerced);
            } else if strip_extra_fields && !additional_properties {
                // Field not in schema and additionalProperties is false - remove
                debug!("Removed extra field: {}", field_path);
                actions.push(RepairAction::ExtraFieldRemoved { path: field_path });
            } else {
                // Keep the field as-is
                result.insert(key.clone(), val.clone());
            }
        } else {
            // No properties defined in schema, keep as-is
            result.insert(key.clone(), val.clone());
        }
    }

    // Add missing required fields with defaults
    if add_defaults {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            if let Some(props) = properties {
                for req in required {
                    if let Some(req_key) = req.as_str() {
                        if !result.contains_key(req_key) {
                            if let Some(prop_schema) = props.get(req_key) {
                                if let Some(default_val) = prop_schema.get("default") {
                                    let field_path = if path.is_empty() {
                                        req_key.to_string()
                                    } else {
                                        format!("{}.{}", path, req_key)
                                    };
                                    actions.push(RepairAction::DefaultAdded { path: field_path });
                                    result.insert(req_key.to_string(), default_val.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Value::Object(result)
}

fn coerce_array(
    value: &Value,
    schema: &Value,
    path: &str,
    strip_extra_fields: bool,
    add_defaults: bool,
    normalize_enums: bool,
    actions: &mut Vec<RepairAction>,
) -> Value {
    let arr = match value.as_array() {
        Some(arr) => arr,
        None => return value.clone(),
    };

    let items_schema = schema.get("items");

    let result: Vec<Value> = arr
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let item_path = format!("{}[{}]", path, i);
            if let Some(item_schema) = items_schema {
                coerce_recursive(
                    item,
                    item_schema,
                    item_path,
                    strip_extra_fields,
                    add_defaults,
                    normalize_enums,
                    actions,
                )
            } else {
                item.clone()
            }
        })
        .collect();

    Value::Array(result)
}

fn coerce_to_string(value: &Value, path: &str, actions: &mut Vec<RepairAction>) -> Value {
    match value {
        Value::String(_) => value.clone(),
        Value::Number(n) => {
            let s = n.to_string();
            actions.push(RepairAction::TypeCoerced {
                path: path.to_string(),
                from: "number".to_string(),
                to: "string".to_string(),
            });
            Value::String(s)
        }
        Value::Bool(b) => {
            actions.push(RepairAction::TypeCoerced {
                path: path.to_string(),
                from: "boolean".to_string(),
                to: "string".to_string(),
            });
            Value::String(b.to_string())
        }
        Value::Null => {
            actions.push(RepairAction::TypeCoerced {
                path: path.to_string(),
                from: "null".to_string(),
                to: "string".to_string(),
            });
            Value::String(String::new())
        }
        _ => value.clone(),
    }
}

fn coerce_to_number(
    value: &Value,
    schema_type: &str,
    path: &str,
    actions: &mut Vec<RepairAction>,
) -> Value {
    match value {
        Value::Number(_) => {
            // If schema says integer and we have a float, truncate
            if schema_type == "integer" {
                if let Some(f) = value.as_f64() {
                    let i = f as i64;
                    if (f - i as f64).abs() > f64::EPSILON {
                        actions.push(RepairAction::TypeCoerced {
                            path: path.to_string(),
                            from: "number".to_string(),
                            to: "integer".to_string(),
                        });
                        return Value::Number(serde_json::Number::from(i));
                    }
                }
            }
            value.clone()
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if schema_type == "integer" {
                if let Ok(i) = trimmed.parse::<i64>() {
                    actions.push(RepairAction::TypeCoerced {
                        path: path.to_string(),
                        from: "string".to_string(),
                        to: "integer".to_string(),
                    });
                    return Value::Number(serde_json::Number::from(i));
                }
            }
            if let Ok(f) = trimmed.parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    actions.push(RepairAction::TypeCoerced {
                        path: path.to_string(),
                        from: "string".to_string(),
                        to: "number".to_string(),
                    });
                    return Value::Number(n);
                }
            }
            value.clone()
        }
        Value::Bool(b) => {
            actions.push(RepairAction::TypeCoerced {
                path: path.to_string(),
                from: "boolean".to_string(),
                to: schema_type.to_string(),
            });
            Value::Number(serde_json::Number::from(if *b { 1 } else { 0 }))
        }
        _ => value.clone(),
    }
}

fn coerce_to_boolean(value: &Value, path: &str, actions: &mut Vec<RepairAction>) -> Value {
    match value {
        Value::Bool(_) => value.clone(),
        Value::String(s) => {
            let lower = s.trim().to_lowercase();
            let b = match lower.as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" | "" => Some(false),
                _ => None,
            };
            if let Some(b) = b {
                actions.push(RepairAction::TypeCoerced {
                    path: path.to_string(),
                    from: "string".to_string(),
                    to: "boolean".to_string(),
                });
                Value::Bool(b)
            } else {
                value.clone()
            }
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                actions.push(RepairAction::TypeCoerced {
                    path: path.to_string(),
                    from: "number".to_string(),
                    to: "boolean".to_string(),
                });
                Value::Bool(i != 0)
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn normalize_enum_value(value: &Value, enum_values: &[Value]) -> Option<Value> {
    if let Value::String(s) = value {
        let lower = s.to_lowercase();
        for ev in enum_values {
            if let Value::String(es) = ev {
                if es.to_lowercase() == lower && es != s {
                    return Some(Value::String(es.clone()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_string_to_number() {
        let schema = json!({"type": "number"});
        let value = json!("42.5");
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!(42.5));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_string_to_integer() {
        let schema = json!({"type": "integer"});
        let value = json!("42");
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!(42));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_number_to_string() {
        let schema = json!({"type": "string"});
        let value = json!(42);
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!("42"));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_string_to_boolean() {
        let schema = json!({"type": "boolean"});
        let value = json!("true");
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!(true));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_boolean_to_string() {
        let schema = json!({"type": "string"});
        let value = json!(true);
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!("true"));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_strip_extra_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "additionalProperties": false
        });
        let value = json!({"name": "John", "age": 30, "extra": "field"});
        let (result, actions) = coerce_to_schema(&value, &schema, true, false, false);
        assert_eq!(result, json!({"name": "John", "age": 30}));
        assert!(actions
            .iter()
            .any(|a| matches!(a, RepairAction::ExtraFieldRemoved { path } if path == "extra")));
    }

    #[test]
    fn test_add_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "status": {"type": "string", "default": "active"}
            },
            "required": ["name", "status"]
        });
        let value = json!({"name": "John"});
        let (result, actions) = coerce_to_schema(&value, &schema, false, true, false);
        assert_eq!(result, json!({"name": "John", "status": "active"}));
        assert!(actions
            .iter()
            .any(|a| matches!(a, RepairAction::DefaultAdded { path } if path == "status")));
    }

    #[test]
    fn test_enum_normalization() {
        let schema = json!({
            "type": "string",
            "enum": ["Active", "Inactive", "Pending"]
        });
        let value = json!("active");
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, true);
        assert_eq!(result, json!("Active"));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_nested_coercion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "age": {"type": "integer"}
                    }
                }
            }
        });
        let value = json!({"user": {"name": "John", "age": "30"}});
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!({"user": {"name": "John", "age": 30}}));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_array_coercion() {
        let schema = json!({
            "type": "array",
            "items": {"type": "integer"}
        });
        let value = json!(["1", "2", "3"]);
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!([1, 2, 3]));
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn test_no_coercion_when_valid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            }
        });
        let value = json!({"name": "John", "age": 30});
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, value);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_float_to_integer_truncation() {
        let schema = json!({"type": "integer"});
        let value = json!(3.7);
        let (result, actions) = coerce_to_schema(&value, &schema, false, false, false);
        assert_eq!(result, json!(3));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_combined_operations() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "status": {"type": "string", "enum": ["Active", "Inactive"], "default": "Active"}
            },
            "required": ["name", "age", "status"],
            "additionalProperties": false
        });
        let value = json!({"name": "John", "age": "30", "extra": true});
        let (result, actions) = coerce_to_schema(&value, &schema, true, true, true);
        assert_eq!(
            result,
            json!({"name": "John", "age": 30, "status": "Active"})
        );
        // Should have: type coerced age, removed extra, added default status
        assert_eq!(actions.len(), 3);
    }
}
