//! Explicit local session controls. Raw session identifiers never go upstream.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const EXTENSION: &str = "localrouter.session";
pub const CONTROL_TOKEN: &str = "localrouter.session_control_token";
pub const CACHE_KEY: &str = "localrouter.prompt_cache_key";
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionOptions {
    pub id: String,
    #[serde(default)]
    pub reset: bool,
    #[serde(default)]
    pub reclassify: bool,
    /// Refuse migration/expiry rather than silently moving provider-bound state.
    #[serde(default)]
    pub require_continuity: bool,
}
impl SessionOptions {
    pub fn parse(value: Option<&Value>) -> AppResult<Option<Self>> {
        value.map(|value| {
            let options: Self = serde_json::from_value(value.clone()).map_err(|e| AppError::InvalidParams(format!("Invalid {EXTENSION}: {e}")))?;
            if options.id.is_empty() || options.id.len() > 256 || options.id.chars().any(char::is_control) {
                return Err(AppError::InvalidParams("Session id must contain 1–256 bytes without control characters".into()));
            }
            if options.reset && options.require_continuity { return Err(AppError::InvalidParams("Session reset cannot preserve provider-bound continuation; start a new conversation".into())); }
            Ok(options)
        }).transpose()
    }
    pub fn from_request(request: &crate::CompletionRequest) -> AppResult<Option<Self>> {
        Self::parse(request.extensions.as_ref().and_then(|e| e.get(EXTENSION)))
    }
}
pub fn cache_key(request: &crate::CompletionRequest) -> Option<String> {
    request
        .extensions
        .as_ref()?
        .get(CACHE_KEY)?
        .as_str()
        .map(str::to_owned)
}

/// Normalize supported caller session headers without guessing sessions from user identity.
pub fn normalize_ingress(value: &mut Value, header_session: Option<&str>) -> AppResult<()> {
    if value
        .get("extensions")
        .and_then(|e| e.get(CONTROL_TOKEN))
        .is_some()
    {
        return Err(AppError::InvalidParams(
            "Session control tokens are internal".into(),
        ));
    }
    let mut controls = value
        .get("extensions")
        .and_then(|e| e.get(EXTENSION))
        .cloned();
    if let Some(id) = header_session {
        controls
            .get_or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| AppError::InvalidParams("Session options must be an object".into()))?
            .insert("id".into(), Value::String(id.into()));
    }
    let mut options = SessionOptions::parse(controls.as_ref())?;
    fn bound_state(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return true;
        }
        match value {
            Value::Object(fields) => fields.iter().any(|(key, v)| {
                (matches!(
                    key.as_str(),
                    "signature" | "thought_signature" | "encrypted_content"
                ) && v.as_str().is_some_and(|s| !s.is_empty()))
                    || bound_state(v, depth + 1)
            }),
            Value::Array(values) => values.iter().any(|v| bound_state(v, depth + 1)),
            _ => false,
        }
    }
    let bound = value.get("messages").is_some_and(|v| bound_state(v, 0))
        || value.get("input").is_some_and(|v| bound_state(v, 0));
    if bound {
        if let Some(options) = options.as_mut() {
            if options.reset {
                return Err(AppError::InvalidParams(
                    "A session reset cannot carry provider-bound continuation state".into(),
                ));
            }
            options.require_continuity = true;
        } else if value.get("model").and_then(Value::as_str) == Some("localrouter/auto") {
            return Err(AppError::InvalidParams("Provider-bound continuation requires an established session or explicit provider/model; start a new conversation if the session expired".into()));
        }
    }
    let explicit_key = value.get("prompt_cache_key").cloned().or_else(|| {
        value
            .get("extensions")
            .and_then(|e| e.get(CACHE_KEY))
            .cloned()
    });
    if let Some(key) = &explicit_key {
        if key
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
        {
            return Err(AppError::InvalidParams(
                "prompt_cache_key must contain 1–256 bytes without control characters".into(),
            ));
        }
    }
    if options.is_some() || explicit_key.is_some() {
        let extensions = value
            .as_object_mut()
            .ok_or_else(|| AppError::InvalidParams("Request must be an object".into()))?
            .entry("extensions")
            .or_insert_with(|| serde_json::json!({}));
        let extensions = extensions
            .as_object_mut()
            .ok_or_else(|| AppError::InvalidParams("extensions must be an object".into()))?;
        if let Some(options) = options {
            if options.reset || options.reclassify {
                extensions.insert(
                    CONTROL_TOKEN.into(),
                    Value::String(uuid::Uuid::new_v4().to_string()),
                );
            }
            extensions.insert(EXTENSION.into(), serde_json::to_value(options).unwrap());
        }
        if let Some(key) = explicit_key {
            extensions.insert(CACHE_KEY.into(), key);
        }
    }
    Ok(())
}

