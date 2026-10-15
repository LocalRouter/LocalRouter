//! Anthropic Messages wire contracts. Native payloads remain separate from
//! the policy projection: a text projection cannot reconstruct signed history.

use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

/// A Messages request retains all native block fields and their order.
/// Validation is deliberately separate from deserialization so ingress can
/// report unsupported actions before selecting or contacting a provider.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessagesRequest {
    pub model: String,
    /// Zero warms the provider prompt cache without generating output.
    pub max_tokens: u32,
    pub messages: Vec<MessagesInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<Value>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, Value>>,
    #[serde(flatten)]
    pub options: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessagesInput {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessagesResponse {
    pub id: String,
    pub r#type: String,
    pub role: String,
    pub model: String,
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: MessagesUsage,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessagesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<MessagesCacheCreation>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessagesCacheCreation {
    pub ephemeral_5m_input_tokens: u32,
    pub ephemeral_1h_input_tokens: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CountMessageTokensResponse {
    pub input_tokens: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CountMessageTokensRequest {
    pub model: String,
    pub messages: Vec<MessagesInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
}

impl CountMessageTokensRequest {
    pub fn into_messages(self) -> MessagesRequest {
        let mut options = Map::new();
        for (name, value) in [
            ("tools", self.tools),
            ("tool_choice", self.tool_choice),
            ("thinking", self.thinking),
        ] {
            if let Some(value) = value {
                options.insert(name.into(), value);
            }
        }
        MessagesRequest {
            model: self.model,
            max_tokens: 0,
            messages: self.messages,
            system: self.system,
            stream: false,
            extensions: None,
            options,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesOperation {
    Create,
    CountTokens,
}

#[derive(Debug, Clone)]
pub struct MessagesContext {
    pub request: MessagesRequest,
    pub headers: MessagesHeaders,
}

/// Private envelope carried on provider responses, never accepted from clients.
pub const RESPONSE: &str = "localrouter.native_messages_response";
pub const EVENT: &str = "localrouter.native_messages_event";

pub fn validate_dispatch(
    provider: &dyn crate::ModelProvider,
    request: &crate::CompletionRequest,
) -> AppResult<()> {
    let Some(context) = request.native_messages.as_ref() else {
        return Ok(());
    };
    if provider.supports_native_messages() {
        context.prepared(request).map(|_| ())
    } else {
        context.translated(request).map(|_| ())
    }
}

impl MessagesContext {
    pub fn prepared(&self, request: &crate::CompletionRequest) -> AppResult<MessagesRequest> {
        let projection = self.request.policy_projection();
        let expected: Vec<crate::ChatMessage> =
            serde_json::from_value(projection["messages"].clone())
                .map_err(|e| invalid(format!("Invalid Messages policy projection: {e}")))?;
        if serde_json::to_value(expected).ok() != serde_json::to_value(&request.messages).ok() {
            return Err(invalid(
                "Native Messages history cannot be rewritten through the chat policy projection",
            ));
        }
        let expected_tools: Option<Vec<crate::Tool>> = projection
            .get("tools")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| invalid(format!("Invalid native tool projection: {e}")))?;
        if serde_json::to_value(expected_tools).ok() != serde_json::to_value(&request.tools).ok() {
            return Err(invalid(
                "Native tool definitions cannot be rewritten through the chat policy projection",
            ));
        }
        let mut native = self.request.clone();
        native.model = request.model.clone();
        native.max_tokens = request.max_tokens.unwrap_or(native.max_tokens);
        native.stream = request.stream;
        for (name, value) in [
            ("temperature", request.temperature.map(|v| json!(v))),
            ("top_p", request.top_p.map(|v| json!(v))),
            ("top_k", request.top_k.map(|v| json!(v))),
        ] {
            let original = projection.get(name);
            let unchanged = match name {
                "temperature" | "top_p" => {
                    original.and_then(Value::as_f64).map(|v| v as f32)
                        == value.as_ref().and_then(Value::as_f64).map(|v| v as f32)
                }
                _ => original == value.as_ref(),
            };
            if unchanged
                || (name == "top_k"
                    && value.is_none()
                    && native.options.get(name).and_then(Value::as_u64) == Some(0))
            {
                continue;
            }
            if let Some(value) = value {
                native.options.insert(name.into(), value);
            } else {
                native.options.remove(name);
            }
        }
        native.validate()?;
        Ok(native)
    }

    pub(crate) fn translated(
        &self,
        request: &crate::CompletionRequest,
    ) -> AppResult<crate::CompletionRequest> {
        if self.headers.beta.is_some()
            || self
                .headers
                .version
                .as_deref()
                .is_some_and(|v| v != "2023-06-01")
        {
            return Err(invalid(
                "Native API version/beta semantics require native Messages transport",
            ));
        }
        let mut translated = self.prepared(request)?.translated_chat()?;
        translated.extensions = request.extensions.clone();
        translated.pre_computed_routing = request.pre_computed_routing.clone();
        Ok(translated)
    }
}

/// Router transport boundary. Ordinary Chat calls keep their existing path;
/// native calls retain their envelope instead of rebuilding it from messages.
/// Document provenance is local metadata, never an upstream request parameter.
pub async fn dispatch_complete(
    provider: &dyn crate::ModelProvider,
    request: crate::CompletionRequest,
) -> AppResult<crate::CompletionResponse> {
    let documents = request.documents.clone();
    let mut response = dispatch_complete_inner(provider, request).await?;
    if let Some(context) = documents {
        let mut metadata = context.metadata();
        metadata["input_usage_reported"] = serde_json::json!(true);
        response
            .extensions
            .get_or_insert_with(Default::default)
            .insert(crate::documents::METADATA_EXTENSION.into(), metadata);
    }
    Ok(response)
}

async fn dispatch_complete_inner(
    provider: &dyn crate::ModelProvider,
    request: crate::CompletionRequest,
) -> AppResult<crate::CompletionResponse> {
    let Some(context) = request.native_messages.as_ref() else {
        return provider.complete(request).await;
    };
    if !provider.supports_native_messages() {
        return provider.complete(context.translated(&request)?).await;
    }
    let native = context.prepared(&request)?;
    let response = provider
        .native_messages(&native, &context.headers, MessagesOperation::Create)
        .await?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let body = read_json_response(response, 32 * 1024 * 1024).await?;
    completion_from_native(body, provider.name()).map_err(|error| match error {
        AppError::InvalidParams(message) => AppError::Provider(message),
        other => other,
    })
}

async fn response_bytes(response: reqwest::Response, limit: usize) -> AppResult<Vec<u8>> {
    use futures::StreamExt;
    let mut chunks = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| {
            AppError::Provider(format!("Native Messages response read failed: {e}"))
        })?;
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(AppError::Provider(format!(
                "Native Messages response exceeds {limit} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn read_json_response(response: reqwest::Response, limit: usize) -> AppResult<Value> {
    serde_json::from_slice(&response_bytes(response, limit).await?)
        .map_err(|e| AppError::Provider(format!("Invalid native Messages JSON: {e}")))
}

pub async fn response_error(response: reqwest::Response) -> AppError {
    let status = response.status();
    let retry = crate::http_client::retry_after(response.headers(), std::time::SystemTime::now());
    let body = response_bytes(response, 64 * 1024)
        .await
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_else(|error| error.to_string());
    crate::http_client::with_retry_after(
        crate::http_client::classify_openai_error(status, &body),
        retry,
    )
}

fn count(value: &Value, name: &str) -> AppResult<u32> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| AppError::Provider(format!("Native Messages usage requires valid {name}")))
}

pub(crate) fn native_usage(value: &Value) -> AppResult<crate::TokenUsage> {
    let input = count(value, "input_tokens")?;
    let output = count(value, "output_tokens")?;
    let optional_count = |key: &str| -> AppResult<Option<u32>> {
        if value.get(key).is_none_or(Value::is_null) {
            Ok(None)
        } else {
            count(value, key).map(Some)
        }
    };
    let read = optional_count("cache_read_input_tokens")?;
    let write = optional_count("cache_creation_input_tokens")?;
    let mut usage = crate::anthropic::token_usage(input, output, write, read);
    if let Some(creation) = value.get("cache_creation").filter(|v| !v.is_null()) {
        let one_hour = count(creation, "ephemeral_1h_input_tokens")?;
        let five_minute = count(creation, "ephemeral_5m_input_tokens")?;
        if one_hour.checked_add(five_minute) != write {
            return Err(AppError::Provider(
                "Native cache TTL counts do not match cache creation usage".into(),
            ));
        }
        if let Some(details) = usage.prompt_tokens_details.as_mut() {
            details.anthropic_cache_creation_1h_tokens = Some(one_hour);
        }
    }
    Ok(usage)
}

pub fn completion_from_native(body: Value, provider: &str) -> AppResult<crate::CompletionResponse> {
    if body.get("type").and_then(Value::as_str) != Some("message")
        || body.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Err(AppError::Provider(
            "Invalid native Messages response envelope".into(),
        ));
    }
    let id = required_string(&body, "id")?.to_owned();
    let model = required_string(&body, "model")?.to_owned();
    let content = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::Provider("Native Messages response requires content blocks".into())
        })?;
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tools = Vec::new();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AppError::Provider("Invalid native text output".into()))?,
            ),
            Some("thinking") => thinking.push_str(
                block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            Some("tool_use") => tools.push(crate::ToolCall {
                id: required_string(block, "id")?.into(),
                tool_type: "function".into(),
                function: crate::FunctionCall {
                    name: required_string(block, "name")?.into(),
                    arguments: block
                        .get("input")
                        .ok_or_else(|| {
                            AppError::Provider("Native tool output requires input".into())
                        })?
                        .to_string(),
                },
            }),
            _ => (), // Native envelope retains opaque/future response blocks intact.
        }
    }
    let finish = match body.get("stop_reason").and_then(Value::as_str) {
        Some("end_turn" | "stop_sequence") => "stop",
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        Some(other) => other,
        None => {
            return Err(AppError::Provider(
                "Incomplete native Messages response".into(),
            ))
        }
    }
    .to_owned();
    Ok(crate::CompletionResponse {
        id,
        object: "chat.completion".into(),
        created: chrono::Utc::now().timestamp(),
        model,
        provider: provider.into(),
        choices: vec![crate::CompletionChoice {
            index: 0,
            message: crate::ChatMessage {
                annotations: None,
                role: "assistant".into(),
                content: crate::ChatMessageContent::Text(text),
                tool_calls: (!tools.is_empty()).then_some(tools),
                tool_call_id: None,
                name: None,
                reasoning_content: (!thinking.is_empty()).then_some(thinking),
            },
            finish_reason: Some(finish),
            logprobs: None,
        }],
        usage: native_usage(&body["usage"])?,
        system_fingerprint: None,
        service_tier: None,
        extensions: Some([(RESPONSE.to_owned(), body)].into_iter().collect()),
        routellm_win_rate: None,
        request_usage_entries: None,
    })
}

