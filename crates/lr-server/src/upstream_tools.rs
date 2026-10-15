//! Validate public server-tool variants before legacy function-tool conversion.
use lr_providers::server_tools::{
    PositionedServerTool, ServerToolDefinition, ServerToolsContext, INGRESS_EXTENSION,
};
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum PublicFunctionTool {
    #[serde(rename = "function")]
    Function {
        function: crate::types::FunctionDefinition,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PublicChatTool {
    Function(PublicFunctionTool),
    Server(ServerToolDefinition),
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum PublicResponseFunctionTool {
    #[serde(rename = "function")]
    Function {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        parameters: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PublicResponseTool {
    Function(PublicResponseFunctionTool),
    Server(ServerToolDefinition),
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum PublicForcedServerTool {
    #[serde(rename = "web_search")]
    WebSearch,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PublicToolChoice {
    Function(crate::types::ToolChoice),
    Server(PublicForcedServerTool),
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}

pub fn normalize_ingress(value: &mut Value, path: &str) -> AppResult<()> {
    if value.get("max_tool_calls").is_some_and(Value::is_null) {
        if let Some(object) = value.as_object_mut() {
            object.remove("max_tool_calls");
        }
    }
    if value
        .get("extensions")
        .and_then(Value::as_object)
        .is_some_and(|extensions| extensions.contains_key(INGRESS_EXTENSION))
    {
        return Err(invalid(
            "Server-tool context is private; provide typed tools in the public tools array",
        ));
    }
    if value.get("stop_server_tools_when").is_some() || value.get("web_search").is_some() {
        return Err(invalid("Use typed search/fetch tools and verified max_tool_calls; this shorthand or stop-condition mapping is not supported"));
    }
    let definitions = value.get("tools").and_then(Value::as_array);
    let Some(definitions) = definitions else {
        if value.get("max_tool_calls").is_some() {
            return Err(invalid(
                "A server-tool step cap requires supported server-tool definitions",
            ));
        }
        return Ok(());
    };
    let mut server = Vec::new();
    let mut functions = Vec::new();
    for (position, definition) in definitions.iter().enumerate() {
        let kind = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if kind == "function" {
            functions.push(definition.clone());
            continue;
        }
        let definition=serde_json::from_value::<ServerToolDefinition>(definition.clone()).map_err(|_|invalid("Unsupported server tool type or parameters; only verified search/fetch variants are accepted"))?;
        definition.validate()?;
        server.push(PositionedServerTool {
            position,
            definition,
        });
    }
    if server.is_empty() {
        if value.get("max_tool_calls").is_some() {
            return Err(invalid(
                "A server-tool step cap requires supported server-tool definitions",
            ));
        }
        return Ok(());
    }
    if !matches!(
        path.strip_prefix("/v1").unwrap_or(path),
        "/chat/completions" | "/responses"
    ) {
        return Err(invalid(
            "Server tools currently support Chat Completions and Responses only",
        ));
    }
    if value
        .get("previous_response_id")
        .is_some_and(|value| !value.is_null())
    {
        return Err(invalid(
            "Server tools currently require a fresh explicit request history",
        ));
    }
    let mut context = ServerToolsContext {
        tools: server,
        original_tool_count: definitions.len(),
        max_tool_calls: value
            .get("max_tool_calls")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| invalid("Invalid server-tool step cap"))?,
        forced_type: None,
        policy_max_tool_calls: None,
    };
    if let Some(choice) = value.get("tool_choice") {
        if let Some(choice) = choice.as_str() {
            if !matches!(choice, "auto" | "none" | "required") {
                return Err(invalid("Invalid tool choice"));
            }
        } else if let Some(kind) = choice.get("type").and_then(Value::as_str) {
            if kind != "function" {
                if choice.as_object().is_none_or(|object| object.len() != 1) {
                    return Err(invalid("Unsupported forced server-tool choice options"));
                }
                context.forced_type = Some(kind.into());
            }
        } else {
            return Err(invalid("Invalid tool choice"));
        }
    }
    context.validate()?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("Request must be an object"))?;
    object.remove("max_tool_calls");
    if context.forced_type.is_some() {
        object.remove("tool_choice");
    }
    if functions.is_empty() {
        object.remove("tools");
    } else {
        object.insert("tools".into(), Value::Array(functions));
    }
    let extensions = object
        .entry("extensions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("Extensions must be an object"))?;
    extensions.insert(
        INGRESS_EXTENSION.into(),
        serde_json::to_value(context).map_err(|_| invalid("Invalid server-tool context"))?,
    );
    Ok(())
}

pub fn context(
    extensions: Option<&std::collections::HashMap<String, Value>>,
) -> AppResult<Option<Box<ServerToolsContext>>> {
    extensions
        .and_then(|extensions| extensions.get(INGRESS_EXTENSION))
        .map(|value| {
            let context: ServerToolsContext = serde_json::from_value(value.clone())
                .map_err(|_| invalid("Invalid private server-tool context"))?;
            context.validate()?;
            Ok(Box::new(context))
        })
        .transpose()
}

/// Monitor/export uses the public representation rather than the private
/// projection. This never executes a tool or fetches a cited URL.
pub fn public_representation(value: &Value) -> Value {
    let mut value = value.clone();
    let raw = value
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
        .and_then(|extensions| extensions.remove(INGRESS_EXTENSION));
    let Some(context) = raw.and_then(|raw| serde_json::from_value::<ServerToolsContext>(raw).ok())
    else {
        return value;
    };
    if context.validate().is_err() {
        return value;
    }
    let mut tools = value
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tools.len() + context.tools.len() != context.original_tool_count {
        return value;
    }
    let mut native = context.tools.iter().collect::<Vec<_>>();
    native.sort_by_key(|tool| tool.position);
    for tool in native {
        tools.insert(
            tool.position,
            serde_json::to_value(&tool.definition).unwrap_or(Value::Null),
        );
    }
    value["tools"] = Value::Array(tools);
    if let Some(cap) = context.max_tool_calls {
        value["max_tool_calls"] = json!(cap);
    }
    if let Some(kind) = context.forced_type {
        value["tool_choice"] = json!({"type":kind});
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_ingress_preserves_mixed_order_without_inventing_function_definitions() {
        let original = json!({"model":"gateway/model","messages":[{"role":"user","content":"Find sources"}],"max_tool_calls":3,"tools":[{"type":"function","function":{"name":"client_tool","parameters":{}}},{"type":"openrouter:web_fetch","parameters":{"engine":"exa","max_content_tokens":500}}]});
        let mut value = original.clone();
        normalize_ingress(&mut value, "/v1/chat/completions").unwrap();
        let request: crate::types::ChatCompletionRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.tools.as_ref().unwrap().len(), 1);
        let context = context(request.extensions.as_ref()).unwrap().unwrap();
        assert_eq!(context.tools.len(), 1);
        assert_eq!(context.original_tool_count, 2);
        assert_eq!(context.max_tool_calls, Some(3));
        let public = public_representation(&value);
        assert_eq!(public["tools"], original["tools"]);
        assert_eq!(public["max_tool_calls"], 3);
        assert!(public["extensions"].get(INGRESS_EXTENSION).is_none());
    }
    #[test]
    fn private_context_and_unverified_controls_never_bypass_public_validation() {
        for mut value in [
            json!({"extensions":{INGRESS_EXTENSION:{"tools":[]}}}),
            json!({"stop_server_tools_when":[{}]}),
            json!({"web_search":true}),
            json!({"max_tool_calls":2,"tools":[{"type":"function","function":{"name":"client","parameters":{}}}]}),
            json!({"tools":[{"type":"openrouter:shell"}]}),
            json!({"previous_response_id":"resp_old","tools":[{"type":"web_search"}]}),
        ] {
            assert!(normalize_ingress(&mut value, "/v1/chat/completions").is_err());
        }
        let mut request = json!({"tools":[{"type":"web_search"}]});
        assert!(normalize_ingress(&mut request, "/v1/systemone").is_err());
    }
    #[test]
    fn forced_native_choices_remain_typed_and_do_not_enter_function_tool_choice() {
        let mut value = json!({"tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"},"max_tool_calls":null});
        normalize_ingress(&mut value, "/responses").unwrap();
        assert!(value.get("tool_choice").is_none());
        assert!(value.get("tools").is_none());
        assert_eq!(
            value["extensions"][INGRESS_EXTENSION]["forced_type"],
            "web_search"
        );
    }
}
