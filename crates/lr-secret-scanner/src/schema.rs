//! Local, bounded validation of the effective restored output and tool arguments.
use lr_types::{AppError, AppResult};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ResponseContract {
    tools: BTreeMap<String, Value>,
    output: Option<Value>,
    incompatible: bool,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
fn check_schema(
    value: &Value,
    root: &Value,
    depth: usize,
    references: &mut Vec<String>,
    budget: &mut usize,
) -> AppResult<()> {
    if depth > 64 || *budget == 0 {
        return Err(invalid("Protected schema exceeds validation budget"));
    }
    *budget -= 1;
    match value {
        Value::Object(map) => {
            if map.contains_key("$id") {
                return Err(invalid(
                    "Protected schema validation does not support schema IDs",
                ));
            }
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                if !reference.starts_with("#/") {
                    return Err(invalid(
                        "Protected schema validation requires local non-recursive JSON pointers",
                    ));
                }
                if references.iter().any(|seen| seen == reference) {
                    return Err(invalid("Recursive protected schemas are unsupported"));
                }
                let target = root
                    .pointer(&reference[1..])
                    .ok_or_else(|| invalid("Missing protected schema reference"))?;
                references.push(reference.into());
                check_schema(target, root, depth + 1, references, budget)?;
                references.pop();
            }
            for value in schema_children(value) {
                check_schema(value, root, depth + 1, references, budget)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                check_schema(value, root, depth + 1, references, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn validate(schema: &Value, value: &Value) -> AppResult<()> {
    if schema.to_string().len() > 1024 * 1024 {
        return Err(invalid("Protected schema exceeds size limit"));
    }
    check_schema(schema, schema, 0, &mut Vec::new(), &mut 100_000)?;
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(schema)
        .map_err(|_| invalid("Invalid protected response schema"))?;
    if !compiled.is_valid(value) {
        return Err(invalid("Restored content does not satisfy its schema"));
    }
    Ok(())
}
fn schema_children(value: &Value) -> Vec<&Value> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let mut children = Vec::new();
    for key in [
        "$defs",
        "definitions",
        "properties",
        "patternProperties",
        "dependentSchemas",
        "dependencies",
    ] {
        if let Some(values) = map.get(key).and_then(Value::as_object) {
            children.extend(values.values());
        }
    }
    for key in [
        "items",
        "additionalItems",
        "additionalProperties",
        "unevaluatedProperties",
        "contains",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
    ] {
        if let Some(value) = map.get(key) {
            if let Some(values) = value.as_array() {
                children.extend(values);
            } else {
                children.push(value);
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(values) = map.get(key).and_then(Value::as_array) {
            children.extend(values);
        }
    }
    children
}
fn incompatible(value: &Value) -> bool {
    let constrained = value.as_object().is_some_and(|map| {
        ["pattern", "format", "maxLength", "minLength"]
            .iter()
            .any(|key| map.contains_key(*key))
            || map
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(Value::is_string))
            || map.get("const").is_some_and(Value::is_string)
    });
    constrained || schema_children(value).into_iter().any(incompatible)
}
impl ResponseContract {
    pub fn from_request(request: &Value, active: bool) -> AppResult<Self> {
        let mut result = Self::default();
        if let Some(tools) = request.get("tools").and_then(Value::as_array) {
            for tool in tools {
                if tool.get("type").and_then(Value::as_str) == Some("custom")
                    && tool.pointer("/format/type").and_then(Value::as_str) == Some("grammar")
                {
                    result.incompatible = true;
                }
                let function = tool.get("function").unwrap_or(tool);
                if let (Some(name), Some(schema)) = (
                    function.get("name").and_then(Value::as_str),
                    function
                        .get("parameters")
                        .or_else(|| function.get("input_schema")),
                ) {
                    if function.get("strict").and_then(Value::as_bool) == Some(true)
                        && incompatible(schema)
                    {
                        result.incompatible = true;
                    }
                    result.tools.insert(name.into(), schema.clone());
                }
            }
        }
        let format = request
            .pointer("/response_format/json_schema")
            .or_else(|| request.pointer("/text/format"));
        if let Some(format) = format {
            if let Some(schema) = format.get("schema") {
                if format.get("strict").and_then(Value::as_bool) == Some(true)
                    && incompatible(schema)
                {
                    result.incompatible = true;
                }
                result.output = Some(schema.clone());
            }
        }
        if active {
            result.check_placeholders()?;
        }
        Ok(result)
    }
    pub fn check_placeholders(&self) -> AppResult<()> {
        if self.incompatible {
            return Err(invalid(
                "Strict schema has string constraints incompatible with secret placeholders",
            ));
        }
        Ok(())
    }
    pub fn tool(&self, name: Option<&str>, arguments: &Value) -> AppResult<()> {
        if self.tools.is_empty() {
            return Ok(());
        }
        let schema = name
            .and_then(|name| self.tools.get(name))
            .ok_or_else(|| invalid("Protected tool call has no matching request schema"))?;
        validate(schema, arguments)
    }
    pub fn structured(&self) -> bool {
        self.output.is_some()
    }
    pub fn output(&self, text: &str) -> AppResult<()> {
        if let Some(schema) = &self.output {
            let value: Value = serde_json::from_str(text)
                .map_err(|_| invalid("Restored structured output is not valid JSON"))?;
            validate(schema, &value)?;
        }
        Ok(())
    }
    pub fn response(&self, response: &Value) -> AppResult<()> {
        fn walk(contract: &ResponseContract, value: &Value) -> AppResult<()> {
            match value {
                Value::Object(map) => {
                    if let Some(arguments) = map.get("arguments").filter(|_| {
                        map.contains_key("name")
                            || map.get("type").and_then(Value::as_str) == Some("function_call")
                    }) {
                        let args = match arguments {
                            Value::String(s) => serde_json::from_str(s).map_err(|_| {
                                invalid("Restored tool arguments are not valid JSON")
                            })?,
                            value => value.clone(),
                        };
                        contract.tool(map.get("name").and_then(Value::as_str), &args)?;
                    }
                    if map.get("type").and_then(Value::as_str) == Some("tool_use") {
                        if let Some(args) = map.get("input") {
                            contract.tool(map.get("name").and_then(Value::as_str), args)?;
                        }
                    }
                    // Only provider output text participates in structured-output schemas.
                    if map.get("role").and_then(Value::as_str) == Some("assistant") {
                        if let Some(text) = map.get("content").and_then(Value::as_str) {
                            contract.output(text)?;
                        }
                    }
                    if map
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|s| matches!(s, "output_text" | "text"))
                    {
                        if let Some(text) = map.get("text").and_then(Value::as_str) {
                            contract.output(text)?;
                        }
                    }
                    for (key, value) in map {
                        if !matches!(key.as_str(), "arguments" | "input") {
                            walk(contract, value)?;
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        walk(contract, value)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        walk(self, response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_constraints_fail_before_dispatch_without_weakening() {
        for constraint in [
            json!({"pattern":"^abc$"}),
            json!({"maxLength":16}),
            json!({"enum":["abc"]}),
            json!({"const":"abc"}),
            json!({"format":"uuid"}),
        ] {
            let request = json!({"text":{"format":{"type":"json_schema","strict":true,"schema":{"type":"object","properties":{"secret":constraint}}}}});
            assert!(ResponseContract::from_request(&request, true).is_err());
            assert!(ResponseContract::from_request(&request, false).is_ok());
        }
    }
    #[test]
    fn restored_values_require_correct_schema_and_local_references() {
        let contract=ResponseContract::from_request(&json!({"tools":[{"type":"function","name":"write","parameters":{"type":"object","required":["content"],"properties":{"content":{"type":"string","maxLength":5}},"additionalProperties":false}}]}),true).unwrap();
        assert!(contract
            .tool(Some("write"), &json!({"content":"short"}))
            .is_ok());
        assert!(contract
            .tool(Some("write"), &json!({"content":"too long"}))
            .is_err());
        assert!(contract
            .tool(Some("unknown"), &json!({"content":"short"}))
            .is_err());
        assert!(validate(
            &json!({"$ref":"https://example.invalid/schema"}),
            &json!({})
        )
        .is_err());
        assert!(validate(
            &json!({"$defs":{"x":{"type":"string"}},"$ref":"#/$defs/x"}),
            &json!("ok")
        )
        .is_ok());
    }
}

#[cfg(test)]
mod validation_boundary_tests {
    use super::*;
    use crate::placeholders::Vault;
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    use std::sync::Arc;
    #[test]
    fn recursive_and_rebased_schemas_cannot_resolve_external_resources() {
        assert!(validate(
            &json!({"$defs":{"loop":{"$ref":"#/$defs/loop"}},"$ref":"#/$defs/loop"}),
            &json!({})
        )
        .is_err());
        assert!(validate(
            &json!({"$id":"https://example.invalid/remote","type":"string"}),
            &json!("test")
        )
        .is_err());
    }
    #[tokio::test]
    async fn constraints_are_rechecked_when_a_later_tool_introduces_a_secret() {
        let vault = Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "late-schema",
            "scope",
            true,
        );
        vault.set_request_contract(&json!({"text":{"format":{"strict":true,"schema":{"type":"string","maxLength":5}}}})).unwrap();
        vault.register("late synthetic secret").unwrap();
        assert!(vault
            .prepare(&mut json!({"content":"late synthetic secret"}))
            .await
            .is_err());
    }
    #[test]
    fn non_strict_masked_enum_is_validated_against_its_restored_original() {
        let vault = Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "enum-schema",
            "scope",
            true,
        );
        vault.register("synthetic enum secret").unwrap();
        let mut request = json!({"input":"synthetic enum secret","tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"content":{"enum":["synthetic enum secret"]}},"required":["content"]}}]});
        vault.protect(&mut request).unwrap();
        assert!(!request.to_string().contains("synthetic enum secret"));
        vault.set_request_contract(&request).unwrap();
        assert!(vault
            .validate_tool(Some("write"), &json!({"content":"synthetic enum secret"}))
            .is_ok());
        assert!(vault
            .validate_tool(Some("write"), &json!({"content":"other"}))
            .is_err());
    }
}

#[cfg(test)]
mod schema_keyword_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn property_names_and_examples_are_not_mistaken_for_schema_keywords() {
        let schema = json!({"type":"object","properties":{"format":{"type":"string"},"pattern":{"type":"string"},"$id":{"type":"string"}},"examples":[{"$ref":"https://example.invalid/example","format":"date-time"}]});
        assert!(!incompatible(&schema));
        assert!(validate(
            &schema,
            &json!({"format":"text","pattern":"value","$id":"id"})
        )
        .is_ok());
    }
    #[test]
    fn custom_grammars_require_a_verified_placeholder_mapping() {
        let request = json!({"tools":[{"type":"custom","name":"code","format":{"type":"grammar","syntax":"regex","definition":"[a-z]+"}}]});
        assert!(ResponseContract::from_request(&request, true).is_err());
        assert!(ResponseContract::from_request(&request, false).is_ok());
    }
}