impl MessagesOperation {
    pub fn path(self) -> &'static str {
        match self {
            Self::Create => "messages",
            Self::CountTokens => "messages/count_tokens",
        }
    }

    pub fn body(self, request: &MessagesRequest) -> AppResult<Value> {
        request.validate()?;
        let mut body = serde_json::to_value(request)
            .map_err(|e| invalid(format!("Invalid Messages request: {e}")))?;
        body.as_object_mut()
            .expect("serialized MessagesRequest is an object")
            .remove("extensions");
        if self == Self::CountTokens {
            // Token counting has a smaller contract than generation. Do not
            // silently discard a caller option and pretend it was counted.
            if request.stream
                || request
                    .options
                    .keys()
                    .any(|key| !matches!(key.as_str(), "thinking" | "tools" | "tool_choice"))
            {
                return Err(invalid("Token counting supports messages, system, tools, tool_choice and thinking only"));
            }
            let object = body
                .as_object_mut()
                .expect("serialized MessagesRequest is an object");
            object.remove("max_tokens");
            object.remove("stream");
        }
        Ok(body)
    }
}

/// Semantic headers only. SDK diagnostics, caller credentials and provider
/// workspace selection never become upstream transport headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessagesHeaders {
    pub version: Option<String>,
    pub beta: Option<String>,
}

