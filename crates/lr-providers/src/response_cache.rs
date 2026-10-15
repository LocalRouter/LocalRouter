//! Public bypass controls and private authenticated cache scope.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const EXTENSION: &str = "localrouter.response_cache";
pub const PENDING: &str = "localrouter.response_cache_pending";
pub const SCOPE: &str = "localrouter.response_cache_scope";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    #[default]
    Use,
    Bypass,
    Refresh,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CacheOptions {
    pub mode: CacheMode,
}
pub fn validate_ingress(body: &Value) -> AppResult<()> {
    if body
        .pointer("/extensions/localrouter.response_cache_scope")
        .is_some()
        || body
            .pointer("/extensions/localrouter.response_cache_pending")
            .is_some()
    {
        return Err(AppError::InvalidParams(
            "Cache credential scope is server-owned".into(),
        ));
    }
    if let Some(value) = body.pointer("/extensions/localrouter.response_cache") {
        serde_json::from_value::<CacheOptions>(value.clone())
            .map_err(|e| AppError::InvalidParams(format!("Invalid response cache options: {e}")))?;
    }
    Ok(())
}
pub fn is_hit(extensions: Option<&std::collections::HashMap<String, Value>>) -> bool {
    extensions
        .and_then(|e| e.get(EXTENSION))
        .and_then(|e| e.get("hit"))
        .and_then(Value::as_bool)
        == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn public_controls_cannot_forge_identity_or_a_free_cache_receipt() {
        for value in [
            json!({"extensions":{"localrouter.response_cache_scope":"fake"}}),
            json!({"extensions":{"localrouter.response_cache":{"hit":true}}}),
            json!({"extensions":{"localrouter.response_cache":{"mode":"semantic"}}}),
        ] {
            assert!(validate_ingress(&value).is_err());
        }
        for mode in ["use", "bypass", "refresh"] {
            assert!(validate_ingress(
                &json!({"extensions":{"localrouter.response_cache":{"mode":mode}}})
            )
            .is_ok());
        }
    }
}
