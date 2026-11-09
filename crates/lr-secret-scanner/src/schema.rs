//! Local, bounded validation of the effective restored output and tool arguments.
use lr_types::{AppError, AppResult};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ResponseContract {
    tools: BTreeMap<String, Value>,
    aliases: BTreeMap<String, Option<String>>,
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
fn verified_codex_grammar(tool: &Value) -> bool {
    tool.pointer("/format/syntax").and_then(Value::as_str) == Some("lark")
        && tool
            .pointer("/format/definition")
            .and_then(Value::as_str)
            .is_some_and(|definition| {
                [
                    include_str!("codex-grammars/apply_patch.lark"),
                    include_str!("codex-grammars/exec.lark"),
                    include_str!("codex-grammars/js_repl.lark"),
                ]
                .iter()
                .any(|known| known.trim().lines().eq(definition.trim().lines()))
            })
}

fn tool_definitions<'a>(
    tools: &'a [Value],
    prefix: &str,
    depth: usize,
    output: &mut Vec<(String, &'a Value)>,
    budget: &mut usize,
) -> AppResult<()> {
    if depth > 8 || tools.len() > *budget {
        return Err(invalid(
            "Protected tool definitions exceed inspection budget",
        ));
    }
    *budget -= tools.len();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) == Some("namespace") {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Missing tool namespace name"))?;
            if name.is_empty() || name.len() > 256 || prefix.len().saturating_add(name.len()) > 1024
            {
                return Err(invalid(
                    "Protected tool namespace name exceeds inspection budget",
                ));
            }
            let children = tool
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Missing tool namespace definitions"))?;
            tool_definitions(
                children,
                &format!("{prefix}{name}."),
                depth + 1,
                output,
                budget,
            )?;
        } else {
            output.push((prefix.into(), tool));
        }
    }
    Ok(())
}

pub(crate) fn qualified_tool_name(value: &Value) -> Option<String> {
    let name = value.get("name").and_then(Value::as_str)?;
    Some(
        match value
            .get("namespace")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
        {
            Some(namespace) => format!("{namespace}.{name}"),
            None => name.into(),
        },
    )
}

/// Tool declarations in ordinary Responses and Codex Responses Lite. Lite
/// transports carry developer `additional_tools` items in the input prefix.
pub(crate) fn request_tool_groups(request: &Value) -> Vec<&[Value]> {
    request
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .into_iter()
        .chain(
            request
                .get("input")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "additional_tools")
                .filter_map(|item| {
                    item.get("tools")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                }),
        )
        .collect()
}