impl MessagesHeaders {
    pub fn from_headers(headers: &http::HeaderMap) -> AppResult<Self> {
        let read = |name: &str| -> AppResult<Option<String>> {
            let values = headers.get_all(name);
            if values.iter().count() > 1 {
                return Err(invalid(format!("Repeated {name} header")));
            }
            values
                .iter()
                .next()
                .map(|value| {
                    let value = value
                        .to_str()
                        .map_err(|_| invalid(format!("Invalid {name} header")))?;
                    if value.is_empty() || value.len() > 1024 || !value.is_ascii() {
                        return Err(invalid(format!("Invalid {name} header")));
                    }
                    Ok(value.to_owned())
                })
                .transpose()
        };
        Ok(Self {
            version: read("anthropic-version")?,
            beta: read("anthropic-beta")?,
        })
    }

    pub fn apply(
        &self,
        mut request: reqwest_middleware::RequestBuilder,
    ) -> reqwest_middleware::RequestBuilder {
        request = request.header(
            "anthropic-version",
            self.version.as_deref().unwrap_or("2023-06-01"),
        );
        if let Some(beta) = &self.beta {
            request = request.header("anthropic-beta", beta);
        }
        request
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidParams(message.into())
}

fn required_string<'a>(object: &'a Value, key: &str) -> AppResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(format!("Messages block requires nonempty {key}")))
}

fn validate_cache_control(value: &Value) -> AppResult<()> {
    let Some(cache) = value.get("cache_control").filter(|v| !v.is_null()) else {
        return Ok(());
    };
    let object = cache
        .as_object()
        .ok_or_else(|| invalid("cache_control must be an object"))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "type" | "ttl"))
        || cache.get("type").and_then(Value::as_str) != Some("ephemeral")
        || cache
            .get("ttl")
            .is_some_and(|ttl| !matches!(ttl.as_str(), Some("5m" | "1h")))
    {
        return Err(invalid("Unsupported Messages cache_control"));
    }
    Ok(())
}

fn validate_source(block: &Value) -> AppResult<()> {
    let source = block
        .get("source")
        .ok_or_else(|| invalid("Messages image/document requires source"))?;
    match required_string(source, "type")? {
        "base64" => {
            required_string(source, "media_type")?;
            required_string(source, "data")?;
        }
        "url" => {
            let url = reqwest::Url::parse(required_string(source, "url")?)
                .map_err(|_| invalid("Invalid Messages source URL"))?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.host_str().is_none()
            {
                return Err(invalid(
                    "Messages sources require an HTTP(S) URL without credentials",
                ));
            }
        }
        "text" if block.get("type").and_then(Value::as_str) == Some("document") => {
            required_string(source, "data")?;
        }
        _ => {
            return Err(invalid(
                "Unsupported Messages source; file references require ownership support",
            ))
        }
    }
    Ok(())
}

fn validate_content(content: &Value, role: &str, nested: bool) -> AppResult<()> {
    if content.is_string() {
        return Ok(());
    }
    let blocks = content
        .as_array()
        .ok_or_else(|| invalid("Messages content must be text or an array of blocks"))?;
    for block in blocks {
        validate_cache_control(block)?;
        match required_string(block, "type")? {
            "text" => {
                if block.get("text").and_then(Value::as_str).is_none() {
                    return Err(invalid("Messages text block requires text"));
                }
            }
            "image" | "document" if role == "user" => validate_source(block)?,
            "thinking" if role == "assistant" && !nested => {
                required_string(block, "signature")?;
                if block.get("thinking").and_then(Value::as_str).is_none() {
                    return Err(invalid("Messages thinking block requires thinking"));
                }
            }
            "redacted_thinking" if role == "assistant" && !nested => {
                required_string(block, "data")?;
            }
            "tool_use" if role == "assistant" && !nested => {
                required_string(block, "id")?;
                required_string(block, "name")?;
                if !block.get("input").is_some_and(Value::is_object) {
                    return Err(invalid("Messages tool_use requires object input"));
                }
            }
            "tool_result" if role == "user" && !nested => {
                required_string(block, "tool_use_id")?;
                if let Some(content) = block.get("content") {
                    validate_content(content, "user", true)?;
                }
                if block.get("is_error").is_some_and(|v| !v.is_boolean()) {
                    return Err(invalid("Messages tool_result is_error must be boolean"));
                }
            }
            _ => return Err(invalid("Unsupported Messages content block or role; server actions require explicit support")),
        }
    }
    Ok(())
}