/// Provider prompt reuse, distinct from serving a saved response. Price effects are estimates.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PromptCacheAccounting {
    pub read_tokens: u32,
    pub write_tokens: u32,
    pub uncached_tokens: u32,
    pub estimated_net_savings_usd: Option<f64>,
    pub source: String,
}
pub const ACCOUNTING: &str = "localrouter.prompt_cache";
pub fn prompt_cache_accounting(
    pricing: &crate::PricingInfo,
    usage: &crate::TokenUsage,
) -> Option<PromptCacheAccounting> {
    let details = usage.prompt_tokens_details.as_ref()?;
    let read = details
        .cache_read_tokens
        .or(details.cached_tokens)
        .unwrap_or(0)
        .min(usage.prompt_tokens);
    let write = details
        .cache_creation_tokens
        .unwrap_or(0)
        .min(usage.prompt_tokens - read);
    let known = (read == 0 || pricing.cache_read_cost_per_1k.is_some())
        && (write == 0 || pricing.cache_write_cost_per_1k.is_some());
    let savings = known
        .then(|| {
            usage.prompt_tokens as f64 * pricing.input_cost_per_1k / 1000.0
                - pricing.input_cost(usage)
        })
        .filter(|n| n.is_finite());
    Some(PromptCacheAccounting {
        read_tokens: read,
        write_tokens: write,
        uncached_tokens: usage.prompt_tokens - read - write,
        estimated_net_savings_usd: savings,
        source: "provider_tokens_pricing_estimate".into(),
    })
}

/// An HTTP turn resets/reclassifies once, even when MCP requests several model steps.
pub fn consume_turn_controls(request: &mut crate::CompletionRequest) {
    if let Some(options) = request
        .extensions
        .as_mut()
        .and_then(|e| e.get_mut(EXTENSION))
        .and_then(Value::as_object_mut)
    {
        options.insert("reset".into(), Value::Bool(false));
        options.insert("reclassify".into(), Value::Bool(false));
    }
}