impl ResponseContract {
    pub fn from_request(request: &Value, active: bool) -> AppResult<Self> {
        let mut result = Self::default();
        let mut budget = 1024;
        for tools in request_tool_groups(request) {
            let mut definitions = Vec::new();
            tool_definitions(tools, "", 0, &mut definitions, &mut budget)?;
            for (prefix, tool) in definitions {
                if tool.get("type").and_then(Value::as_str) == Some("custom")
                    && tool.pointer("/format/type").and_then(Value::as_str) == Some("grammar")
                    && !verified_codex_grammar(tool)
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
                    if name.is_empty() || name.len() > 256 {
                        return Err(invalid("Protected tool name exceeds inspection budget"));
                    }
                    let qualified = format!("{prefix}{name}");
                    if result
                        .tools
                        .insert(qualified.clone(), schema.clone())
                        .is_some()
                    {
                        return Err(invalid("Duplicate protected tool definition"));
                    }
                    result
                        .aliases
                        .entry(name.into())
                        .and_modify(|alias| *alias = None)
                        .or_insert(Some(qualified));
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
            .and_then(|name| {
                self.tools.get(name).or_else(|| {
                    self.aliases
                        .get(name)
                        .and_then(Option::as_ref)
                        .and_then(|key| self.tools.get(key))
                })
            })
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
                        contract.tool(qualified_tool_name(value).as_deref(), &args)?;
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
    async fn unrelated_constraints_do_not_block_later_secret_masking() {
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
            .is_ok());
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
            .is_ok());
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
    fn codex_default_grammars_and_namespaces_are_compatible_but_changes_are_not_trusted() {
        for definition in [
            include_str!("codex-grammars/apply_patch.lark"),
            include_str!("codex-grammars/exec.lark"),
            include_str!("codex-grammars/js_repl.lark"),
        ] {
            let mut request = json!({"tools":[{"type":"web_search"},{"type":"custom","name":"local_code","format":{"type":"grammar","syntax":"lark","definition":definition}},{"type":"namespace","name":"files","tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}]}]});
            let contract = ResponseContract::from_request(&request, true).unwrap();
            assert!(contract
                .tool(Some("files.write"), &json!({"value":"secret"}))
                .is_ok());
            assert!(contract
                .tool(Some("write"), &json!({"value":"secret"}))
                .is_ok());
            assert!(contract
                .tool(Some("files.write"), &json!({"value":42}))
                .is_err());
            request["tools"][1]["format"]["definition"] = json!("start: /[a-z]+/");
            assert!(ResponseContract::from_request(&request, true).is_err());
        }
    }

    #[test]
    fn namespaced_tool_validation_keeps_same_named_tools_distinct() {
        let request = json!({"tools":[
            {"type":"namespace","name":"alpha","tools":[{"type":"function","name":"write","parameters":{"type":"string"}}]},
            {"type":"namespace","name":"beta","tools":[{"type":"function","name":"write","parameters":{"type":"number"}}]}
        ]});
        let contract = ResponseContract::from_request(&request, true).unwrap();
        assert!(contract.tool(Some("write"), &json!("text")).is_err());
        assert!(contract.response(&json!({"type":"function_call","namespace":"alpha","name":"write","arguments":"\"text\""})).is_ok());
        assert!(contract.response(&json!({"type":"function_call","namespace":"beta","name":"write","arguments":"\"text\""})).is_err());
        assert!(contract
            .response(
                &json!({"type":"function_call","namespace":"beta","name":"write","arguments":"42"})
            )
            .is_ok());
    }

    #[test]
    fn namespace_metadata_is_bounded_and_duplicates_are_rejected() {
        for tool in [
            json!({"type":"namespace","tools":[]}),
            json!({"type":"namespace","name":"files"}),
            json!({"type":"namespace","name":"x".repeat(1025),"tools":[]}),
            json!({"type":"namespace","name":"","tools":[]}),
        ] {
            assert!(ResponseContract::from_request(&json!({"tools":[tool]}), true).is_err());
        }
        assert!(ResponseContract::from_request(
            &json!({"tools":[{"type":"function","name":"x".repeat(1025),"parameters":{}}]}),
            true
        )
        .is_err());
        let tool = json!({"type":"function","name":"write","parameters":{"type":"object"}});
        assert!(ResponseContract::from_request(
            &json!({"tools":[tool.clone(),tool.clone()]}),
            true
        )
        .is_err());
        assert!(
            ResponseContract::from_request(&json!({"tools":vec![tool.clone();1025]}), true)
                .is_err()
        );
        let mut nested = tool;
        for _ in 0..10 {
            nested = json!({"type":"namespace","name":"nested","tools":[nested]});
        }
        assert!(ResponseContract::from_request(&json!({"tools":[nested]}), true).is_err());
    }

    #[test]
    fn custom_grammars_require_a_verified_placeholder_mapping() {
        let request = json!({"tools":[{"type":"custom","name":"code","format":{"type":"grammar","syntax":"regex","definition":"[a-z]+"}}]});
        assert!(ResponseContract::from_request(&request, true).is_err());
        assert!(ResponseContract::from_request(&request, false).is_ok());
    }
}

#[cfg(test)]
mod lite_tests {
    use super::*;
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    use std::sync::Arc;
    #[test]
    fn lite_tool_schemas_restore_and_validate_under_their_namespace() {
        let vault = crate::placeholders::Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "lite-contract",
            "scope",
            false,
        );
        let secret = "synthetic schema secret";
        vault.register(secret).unwrap();
        let mut request = json!({"input":[{"type":"additional_tools","role":"developer","tools":[{"type":"namespace","name":"functions","tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"content":{"enum":[secret]}},"required":["content"]}}]}]},{"role":"user","content":secret}]});
        vault.protect(&mut request).unwrap();
        assert!(!request.to_string().contains(secret));
        vault.set_request_contract(&request).unwrap();
        vault
            .validate_tool(Some("functions.write"), &json!({"content":secret}))
            .unwrap();
        assert!(vault
            .validate_tool(Some("functions.write"), &json!({"content":"wrong"}))
            .is_ok());
        assert!(vault
            .validate_tool(Some("unrelated.write"), &json!({"content":secret}))
            .is_ok());
    }
    #[test]
    fn pinned_tool_grammars_accept_platform_line_endings_without_accepting_changed_rules() {
        for known in [
            include_str!("codex-grammars/apply_patch.lark"),
            include_str!("codex-grammars/exec.lark"),
            include_str!("codex-grammars/js_repl.lark"),
        ] {
            let crlf = known.replace("\r\n", "\n").replace('\n', "\r\n");
            let mut body = json!({"input":[{"type":"additional_tools","tools":[{"type":"custom","name":"exec","format":{"type":"grammar","syntax":"lark","definition":crlf}}]}]});
            ResponseContract::from_request(&body, true).unwrap();
            body["input"][0]["tools"][0]["format"]["definition"] =
                json!(format!("{crlf}\nchanged_rule: /untrusted/"));
            assert!(ResponseContract::from_request(&body, true).is_err());
        }
    }
    #[test]
    fn lite_tool_grammars_and_inspection_budgets_are_enforced() {
        let body = json!({"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"exec","format":{"type":"grammar","syntax":"lark","definition":"unknown grammar"}}]}]}]});
        assert!(ResponseContract::from_request(&body, true).is_err());
        let body = json!({"input":(0..1025).map(|_|json!({"type":"additional_tools","tools":[{"type":"web_search"}]})).collect::<Vec<_>>()});
        assert!(ResponseContract::from_request(&body, true).is_err());
    }
}