impl MessagesRequest {
    pub fn validate(&self) -> AppResult<()> {
        if self.extensions.as_ref().is_some_and(|extensions| {
            extensions.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    crate::gateway::EXTENSION
                        | crate::session::EXTENSION
                        | crate::response_cache::EXTENSION
                )
            })
        }) {
            return Err(invalid("Unsupported native Messages extension"));
        }
        if self.model.trim().is_empty() || self.model.len() > 1024 {
            return Err(invalid(
                "Messages model is required and must be at most 1024 bytes",
            ));
        }
        if self.messages.is_empty() || self.messages.len() > 100_000 {
            return Err(invalid("Messages requires 1–100000 messages"));
        }
        if let Some(system) = &self.system {
            validate_content(system, "system", false)?;
        }
        for message in &self.messages {
            if !matches!(message.role.as_str(), "user" | "assistant") {
                return Err(invalid(
                    "Messages roles must be user or assistant; use top-level system",
                ));
            }
            validate_content(&message.content, &message.role, false)?;
        }
        for (name, value) in &self.options {
            match name.as_str() {
                "temperature" | "top_p" => {
                    if !value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v)) {
                        return Err(invalid(format!("Messages {name} must be between 0 and 1")));
                    }
                }
                "top_k" => {
                    if !value.as_u64().is_some_and(|v| v <= u32::MAX as u64) {
                        return Err(invalid("Messages top_k must be an unsigned 32-bit integer"));
                    }
                }
                "stop_sequences" => {
                    if !value
                        .as_array()
                        .is_some_and(|values| values.iter().all(Value::is_string))
                    {
                        return Err(invalid("Messages stop_sequences must be a string array"));
                    }
                }
                "tools" => {
                    let tools = value
                        .as_array()
                        .ok_or_else(|| invalid("Messages tools must be an array"))?;
                    for tool in tools {
                        if tool
                            .get("type")
                            .is_some_and(|v| v.as_str() != Some("custom"))
                            || tool
                                .get("defer_loading")
                                .is_some_and(|v| v != &Value::Bool(false))
                        {
                            return Err(invalid("Messages server tools and deferred tool search require explicit support"));
                        }
                        required_string(tool, "name")?;
                        if !tool.get("input_schema").is_some_and(Value::is_object) {
                            return Err(invalid("Messages client tools require input_schema"));
                        }
                        validate_cache_control(tool)?;
                    }
                }
                "tool_choice" => match required_string(value, "type")? {
                    "auto" | "any" | "none" => (),
                    "tool" => {
                        required_string(value, "name")?;
                    }
                    _ => return Err(invalid("Unsupported Messages tool_choice")),
                },
                "metadata" | "thinking" | "output_config" => {
                    if !value.is_object() {
                        return Err(invalid(format!("Messages {name} must be an object")));
                    }
                }
                "service_tier" => {
                    if !matches!(value.as_str(), Some("auto" | "standard_only")) {
                        return Err(invalid("Unsupported Messages service_tier"));
                    }
                }
                "cache_control" => validate_cache_control(&json!({"cache_control":value}))?,
                _ => return Err(invalid(format!("Unsupported Messages field: {name}"))),
            }
        }
        Ok(())
    }

    /// A conservative binding marker. Native tools can return provider-specific
    /// identifiers even when their visible arguments are ordinary JSON.
    pub fn has_bound_state(&self) -> bool {
        self.messages.iter().any(|message| {
            message.content.as_array().is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("thinking" | "redacted_thinking" | "tool_use" | "tool_result")
                    )
                })
            })
        })
    }

    /// Translate only the subset whose semantics survive the chat wire format.
    /// Native markers, signed state and provider-side actions never disappear
    /// as a side effect of choosing a different transport.
    pub fn translated_chat(&self) -> AppResult<crate::CompletionRequest> {
        self.validate()?;
        if self.max_tokens == 0 {
            return Err(invalid(
                "Zero-token prompt-cache warming requires native Messages",
            ));
        }
        let mut result = crate::CompletionRequest::new(&self.model, Vec::new());
        result.extensions = self.extensions.clone();
        result.max_tokens = Some(self.max_tokens);
        result.stream = self.stream;
        if let Some(system) = &self.system {
            result.messages.push(crate::ChatMessage {
                annotations: None,
                role: "system".into(),
                content: translated_content(system)?,
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            });
        }
        let mut pending = std::collections::HashSet::new();
        for message in &self.messages {
            translate_message(message, &mut result.messages, &mut pending)?;
        }
        if !pending.is_empty() {
            return Err(invalid(
                "Translated tool history requires a result for every pending tool call",
            ));
        }
        for (name, value) in &self.options {
            match name.as_str() {
                "temperature" => result.temperature = value.as_f64().map(|n| n as f32),
                "top_p" => result.top_p = value.as_f64().map(|n| n as f32),
                "top_k" => {
                    let n = value.as_u64().expect("validated top_k") as u32;
                    if n == 0 {
                        return Err(invalid("top_k=0 requires native Messages"));
                    }
                    result.top_k = Some(n);
                }
                "stop_sequences" => {
                    result.stop = Some(
                        serde_json::from_value(value.clone())
                            .map_err(|e| invalid(e.to_string()))?,
                    )
                }
                "tools" => {
                    let mut tools = Vec::new();
                    for tool in value.as_array().expect("validated tools") {
                        if tool.as_object().expect("validated tool").keys().any(|key| {
                            !matches!(
                                key.as_str(),
                                "name" | "description" | "input_schema" | "type"
                            )
                        }) {
                            return Err(invalid("Native tool options require native Messages"));
                        }
                        if tool.get("description").is_some_and(|v| !v.is_string()) {
                            return Err(invalid("Tool description must be text"));
                        }
                        tools.push(crate::Tool {
                            tool_type: "function".into(),
                            function: crate::FunctionDefinition {
                                name: required_string(tool, "name")?.into(),
                                description: tool
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                                parameters: tool["input_schema"].clone(),
                                strict: None,
                            },
                        });
                    }
                    result.tools = Some(tools);
                }
                "tool_choice" => {
                    if value
                        .as_object()
                        .expect("validated tool_choice")
                        .keys()
                        .any(|key| {
                            !matches!(key.as_str(), "type" | "name" | "disable_parallel_tool_use")
                        })
                    {
                        return Err(invalid(
                            "Native tool choice options require native Messages",
                        ));
                    }
                    result.tool_choice = Some(match required_string(value, "type")? {
                        "auto" => crate::ToolChoice::Auto("auto".into()),
                        "any" => crate::ToolChoice::Auto("required".into()),
                        "none" => crate::ToolChoice::Auto("none".into()),
                        "tool" => crate::ToolChoice::Specific {
                            tool_type: "function".into(),
                            function: crate::FunctionName {
                                name: required_string(value, "name")?.into(),
                            },
                        },
                        _ => unreachable!("validated tool choice"),
                    });
                    if let Some(disabled) = value.get("disable_parallel_tool_use") {
                        result.parallel_tool_calls =
                            Some(!disabled.as_bool().ok_or_else(|| {
                                invalid("disable_parallel_tool_use must be boolean")
                            })?);
                    }
                }
                _ => {
                    return Err(invalid(format!(
                        "Messages {name} requires native transport"
                    )))
                }
            }
        }
        Ok(result)
    }

    /// The complete native content is included for policy scanning. This is
    /// deliberately not a transport conversion; signatures and block order
    /// stay in the original request. Treat documents/images as multimodal.
    pub fn policy_projection(&self) -> Value {
        let mut messages = Vec::new();
        if let Some(system) = &self.system {
            messages.push(json!({"role":"system","content":projection_text(system)}));
        }
        if !self.options.is_empty() {
            // Tool descriptions, schemas and native configuration can contain
            // secrets or instructions too. Keep them in the scan input even
            // though only the original native envelope is dispatched.
            messages.push(
                json!({"role":"system","content":Value::Object(self.options.clone()).to_string()}),
            );
        }
        for message in &self.messages {
            messages
                .push(json!({"role":message.role,"content":projected_content(&message.content)}));
        }
        let mut projection = json!({"model":self.model,"messages":messages,"max_tokens":self.max_tokens,"stream":self.stream});
        if let Some(extensions) = &self.extensions {
            projection["extensions"] = json!(extensions);
        }
        for name in ["temperature", "top_p", "top_k"] {
            if let Some(value) = self.options.get(name) {
                if name == "top_k" && value.as_u64() == Some(0) {
                    continue;
                }
                projection[name] = value.clone();
            }
        }
        if let Some(tools) = self.options.get("tools").and_then(Value::as_array) {
            projection["tools"]=json!(tools.iter().map(|tool|json!({"type":"function","function":{"name":tool["name"],"description":tool.get("description"),"parameters":tool["input_schema"]}})).collect::<Vec<_>>());
        }
        projection
    }
}