/// A control action is applied once throughout a logical HTTP/MCP turn.
pub fn prepare_turn(request: &mut crate::CompletionRequest) -> AppResult<()> {
    if SessionOptions::from_request(request)?.is_some_and(|o| o.reset || o.reclassify) {
        request
            .extensions
            .get_or_insert_with(Default::default)
            .entry(CONTROL_TOKEN.into())
            .or_insert_with(|| Value::String(uuid::Uuid::new_v4().to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn headers_override_body_and_never_use_user_identity_as_session() {
        let mut value = json!({"model":"m","user":"shared-user","extensions":{"localrouter.session":{"id":"body","reclassify":true}}});
        normalize_ingress(&mut value, Some("header")).unwrap();
        let options = SessionOptions::parse(value.pointer("/extensions/localrouter.session"))
            .unwrap()
            .unwrap();
        assert_eq!(options.id, "header");
        assert!(options.reclassify);
        let mut value = json!({"model":"m","user":"shared-user"});
        normalize_ingress(&mut value, None).unwrap();
        assert!(value.get("extensions").is_none());
    }
    #[test]
    fn rejects_malformed_controls_and_requires_bound_session() {
        for value in [
            json!({"id":""}),
            json!({"id":"x\n"}),
            json!({"id":"x","unknown":true}),
            json!({"id":"x","reset":true,"require_continuity":true}),
        ] {
            assert!(SessionOptions::parse(Some(&value)).is_err());
        }
        let mut bound = json!({"model":"localrouter/auto","input":[{"type":"reasoning","encrypted_content":"opaque"}]});
        assert!(normalize_ingress(&mut bound, None).is_err());
        normalize_ingress(&mut bound, Some("turn")).unwrap();
        assert_eq!(
            bound.pointer("/extensions/localrouter.session/require_continuity"),
            Some(&json!(true))
        );
    }
    #[test]
    fn raw_session_is_consumed_and_turn_reset_is_only_once() {
        let mut request = crate::CompletionRequest::new("m", vec![]);
        request.extensions = Some(
            [(
                EXTENSION.into(),
                json!({"id":"private","reset":true,"reclassify":true}),
            )]
            .into_iter()
            .collect(),
        );
        prepare_turn(&mut request).unwrap();
        consume_turn_controls(&mut request);
        let options = SessionOptions::from_request(&request).unwrap().unwrap();
        assert!(!options.reset && !options.reclassify);
        assert!(request.without_routing_metadata().extensions.is_none());
    }
    #[test]
    fn cache_savings_follow_actual_rates_and_include_write_premiums() {
        let mut pricing = crate::PricingInfo {
            input_cost_per_1k: 1.0,
            cache_read_cost_per_1k: Some(0.5),
            cache_write_cost_per_1k: Some(2.0),
            ..crate::PricingInfo::free()
        };
        let mut usage = crate::TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 0,
            total_tokens: 1000,
            prompt_tokens_details: Some(crate::PromptTokensDetails {
                anthropic_cache_creation_1h_tokens: None,
                cached_tokens: Some(500),
                cache_read_tokens: None,
                cache_creation_tokens: Some(500),
            }),
            completion_tokens_details: None,
        };
        let accounting = prompt_cache_accounting(&pricing, &usage).unwrap();
        assert_eq!(accounting.estimated_net_savings_usd, Some(-0.25));
        assert_eq!(accounting.uncached_tokens, 0);
        pricing.cache_write_cost_per_1k = None;
        assert!(prompt_cache_accounting(&pricing, &usage)
            .unwrap()
            .estimated_net_savings_usd
            .is_none());
        usage
            .prompt_tokens_details
            .as_mut()
            .unwrap()
            .cache_creation_tokens = Some(u32::MAX);
        let accounting = prompt_cache_accounting(&pricing, &usage).unwrap();
        assert_eq!(accounting.write_tokens, 500);
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::ModelProvider;
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::{
        matchers::{body_partial_json, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    #[tokio::test]
    async fn openai_cache_keys_and_detailed_usage_survive_normal_and_streaming_transport() {
        let server = MockServer::start().await;
        let usage = json!({"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":7,"cache_creation_tokens":2},"completion_tokens_details":{"reasoning_tokens":3}});
        Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"prompt_cache_key":"caller-choice","stream":false}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"normal","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":usage}))).expect(1).mount(&server).await;
        let frame = json!({"id":"stream","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":usage});
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(
                json!({"prompt_cache_key":"caller-choice","stream":true}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("data: {frame}\n\ndata: [DONE]\n\n"))
                    .insert_header("Content-Type", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider =
            crate::openai::OpenAIProvider::with_base_url("key".into(), server.uri()).unwrap();
        let mut request = crate::CompletionRequest::new("m", vec![]);
        request.extensions = Some(
            [(CACHE_KEY.into(), json!("caller-choice"))]
                .into_iter()
                .collect(),
        );
        let normal = provider.complete(request.clone()).await.unwrap();
        assert_eq!(
            normal
                .usage
                .prompt_tokens_details
                .as_ref()
                .unwrap()
                .cached_tokens,
            Some(7)
        );
        assert_eq!(
            normal
                .usage
                .prompt_tokens_details
                .as_ref()
                .unwrap()
                .cache_creation_tokens,
            Some(2)
        );
        assert_eq!(
            normal
                .usage
                .completion_tokens_details
                .as_ref()
                .unwrap()
                .reasoning_tokens,
            Some(3)
        );
        let frames: Vec<_> = provider
            .stream_complete(request)
            .await
            .unwrap()
            .collect()
            .await;
        let usage = frames
            .iter()
            .filter_map(|frame| frame.as_ref().ok()?.usage.as_ref())
            .next_back()
            .unwrap();
        assert_eq!(
            usage.prompt_tokens_details.as_ref().unwrap().cached_tokens,
            Some(7)
        );
        assert_eq!(
            usage
                .completion_tokens_details
                .as_ref()
                .unwrap()
                .reasoning_tokens,
            Some(3)
        );
        for request in server.received_requests().await.unwrap() {
            let value: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(value.get("extensions").is_none());
        }
        server.verify().await;
    }
}
