//! Replay requests explicitly disable callable tools and MCP orchestration.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
pub const EXTENSION: &str = "localrouter.replay";
#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayTools {
    #[default]
    Disabled,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ReplayOptions {
    pub tools: ReplayTools,
}
pub fn tools_disabled(extensions: Option<&HashMap<String, Value>>) -> bool {
    extensions.is_some_and(|e| e.contains_key(EXTENSION))
}
pub fn normalize_ingress(value: &mut Value) -> AppResult<()> {
    let Some(options) = value.get("extensions").and_then(|e| e.get(EXTENSION)) else {
        return Ok(());
    };
    serde_json::from_value::<ReplayOptions>(options.clone())
        .map_err(|e| AppError::InvalidParams(format!("Invalid replay controls: {e}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidParams("Replay request must be an object".into()))?;
    for key in [
        "max_tool_calls",
        "stop_server_tools_when",
        "web_search",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "functions",
        "function_call",
    ] {
        object.remove(key);
    }
    if let Some(extensions) = object.get_mut("extensions").and_then(Value::as_object_mut) {
        extensions.remove(crate::server_tools::INGRESS_EXTENSION);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn replay_removes_callable_tools_without_altering_input_or_gateway_policy() {
        let mut request = json!({"model":"p/m","messages":[{"role":"user","content":"input"}],"tools":[{"type":"function","function":{"name":"delete_files"}}],"tool_choice":"required","functions":[{"name":"legacy"}],"extensions":{"localrouter.replay":{"tools":"disabled"},"localrouter.gateway":{"zdr":true}}});
        normalize_ingress(&mut request).unwrap();
        assert!(request.get("tools").is_none());
        assert!(request.get("tool_choice").is_none());
        assert!(request.get("functions").is_none());
        assert_eq!(request["messages"][0]["content"], "input");
        assert_eq!(request["extensions"]["localrouter.gateway"]["zdr"], true);
    }
    #[test]
    fn replay_cannot_enable_live_actions_or_sneak_in_unknown_controls() {
        for options in [
            json!({"tools":"live"}),
            json!({"execute":true}),
            json!(true),
        ] {
            let mut request = json!({"extensions":{"localrouter.replay":options}});
            assert!(normalize_ingress(&mut request).is_err());
        }
    }
}