fn projection_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn projected_content(value: &Value) -> Value {
    let mut images = Vec::new();
    fn collect(value: &Value, images: &mut Vec<Value>) {
        if let Some(blocks) = value.as_array() {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("image") {
                    let source = &block["source"];
                    let url = match source["type"].as_str() {
                        Some("url") => source["url"].as_str().map(str::to_owned),
                        Some("base64") => source["media_type"]
                            .as_str()
                            .zip(source["data"].as_str())
                            .map(|(mime, data)| format!("data:{mime};base64,{data}")),
                        _ => None,
                    };
                    if let Some(url) = url {
                        images.push(json!({"type":"image_url","image_url":{"url":url}}));
                    }
                }
                if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                    collect(&block["content"], images);
                }
            }
        }
    }
    collect(value, &mut images);
    let text = projection_text(value);
    if images.is_empty() {
        json!(text)
    } else {
        let mut parts = vec![json!({"type":"text","text":text})];
        parts.extend(images);
        json!(parts)
    }
}

fn translated_content(value: &Value) -> AppResult<crate::ChatMessageContent> {
    if let Some(text) = value.as_str() {
        return Ok(crate::ChatMessageContent::Text(text.into()));
    }
    let mut parts = Vec::new();
    for block in value
        .as_array()
        .ok_or_else(|| invalid("Invalid Messages content"))?
    {
        let object = block
            .as_object()
            .ok_or_else(|| invalid("Invalid Messages block"))?;
        match required_string(block, "type")? {
            "text" => {
                if object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "type" | "text"))
                {
                    return Err(invalid("Native text markers require native Messages"));
                }
                parts.push(crate::ContentPart::Text {
                    text: block["text"].as_str().expect("validated text").into(),
                });
            }
            "image" => {
                if object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "type" | "source"))
                {
                    return Err(invalid("Native image markers require native Messages"));
                }
                let source = &block["source"];
                let url = match required_string(source, "type")? {
                    "url" => {
                        if source
                            .as_object()
                            .expect("validated source")
                            .keys()
                            .any(|key| !matches!(key.as_str(), "type" | "url"))
                        {
                            return Err(invalid(
                                "Native image source options require native Messages",
                            ));
                        }
                        required_string(source, "url")?.to_owned()
                    }
                    "base64" => {
                        if source
                            .as_object()
                            .expect("validated source")
                            .keys()
                            .any(|key| !matches!(key.as_str(), "type" | "media_type" | "data"))
                        {
                            return Err(invalid(
                                "Native image source options require native Messages",
                            ));
                        }
                        format!(
                            "data:{};base64,{}",
                            required_string(source, "media_type")?,
                            required_string(source, "data")?
                        )
                    }
                    _ => return Err(invalid("Unsupported translated image source")),
                };
                parts.push(crate::ContentPart::ImageUrl {
                    image_url: crate::ImageUrl { url, detail: None },
                });
            }
            _ => {
                return Err(invalid(
                    "Signed history, documents and native tool history require native Messages",
                ))
            }
        }
    }
    if parts.is_empty() {
        Ok(crate::ChatMessageContent::Text(String::new()))
    } else {
        Ok(crate::ChatMessageContent::Parts(parts))
    }
}

