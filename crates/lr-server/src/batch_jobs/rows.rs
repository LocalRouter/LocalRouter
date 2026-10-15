//! The deliberately bounded first native batch payload subset. Accepted fields
//! remain on the sealed wire body; unsupported actions never disappear in serde.
use super::types::InputRow;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) fn parse_input(bytes: &[u8], endpoint: &str) -> ApiResult<(Vec<InputRow>, String)> {
    lr_batches::validate_jsonl(bytes).map_err(|_| {
        ApiErrorResponse::bad_request("Batch input must be bounded UTF-8 JSONL objects")
    })?;
    if !["/v1/chat/completions", "/v1/responses", "/v1/embeddings"].contains(&endpoint) {
        return Err(ApiErrorResponse::bad_request(
            "This native batch endpoint is unsupported",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ApiErrorResponse::bad_request("Batch input must be UTF-8"))?;
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mut model = None;
    let mut embedding_inputs = 0;
    for (index, line) in text.lines().enumerate() {
        let row: InputRow = serde_json::from_str(line).map_err(|_| {
            ApiErrorResponse::bad_request(format!("Invalid batch envelope on row {}", index + 1))
        })?;
        if row.method != "POST"
            || row.url != endpoint
            || row.custom_id.is_empty()
            || row.custom_id.len() > 256
            || row.custom_id.chars().any(char::is_control)
            || !seen.insert(row.custom_id.clone())
        {
            return Err(ApiErrorResponse::bad_request(
                "Rows require POST, the same endpoint and unique bounded custom IDs",
            ));
        }
        validate_body(endpoint, &row.body)?;
        let requested = row.body["model"].as_str().ok_or_else(|| {
            ApiErrorResponse::bad_request("Every row requires an explicit provider/model")
        })?;
        if requested.is_empty()
            || requested.len() > 768
            || model.as_ref().is_some_and(|m| m != requested)
        {
            return Err(ApiErrorResponse::bad_request(
                "Every row must use the same explicit provider/model",
            ));
        }
        model = Some(requested.to_owned());
        if endpoint == "/v1/embeddings" {
            embedding_inputs += embedding_count(&row.body)?;
            if embedding_inputs > lr_batches::MAX_ROWS {
                return Err(ApiErrorResponse::bad_request(
                    "A native batch supports at most 50000 embedding inputs",
                ));
            }
        }
        rows.push(row);
    }
    Ok((
        rows,
        model.ok_or_else(|| ApiErrorResponse::bad_request("Batch input is empty"))?,
    ))
}
pub(crate) fn validate_body(endpoint: &str, body: &Value) -> ApiResult<()> {
    let allowed: &[&str] = match endpoint {
        "/v1/chat/completions" => &[
            "model",
            "messages",
            "temperature",
            "top_p",
            "max_tokens",
            "max_completion_tokens",
            "n",
            "stop",
            "presence_penalty",
            "frequency_penalty",
            "logit_bias",
            "logprobs",
            "top_logprobs",
            "seed",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "response_format",
            "user",
            "metadata",
            "store",
            "stream",
            "reasoning_effort",
        ],
        "/v1/responses" => &[
            "model",
            "input",
            "instructions",
            "temperature",
            "top_p",
            "max_output_tokens",
            "text",
            "user",
            "metadata",
            "store",
            "stream",
        ],
        "/v1/embeddings" => &["model", "input", "encoding_format", "dimensions", "user"],
        _ => {
            return Err(ApiErrorResponse::bad_request(
                "Unsupported native batch endpoint",
            ))
        }
    };
    keys(body, allowed)?;
    if body.get("stream").is_some_and(|v| v != false)
        || body.get("store").is_some_and(|v| v != false)
    {
        return Err(ApiErrorResponse::bad_request(
            "Batch streaming and stored response continuations are unsupported",
        ));
    }
    if body.get("n").is_some_and(|v| v != 1) {
        return Err(ApiErrorResponse::bad_request(
            "Native batches currently support one choice per row",
        ));
    }
    if endpoint != "/v1/embeddings" {
        let fields = if endpoint == "/v1/responses" {
            &["max_output_tokens"][..]
        } else {
            &["max_tokens", "max_completion_tokens"][..]
        };
        let caps = fields
            .iter()
            .filter_map(|key| body.get(*key))
            .collect::<Vec<_>>();
        if caps.len() != 1 || !caps[0].as_u64().is_some_and(|v| (1..=65536).contains(&v)) {
            return Err(ApiErrorResponse::bad_request(
                "Every generation row needs one explicit output cap in 1..65536",
            ));
        }
    }
    match endpoint {
        "/v1/chat/completions" => {
            let messages = body["messages"]
                .as_array()
                .filter(|m| !m.is_empty())
                .ok_or_else(|| ApiErrorResponse::bad_request("Batch chat requires messages"))?;
            for message in messages {
                keys(
                    message,
                    &["role", "content", "name", "tool_calls", "tool_call_id"],
                )?;
                if !message["role"].as_str().is_some_and(|r| {
                    ["system", "developer", "user", "assistant", "tool"].contains(&r)
                }) {
                    return Err(ApiErrorResponse::bad_request(
                        "Unsupported batch message role",
                    ));
                }
                text_content(message.get("content"), &["text"], true)?;
                if let Some(calls) = message.get("tool_calls") {
                    let calls = calls.as_array().ok_or_else(|| {
                        ApiErrorResponse::bad_request("Invalid client tool calls")
                    })?;
                    for call in calls {
                        keys(call, &["id", "type", "function"])?;
                        if call["type"] != "function" {
                            return Err(ApiErrorResponse::bad_request(
                                "Only client function calls are supported in chat batches",
                            ));
                        }
                        keys(&call["function"], &["name", "arguments"])?;
                    }
                }
            }
            if let Some(tools) = body.get("tools") {
                let tools = tools
                    .as_array()
                    .ok_or_else(|| ApiErrorResponse::bad_request("Invalid batch tools"))?;
                for tool in tools {
                    keys(tool, &["type", "function"])?;
                    if tool["type"] != "function" {
                        return Err(ApiErrorResponse::bad_request("Server tools and automatic tool execution are unsupported in native batches"));
                    }
                    keys(
                        &tool["function"],
                        &["name", "description", "parameters", "strict"],
                    )?;
                }
            }
            let _: crate::types::ChatCompletionRequest = serde_json::from_value(body.clone())
                .map_err(|_| ApiErrorResponse::bad_request("Invalid batch chat request"))?;
        }
        "/v1/responses" => {
            if body.get("instructions").is_some_and(|v| !v.is_string()) {
                return Err(ApiErrorResponse::bad_request(
                    "Batch instructions must be text",
                ));
            }
            response_messages(body)?;
        }
        "/v1/embeddings" => {
            embedding_count(body)?;
            if body
                .get("encoding_format")
                .is_some_and(|v| v != "float" && v != "base64")
                || body
                    .get("dimensions")
                    .is_some_and(|v| !v.as_u64().is_some_and(|v| (1..=32768).contains(&v)))
            {
                return Err(ApiErrorResponse::bad_request(
                    "Unsupported embedding encoding or dimensions",
                ));
            }
        }
        _ => unreachable!(),
    }
    reject_external_schema_refs(body)?;
    Ok(())
}
fn keys(value: &Value, allowed: &[&str]) -> ApiResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiErrorResponse::bad_request("Batch request parts must be objects"))?;
    if let Some(key) = object.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(ApiErrorResponse::bad_request(format!(
            "Native batch field '{key}' is unsupported"
        )));
    }
    Ok(())
}
fn text_content(value: Option<&Value>, types: &[&str], nullable: bool) -> ApiResult<String> {
    match value {
        None | Some(Value::Null) if nullable => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Array(parts)) => {
            let mut text = String::new();
            for part in parts {
                keys(part, &["type", "text"])?;
                if !part["type"].as_str().is_some_and(|t| types.contains(&t)) {
                    return Err(ApiErrorResponse::bad_request(
                        "Only inline text is supported in this batch endpoint",
                    ));
                }
                text.push_str(
                    part["text"]
                        .as_str()
                        .ok_or_else(|| ApiErrorResponse::bad_request("Invalid batch text block"))?,
                );
                text.push('\n');
            }
            Ok(text)
        }
        _ => Err(ApiErrorResponse::bad_request(
            "Unsupported batch content or remote file reference",
        )),
    }
}
pub(crate) fn response_messages(body: &Value) -> ApiResult<Vec<Value>> {
    let mut messages = Vec::new();
    if let Some(instructions) = body["instructions"].as_str() {
        messages.push(json!({"role":"system","content":instructions}));
    }
    match &body["input"] {
        Value::String(text) if !text.is_empty() => {
            messages.push(json!({"role":"user","content":text}))
        }
        Value::Array(items) if !items.is_empty() => {
            for item in items {
                keys(item, &["type", "role", "content"])?;
                if item.get("type").is_some_and(|v| v != "message")
                    || !item["role"]
                        .as_str()
                        .is_some_and(|r| ["user", "assistant", "system", "developer"].contains(&r))
                {
                    return Err(ApiErrorResponse::bad_request(
                        "Native Responses batches currently support text messages only",
                    ));
                }
                messages.push(json!({"role":item["role"],"content":text_content(item.get("content"),&["input_text","output_text"],false)?}));
            }
        }
        _ => {
            return Err(ApiErrorResponse::bad_request(
                "Batch Responses requires nonempty text input",
            ))
        }
    }
    Ok(messages)
}
pub(crate) fn embedding_count(body: &Value) -> ApiResult<usize> {
    match &body["input"] {
        Value::String(text) if !text.is_empty()=>Ok(1),
        Value::Array(items) if !items.is_empty() && items.iter().all(|v|v.as_str().is_some_and(|s|!s.is_empty()))=>Ok(items.len()),
        _=>Err(ApiErrorResponse::bad_request("Batch embeddings currently support text strings, not opaque token arrays or other modalities")),
    }
}
fn reject_external_schema_refs(value: &Value) -> ApiResult<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "$ref" && value.as_str().is_some_and(|v| !v.starts_with('#')) {
                    return Err(ApiErrorResponse::bad_request(
                        "External schema references are unsupported in batches",
                    ));
                }
                reject_external_schema_refs(value)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_external_schema_refs(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn token_bounds(endpoint: &str, body: &Value) -> ApiResult<(u64, u64)> {
    validate_body(endpoint, body)?;
    let inputs = serde_json::to_vec(body)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid batch body"))?
        .len() as u64;
    let framing = body["messages"].as_array().map_or(1, Vec::len) as u64
        + body["tools"].as_array().map_or(0, Vec::len) as u64;
    let output = if endpoint == "/v1/embeddings" {
        0
    } else {
        body["max_tokens"]
            .as_u64()
            .or_else(|| body["max_completion_tokens"].as_u64())
            .or_else(|| body["max_output_tokens"].as_u64())
            .unwrap_or(0)
    };
    Ok((inputs.saturating_add(1024 * framing.max(1)), output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chat() -> Value {
        json!({"model":"provider/model","max_tokens":16,"messages":[{"role":"user","content":"hello"}]})
    }
    #[test]
    fn generation_caps_and_active_remote_actions_are_explicit() {
        let mut body = chat();
        validate_body("/v1/chat/completions", &body).unwrap();
        for (key, value) in [
            ("stream", json!(true)),
            ("store", json!(true)),
            ("plugins", json!([{"id":"web"}])),
            ("provider", json!({"zdr":true})),
            ("max_completion_tokens", json!(32)),
        ] {
            let mut request = body.clone();
            request[key] = value;
            assert!(
                validate_body("/v1/chat/completions", &request).is_err(),
                "{key}"
            );
        }
        body["tools"] = json!([{"type":"web_search"}]);
        assert!(validate_body("/v1/chat/completions", &body).is_err());
        let mut body = chat();
        body["messages"][0]["content"] =
            json!([{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]);
        assert!(validate_body("/v1/chat/completions", &body).is_err());
    }
    #[test]
    fn caller_tools_are_data_and_opaque_history_is_rejected() {
        let mut body = chat();
        body["tools"] = json!([{"type":"function","function":{"name":"lookup","parameters":{"type":"object","properties":{"query":{"type":"string"}}}}}]);
        validate_body("/v1/chat/completions", &body).unwrap();
        body["messages"][0]["reasoning_details"] = json!([{ "signature":"opaque" }]);
        assert!(validate_body("/v1/chat/completions", &body).is_err());
        let mut body = chat();
        body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"test","schema":{"$ref":"https://example.com/schema"}}});
        assert!(validate_body("/v1/chat/completions", &body).is_err());
    }
    #[test]
    fn response_and_embedding_subsets_preserve_supported_shapes() {
        let response = json!({"model":"provider/model","input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}],"max_output_tokens":16});
        validate_body("/v1/responses", &response).unwrap();
        assert_eq!(
            response_messages(&response).unwrap()[0]["content"],
            "hello\n"
        );
        let mut opaque = response;
        opaque["previous_response_id"] = json!("resp_unowned");
        assert!(validate_body("/v1/responses", &opaque).is_err());
        let embeddings = json!({"model":"provider/embedding","input":["one","two"],"encoding_format":"float","dimensions":32});
        validate_body("/v1/embeddings", &embeddings).unwrap();
        assert_eq!(embedding_count(&embeddings).unwrap(), 2);
        let mut opaque = embeddings;
        opaque["input"] = json!([1, 2, 3]);
        assert!(validate_body("/v1/embeddings", &opaque).is_err());
    }
    #[test]
    fn immutable_manifest_rejects_duplicates_endpoints_and_model_movement() {
        let envelope = |id: &str, model: &str| {
            json!({"custom_id":id,"method":"POST","url":"/v1/chat/completions","body":{"model":model,"max_tokens":4,"messages":[{"role":"user","content":"hi"}]}}).to_string()
        };
        let input = format!(
            "{}\n{}\n",
            envelope("one", "provider/model"),
            envelope("two", "provider/model")
        );
        assert_eq!(
            parse_input(input.as_bytes(), "/v1/chat/completions")
                .unwrap()
                .0
                .len(),
            2
        );
        for input in [
            format!(
                "{}\n{}\n",
                envelope("one", "provider/model"),
                envelope("one", "provider/model")
            ),
            format!(
                "{}\n{}\n",
                envelope("one", "provider/model"),
                envelope("two", "other/model")
            ),
        ] {
            assert!(parse_input(input.as_bytes(), "/v1/chat/completions").is_err());
        }
        assert!(parse_input(input.as_bytes(), "/v1/responses").is_err());
    }
}