fn translate_message(
    message: &MessagesInput,
    out: &mut Vec<crate::ChatMessage>,
    pending: &mut std::collections::HashSet<String>,
) -> AppResult<()> {
    let plain = |role: String, content: crate::ChatMessageContent| crate::ChatMessage {
        annotations: None,
        role,
        content,
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    };
    let Some(blocks) = message.content.as_array().filter(|blocks| {
        blocks
            .iter()
            .any(|block| matches!(block["type"].as_str(), Some("tool_use" | "tool_result")))
    }) else {
        if !pending.is_empty() {
            return Err(invalid(
                "Tool results must precede the next translated conversation turn",
            ));
        }
        out.push(plain(
            message.role.clone(),
            translated_content(&message.content)?,
        ));
        return Ok(());
    };
    if message.role == "assistant" {
        if !pending.is_empty() {
            return Err(invalid("Tool results must precede the next assistant turn"));
        }
        let mut text = Vec::new();
        let mut tools = Vec::new();
        for block in blocks {
            if block["type"] == "tool_use" {
                if block
                    .as_object()
                    .expect("validated block")
                    .keys()
                    .any(|key| !matches!(key.as_str(), "type" | "id" | "name" | "input"))
                {
                    return Err(invalid(
                        "Native tool history markers require native Messages",
                    ));
                }
                let id = required_string(block, "id")?.to_owned();
                if !pending.insert(id.clone()) {
                    return Err(invalid("Repeated pending tool-use id"));
                }
                tools.push(crate::ToolCall {
                    id,
                    tool_type: "function".into(),
                    function: crate::FunctionCall {
                        name: required_string(block, "name")?.into(),
                        arguments: block["input"].to_string(),
                    },
                });
            } else {
                if !tools.is_empty() {
                    return Err(invalid(
                        "Text interleaved after native tool calls requires native Messages",
                    ));
                }
                text.push(block.clone());
            }
        }
        let mut translated = plain("assistant".into(), translated_content(&json!(text))?);
        translated.tool_calls = Some(tools);
        out.push(translated);
    } else {
        let mut remainder = Vec::new();
        for block in blocks {
            if block["type"] == "tool_result" {
                if !remainder.is_empty() {
                    return Err(invalid(
                        "Tool results after ordinary user content require native Messages",
                    ));
                }
                if block
                    .as_object()
                    .expect("validated block")
                    .keys()
                    .any(|key| {
                        !matches!(
                            key.as_str(),
                            "type" | "tool_use_id" | "content" | "is_error"
                        )
                    })
                    || block.get("is_error") == Some(&Value::Bool(true))
                {
                    return Err(invalid(
                        "Tool errors and native tool-result markers require native Messages",
                    ));
                }
                let id = required_string(block, "tool_use_id")?.to_owned();
                if !pending.remove(&id) {
                    return Err(invalid(
                        "Tool result does not match a pending call in this request",
                    ));
                }
                let content = match block.get("content") {
                    None => String::new(),
                    Some(Value::String(value)) => value.clone(),
                    Some(Value::Array(values)) if values.is_empty() => String::new(),
                    Some(Value::Array(values))
                        if values.len() == 1
                            && values[0]["type"] == "text"
                            && values[0].as_object().is_some_and(|v| {
                                v.keys().all(|key| matches!(key.as_str(), "type" | "text"))
                            }) =>
                    {
                        values[0]["text"]
                            .as_str()
                            .expect("validated text")
                            .to_owned()
                    }
                    _ => {
                        return Err(invalid(
                            "Multiblock or multimodal tool results require native Messages",
                        ))
                    }
                };
                let mut translated = plain("tool".into(), crate::ChatMessageContent::Text(content));
                translated.tool_call_id = Some(id);
                out.push(translated);
            } else {
                remainder.push(block.clone());
            }
        }
        if !pending.is_empty() {
            return Err(invalid("Tool result turn is missing a pending call"));
        }
        if !remainder.is_empty() {
            out.push(plain("user".into(), translated_content(&json!(remainder))?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> MessagesRequest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn signed_history_and_cache_markers_survive_validation() {
        let value = json!({"model":"anthropic/claude-fixture","max_tokens":0,"stream":false,
            "system":[{"type":"text","text":"rules","cache_control":{"type":"ephemeral","ttl":"1h"}}],
            "messages":[{"role":"assistant","content":[
                {"type":"thinking","thinking":"unchanged\ntext","signature":"signed+/="},
                {"type":"redacted_thinking","data":"opaque+/="},
                {"type":"tool_use","id":"toolu_fixture","name":"weather","input":{"city":"Toronto"}}
            ]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_fixture","content":"sunny"}]}]});
        let request = parse(value.clone());
        request.validate().unwrap();
        assert!(request.has_bound_state());
        assert_eq!(serde_json::to_value(&request).unwrap(), value);
        assert!(request.policy_projection().to_string().contains("Toronto"));
    }

    #[test]
    fn side_effects_and_foreign_file_references_fail_before_dispatch() {
        for block in [
            json!({"type":"server_tool_use","id":"srvtoolu_x","name":"bash","input":{}}),
            json!({"type":"image","source":{"type":"file","file_id":"file_other_client"}}),
            json!({"type":"document","source":{"type":"url","url":"file:///etc/passwd"}}),
            json!({"type":"tool_result","tool_use_id":"t","content":[{"type":"tool_result","tool_use_id":"nested"}]}),
        ] {
            assert!(parse(json!({"model":"fixture","max_tokens":1,"messages":[{"role":"user","content":[block]}]})).validate().is_err());
        }
        for extra in [
            json!({"mcp_servers":[]}),
            json!({"container":"other_client"}),
            json!({"tools":[{"type":"web_search_20250305","name":"web_search"}]}),
            json!({"thinking":"enabled"}),
        ] {
            let mut value = json!({"model":"fixture","max_tokens":1,"messages":[{"role":"user","content":"hi"}]});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(parse(value).validate().is_err());
        }
    }

    #[test]
    fn malformed_content_and_cache_semantics_are_not_silently_dropped() {
        for content in [
            Value::Null,
            json!({"text":"hi"}),
            json!([{"type":"text"}]),
            json!([{"type":"thinking","thinking":"x"}]),
            json!([{"type":"text","text":"hi","cache_control":{"type":"ephemeral","ttl":"1d"}}]),
        ] {
            assert!(parse(json!({"model":"fixture","max_tokens":1,"messages":[{"role":"user","content":content}]})).validate().is_err());
        }
        assert!(parse(
            json!({"model":"fixture","max_tokens":1,"messages":[{"role":"system","content":"hi"}]})
        )
        .validate()
        .is_err());
    }

    #[test]
    fn semantic_headers_exclude_credentials_and_fingerprints() {
        let mut headers = http::HeaderMap::new();
        headers.insert("x-api-key", "local-secret".parse().unwrap());
        headers.insert("authorization", "Bearer other".parse().unwrap());
        headers.insert("x-stainless-os", "secret-os".parse().unwrap());
        headers.insert(
            "anthropic-workspace-id",
            "foreign-workspace".parse().unwrap(),
        );
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
        headers.insert(
            "anthropic-beta",
            "prompt-caching-2024-07-31".parse().unwrap(),
        );
        let result = MessagesHeaders::from_headers(&headers).unwrap();
        assert_eq!(result.version.as_deref(), Some("2023-06-01"));
        assert_eq!(result.beta.as_deref(), Some("prompt-caching-2024-07-31"));
        headers.append("anthropic-beta", "other".parse().unwrap());
        assert!(MessagesHeaders::from_headers(&headers).is_err());
    }

    #[test]
    fn counting_has_its_own_wire_contract() {
        let request = parse(
            json!({"model":"fixture","max_tokens":0,"messages":[{"role":"user","content":"hi"}],"tools":[{"name":"client_tool","input_schema":{"type":"object"}}]}),
        );
        let body = MessagesOperation::CountTokens.body(&request).unwrap();
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
        assert!(body.get("tools").is_some());
        assert!(request.policy_projection()["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("client_tool"));
        let mut streaming = request.clone();
        streaming.stream = true;
        assert!(MessagesOperation::CountTokens.body(&streaming).is_err());
        let mut temperature = request;
        temperature.options.insert("temperature".into(), json!(0.3));
        assert!(MessagesOperation::CountTokens.body(&temperature).is_err());
    }

    #[test]
    fn translation_preserves_plain_image_and_client_tool_options() {
        let request = parse(
            json!({"model":"fixture","max_tokens":12,"system":"rules","temperature":0.3,
            "messages":[{"role":"user","content":[{"type":"text","text":"describe"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}]}],
            "tools":[{"name":"lookup","description":"search local index","input_schema":{"type":"object"}}],
            "tool_choice":{"type":"tool","name":"lookup","disable_parallel_tool_use":true},"stop_sequences":["STOP"]}),
        );
        let translated = request.translated_chat().unwrap();
        assert_eq!(translated.max_tokens, Some(12));
        assert_eq!(translated.parallel_tool_calls, Some(false));
        assert_eq!(translated.messages[0].content.as_text(), "rules");
        let serialized = serde_json::to_value(translated).unwrap();
        assert_eq!(
            serialized["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,aGVsbG8="
        );
        assert_eq!(serialized["tool_choice"]["function"]["name"], "lookup");
        assert_eq!(
            serialized["tools"][0]["function"]["parameters"]["type"],
            "object"
        );
    }

    #[test]
    fn translation_rejects_semantic_loss() {
        for extra in [
            json!({"cache_control":{"type":"ephemeral","ttl":"1h"}}),
            json!({"thinking":{"type":"enabled","budget_tokens":1024}}),
            json!({"output_config":{"effort":"high"}}),
            json!({"metadata":{"user_id":"x"}}),
            json!({"top_k":0}),
        ] {
            let mut value = json!({"model":"fixture","max_tokens":1,"messages":[{"role":"user","content":"hi"}]});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let request = parse(value);
            request.validate().unwrap();
            assert!(request.translated_chat().is_err());
        }
        for block in [
            json!({"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}),
            json!({"type":"thinking","thinking":"protected","signature":"signed"}),
            json!({"type":"tool_use","id":"toolu_x","name":"lookup","input":{}}),
        ] {
            let request = parse(
                json!({"model":"fixture","max_tokens":1,"messages":[{"role":"assistant","content":[block]}]}),
            );
            request.validate().unwrap();
            assert!(request.translated_chat().is_err());
        }
    }

    #[test]
    fn translation_preserves_ordered_client_tool_history_and_rejects_error_flag_loss() {
        let value = json!({"model":"fixture","max_tokens":20,"messages":[
            {"role":"user","content":"look up Toronto"},
            {"role":"assistant","content":[{"type":"text","text":"Checking."},{"type":"tool_use","id":"toolu_a","name":"lookup","input":{"city":"Toronto"}},{"type":"tool_use","id":"toolu_b","name":"time","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_a","content":[{"type":"text","text":"Sunny"}]},{"type":"tool_result","tool_use_id":"toolu_b","content":"12:00"},{"type":"text","text":"Summarize both."}]}
        ]});
        let translated = parse(value.clone()).translated_chat().unwrap();
        assert_eq!(translated.messages.len(), 5);
        assert_eq!(
            translated.messages[1].tool_calls.as_ref().unwrap()[0].id,
            "toolu_a"
        );
        assert_eq!(translated.messages[2].role, "tool");
        assert_eq!(
            translated.messages[2].tool_call_id.as_deref(),
            Some("toolu_a")
        );
        assert_eq!(translated.messages[2].content.as_text(), "Sunny");
        assert_eq!(
            translated.messages[3].tool_call_id.as_deref(),
            Some("toolu_b")
        );
        assert_eq!(translated.messages[4].content.as_text(), "Summarize both.");
        let mut bad = value.clone();
        bad["messages"][2]["content"][0]["is_error"] = json!(true);
        assert!(parse(bad).translated_chat().is_err());
        let mut bad = value.clone();
        bad["messages"][2]["content"][0]["tool_use_id"] = json!("foreign");
        assert!(parse(bad).translated_chat().is_err());
        let mut bad = value;
        bad["messages"][1]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"text","text":"after the tool"}));
        assert!(parse(bad).translated_chat().is_err());
    }

    #[test]
    fn native_ttl_usage_prices_one_hour_writes_separately() {
        let usage=native_usage(&json!({"input_tokens":10,"output_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":20,"cache_creation":{"ephemeral_5m_input_tokens":40,"ephemeral_1h_input_tokens":60}})).unwrap();
        let pricing = crate::PricingInfo {
            input_cost_per_1k: 3.0,
            output_cost_per_1k: 15.0,
            reasoning_cost_per_1k: None,
            cache_read_cost_per_1k: Some(0.3),
            cache_write_cost_per_1k: Some(3.75),
            currency: "USD".into(),
        };
        assert!((pricing.cost(&usage) - (0.03 + 0.03 + 0.15 + 0.36 + 0.006)).abs() < 1e-10);
        assert!(native_usage(&json!({"input_tokens":1,"output_tokens":0,"cache_creation_input_tokens":10,"cache_creation":{"ephemeral_5m_input_tokens":8,"ephemeral_1h_input_tokens":3}})).is_err());
    }

    #[test]
    fn native_policy_edits_cannot_leave_the_dispatched_body_unchanged() {
        let request = parse(
            json!({"model":"fixture","max_tokens":20,"temperature":0.3,"top_k":0,"messages":[{"role":"user","content":"original"}]}),
        );
        let mut projected: crate::CompletionRequest =
            serde_json::from_value(request.policy_projection()).unwrap();
        let context = MessagesContext {
            request,
            headers: MessagesHeaders::default(),
        };
        let prepared = context.prepared(&projected).unwrap();
        assert_eq!(prepared.options["temperature"], json!(0.3));
        assert_eq!(prepared.options["top_k"], json!(0));
        projected.max_tokens = Some(5);
        projected.temperature = Some(0.8);
        assert_eq!(context.prepared(&projected).unwrap().max_tokens, 5);
        projected.messages[0].content = crate::ChatMessageContent::Text("edited".into());
        assert!(context.prepared(&projected).is_err());
    }

    #[test]
    fn native_projection_keeps_image_capabilities_and_untrusted_context_cannot_be_injected() {
        let request = parse(
            json!({"model":"fixture","max_tokens":4,"messages":[{"role":"user","content":[{"type":"image","source":{"type":"url","url":"https://example.test/image.png"}}]}]}),
        );
        request.validate().unwrap();
        let projection = request.policy_projection();
        assert_eq!(projection["messages"][0]["content"][1]["type"], "image_url");
        let mut hostile = projection;
        hostile["native_messages"] =
            json!({"request":{"model":"different-provider/private-model"}});
        let parsed: crate::CompletionRequest = serde_json::from_value(hostile).unwrap();
        assert!(parsed.native_messages.is_none());
    }

    #[tokio::test]
    async fn native_json_body_limit_is_enforced_before_parsing() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(path("/oversized"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"input_tokens":17,"padding":"long response"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let response = reqwest::Client::new()
            .get(format!("{}/oversized", server.uri()))
            .send()
            .await
            .unwrap();
        assert!(read_json_response(response, 8)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds 8 bytes"));
    }

    #[tokio::test]
    async fn direct_transport_preserves_envelope_status_and_provider_credentials() {
        use crate::{anthropic::AnthropicProvider, ModelProvider};
        use wiremock::{
            matchers::{body_json, header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let request = parse(
            json!({"model":"claude-native-fixture","max_tokens":0,"stream":false,
            "messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"exact","signature":"sig+/="}]}],
            "cache_control":{"type":"ephemeral","ttl":"1h"}}),
        );
        let body = serde_json::to_value(&request).unwrap();
        let native_error = json!({"type":"error","error":{"type":"rate_limit_error","message":"retry later"},"request_id":"req_fixture"});
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "configured-provider-secret"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(header("anthropic-beta", "fixture-beta"))
            .and(body_json(body))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "12")
                    .set_body_json(native_error.clone()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider = AnthropicProvider::with_base_url(
            "configured-provider-secret".into(),
            format!("{}/v1", server.uri()),
        )
        .unwrap();
        assert!(provider.supports_native_messages());
        let response = provider
            .native_messages(
                &request,
                &MessagesHeaders {
                    version: None,
                    beta: Some("fixture-beta".into()),
                },
                MessagesOperation::Create,
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers()["retry-after"], "12");
        assert_eq!(response.json::<Value>().await.unwrap(), native_error);
        let received = server.received_requests().await.unwrap();
        for name in ["authorization", "x-stainless-os", "anthropic-workspace-id"] {
            assert!(!received[0].headers.contains_key(name));
        }
    }

    #[tokio::test]
    async fn direct_counting_uses_verified_path_and_preserves_reported_count() {
        use crate::{anthropic::AnthropicProvider, ModelProvider};
        use wiremock::{
            matchers::{body_json, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let request = parse(
            json!({"model":"fixture","max_tokens":0,"messages":[{"role":"user","content":"hi"}]}),
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .and(body_json(
                json!({"model":"fixture","messages":[{"role":"user","content":"hi"}]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"input_tokens":17})))
            .expect(1)
            .mount(&server)
            .await;
        let provider =
            AnthropicProvider::with_base_url("provider-key".into(), format!("{}/v1", server.uri()))
                .unwrap();
        let response = provider
            .native_messages(
                &request,
                &MessagesHeaders::default(),
                MessagesOperation::CountTokens,
            )
            .await
            .unwrap();
        assert_eq!(response.json::<Value>().await.unwrap()["input_tokens"], 17);
    }

    #[tokio::test]
    async fn native_stream_transport_preserves_events_and_error_bytes() {
        use crate::{anthropic::AnthropicProvider, ModelProvider};
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let wire="event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig+/=\"}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\"}}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(wire, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let request = parse(
            json!({"model":"fixture","max_tokens":10,"stream":true,"messages":[{"role":"user","content":"hi"}]}),
        );
        let provider =
            AnthropicProvider::with_base_url("provider-key".into(), format!("{}/v1", server.uri()))
                .unwrap();
        let response = provider
            .native_messages(
                &request,
                &MessagesHeaders::default(),
                MessagesOperation::Create,
            )
            .await
            .unwrap();
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.text().await.unwrap(), wire);
    }

    #[tokio::test]
    async fn native_transport_does_not_follow_credential_bearing_redirects() {
        use crate::{anthropic::AnthropicProvider, ModelProvider};
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/steal", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/steal"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let request = parse(
            json!({"model":"fixture","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}),
        );
        let provider =
            AnthropicProvider::with_base_url("provider-key".into(), format!("{}/v1", server.uri()))
                .unwrap();
        let response = provider
            .native_messages(
                &request,
                &MessagesHeaders::default(),
                MessagesOperation::Create,
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 307);
    }
}
