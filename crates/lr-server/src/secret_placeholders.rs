//! Authenticated gateway boundary: protect before monitoring/auxiliaries, restore
//! after the tokenized response has been recorded. No prompt instructions added.
use crate::{
    middleware::error::ApiErrorResponse,
    state::{AppState, AuthContext},
};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use lr_secret_scanner::placeholders::{TextStream, Vault};
use lr_types::secret_context::{self, SecretContext};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
const LIMIT: usize = 16 * 1024 * 1024; // Main API body limit.

pub async fn middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    match run(
        state,
        request,
        next,
        Arc::new(lr_api_keys::keychain_trait::SystemKeychain),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}
async fn run(
    state: AppState,
    request: Request,
    next: Next,
    storage: Arc<dyn lr_api_keys::KeychainStorage>,
) -> Result<Response, ApiErrorResponse> {
    let Some(auth) = request.extensions().get::<AuthContext>() else {
        return Ok(next.run(request).await);
    };
    let owner = auth.api_key_id.clone();
    let path = request
        .uri()
        .path()
        .strip_prefix("/v1")
        .unwrap_or(request.uri().path())
        .to_owned();
    let config = state.config_manager.get();
    let client = state.client_manager.get_client(&owner);
    let action = client
        .as_ref()
        .and_then(|c| c.secret_scanning.action.as_ref())
        .unwrap_or(&config.secret_scanning.action);
    if !matches!(
        path.as_str(),
        "/chat/completions"
            | "/completions"
            | "/responses"
            | "/messages"
            | "/systemone"
            | "/decisions"
    ) || request.method() != axum::http::Method::POST
    {
        if *action == lr_config::SecretScanAction::Redact
            && request.method() == axum::http::Method::POST
        {
            return Err(ApiErrorResponse::bad_request(
                "Automatic secret redaction is supported on text gateway endpoints only",
            ));
        }
        return Ok(next.run(request).await);
    }
    if !matches!(
        action,
        lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
    ) {
        return Ok(next.run(request).await);
    }
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, LIMIT)
        .await
        .map_err(|_| ApiErrorResponse::bad_request("Could not read protected request"))?;
    let mut value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid request JSON"))?;
    let session = request_session(&value, &parts.headers)?;
    let ephemeral = session.is_none();
    let session = session.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let vault = Arc::new(
        if let Some(previous) = value.get("previous_response_id").and_then(Value::as_str) {
            Vault::continue_response(storage, &owner, previous).map_err(|_| {
                ApiErrorResponse::bad_request(
                    "Secret scope for previous_response_id is missing or expired",
                )
            })?
        } else {
            Vault::new(storage, &owner, &session, ephemeral && path != "/responses")
        },
    );
    if value
        .pointer("/extensions/localrouter.session/reset")
        .and_then(Value::as_bool)
        == Some(true)
    {
        vault.forget().map_err(|_| {
            ApiErrorResponse::bad_request("Could not reset secret credential storage")
        })?;
    }
    let context: Arc<dyn SecretContext> = vault.clone();
    secret_context::scope(Some(context.clone()),async {
        // Native server tools execute at the provider and cannot be restored locally.
        if value.get("tools").and_then(Value::as_array).is_some_and(|tools|tools.iter().any(|t|t.get("type").and_then(Value::as_str).is_some_and(|kind|!matches!(kind,"function"|"custom")))) {
            return Err(ApiErrorResponse::bad_request("Restorable secrets require locally executed tools"));
        }
        let model=value.get("model").and_then(Value::as_str).unwrap_or("").to_owned();
        let (projection,_)=crate::documents::scan_projection(&value).map_err(ApiErrorResponse::from)?;
        match crate::routes::pipeline::scan_request_for_secrets_with_source(&state,&owner,&model,&projection,None,Some(&session)).await {
            crate::routes::pipeline::SecretScanOutcome::Deny(message)=>return Err(ApiErrorResponse::forbidden(message)),
            crate::routes::pipeline::SecretScanOutcome::Redact(redaction)=>{
                crate::documents::redact_text_files(&mut value,&redaction).map_err(ApiErrorResponse::from)?;
                redaction.apply(&mut value);
            },
            _=>{}
        }
        crate::documents::protect_text_files(&mut value,context.as_ref()).map_err(ApiErrorResponse::from)?;
        context.protect(&mut value).map_err(ApiErrorResponse::from)?;
        context.mark_scanned();
        // Subsequent automatic tool iterations can introduce new credentials.
        // Reuse the exact scanner/exceptions rather than exposing them to the provider.
        let scan_state=state.clone(); let scan_owner=owner.clone();
        let automatic=*action==lr_config::SecretScanAction::Redact;
        let allowed_plaintext = scan_state.secret_scanner.read().as_ref().map(|scanner|{
            let texts=lr_guardrails::text_extractor::extract_request_text(&value).into_iter().enumerate().map(|(i,t)|lr_secret_scanner::ExtractedText{label:t.label,text:t.text,message_index:i}).collect::<Vec<_>>();
            scanner.scan(&texts).findings.into_iter().map(|f|f.raw_text).collect::<std::collections::HashSet<_>>()
        }).unwrap_or_default();
        let weak_vault=Arc::downgrade(&vault);
        vault.set_detector(Arc::new(move |body| {
            let scanner=scan_state.secret_scanner.read().clone();
            let Some(scanner)=scanner else {return Err(lr_types::AppError::Internal("Secret scanner unavailable".into()))};
            let mut texts=lr_guardrails::text_extractor::extract_request_text(body);
            if texts.is_empty() && body.get("content").is_some(){
                let mut copy=body.clone();
                lr_secret_scanner::placeholders::visit(&mut copy,&mut |text,_|{texts.push(lr_guardrails::text_extractor::ExtractedText{label:"tool".into(),text:text.into(),message_index:None,role:"tool".into()});Ok(text.into())},false)?;
            }
            let texts=texts.into_iter().enumerate().map(|(i,t)|lr_secret_scanner::ExtractedText{label:t.label,text:t.text,message_index:i}).collect::<Vec<_>>();
            let result=scanner.scan(&texts);
            let client=scan_state.client_manager.get_client(&scan_owner);
            let scanning=client.as_ref().map(|c|&c.secret_scanning);
            let salt=scanning.and_then(|c|c.dismiss_salt.as_deref());
            let exceptions=scanning.map(|c|c.dismissed_secrets.as_slice()).unwrap_or_default();
            let ignored=exceptions.iter().filter(|e|!e.redact).map(|e|lr_secret_scanner::dismissal::DismissedDigest{hash:&e.hash,iterations:e.iterations}).collect::<Vec<_>>();
            let permanent=exceptions.iter().filter(|e|e.redact).map(|e|lr_secret_scanner::dismissal::DismissedDigest{hash:&e.hash,iterations:e.iterations}).collect::<Vec<_>>();
            let mut irreversible=lr_secret_scanner::redaction::Redaction::default();let mut secrets=Vec::new();
            for finding in result.findings {
                if scan_state.secret_dismissal_cache.is_dismissed(&format!("redact:{scan_owner}"),salt,&permanent,&finding.raw_text){irreversible.add(&finding.raw_text);}
                else if !scan_state.secret_dismissal_cache.is_dismissed(&scan_owner,salt,&ignored,&finding.raw_text) {
                    if !automatic && allowed_plaintext.contains(&finding.raw_text){continue;}
                    if !automatic && !weak_vault.upgrade().is_some_and(|v|v.knows(&finding.raw_text).unwrap_or(false)){return Err(lr_types::AppError::InvalidParams("A tool returned a new secret; resubmit it for an explicit secret decision".into()));}
                    secrets.push(finding.raw_text);
                }
            }
            irreversible.apply(body);Ok(secrets)
        }));
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        let body=serde_json::to_vec(&value).map_err(|_|ApiErrorResponse::internal_error("Could not encode protected request"))?;
        let response=next.run(Request::from_parts(parts,Body::from(body))).await;
        restore_response(response,context,vault).await
    }).await
}
fn request_session(
    value: &Value,
    headers: &axum::http::HeaderMap,
) -> Result<Option<String>, ApiErrorResponse> {
    let mut options = value.pointer("/extensions/localrouter.session").cloned();
    if let Some(header) = [
        "x-session-id",
        "x-session-affinity",
        "session_id",
        "session-id",
    ]
    .iter()
    .find_map(|key| headers.get(*key))
    {
        let id = header
            .to_str()
            .map_err(|_| ApiErrorResponse::bad_request("Invalid session header"))?;
        let object = options
            .get_or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| ApiErrorResponse::bad_request("Session options must be an object"))?;
        object.insert("id".into(), Value::String(id.into()));
    }
    lr_providers::session::SessionOptions::parse(options.as_ref())
        .map(|o| o.map(|o| o.id))
        .map_err(ApiErrorResponse::from)
}
async fn restore_response(
    response: Response,
    context: Arc<dyn SecretContext>,
    vault: Arc<Vault>,
) -> Result<Response, ApiErrorResponse> {
    let streaming = response
        .headers()
        .get("content-type")
        .is_some_and(|v| v.as_bytes().starts_with(b"text/event-stream"));
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    parts.headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if !streaming {
        let bytes = to_bytes(body, LIMIT)
            .await
            .map_err(|_| ApiErrorResponse::internal_error("Protected response exceeds limit"))?;
        let mut value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ApiErrorResponse::internal_error("Invalid protected response"))?;
        if let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("resp_"))
        {
            vault.bind_response(id).map_err(ApiErrorResponse::from)?;
        }
        context
            .restore(&mut value, false)
            .map_err(ApiErrorResponse::from)?;
        let encoded = serde_json::to_vec(&value)
            .map_err(|_| ApiErrorResponse::internal_error("Could not encode restored response"))?;
        if encoded.len() > LIMIT {
            return Err(ApiErrorResponse::internal_error(
                "Restored response exceeds the gateway response budget",
            ));
        }
        return Ok(Response::from_parts(parts, Body::from(encoded)));
    }
    let stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send>,
    > = Box::pin(async_stream::try_stream! {
        let mut body=body.into_data_stream();let mut pending=Vec::new();let mut scanned=0;let mut codec=Events::default();
        while let Some(chunk)=secret_context::scope(Some(context.clone()),body.next()).await {
            let chunk=chunk.map_err(std::io::Error::other)?;pending.extend_from_slice(&chunk);
            if pending.len()>LIMIT {Err(std::io::Error::other("Protected response frame exceeds limit"))?;}
            while let Some((relative,separator))=frame_end(&pending[scanned..]){
                let end=scanned+relative;scanned=0;
                let frame=String::from_utf8(pending.drain(..end).collect()).map_err(std::io::Error::other)?;
                pending.drain(..separator);
                // Bind the response identity independently of delta content.
                if let Some(data)=frame.lines().find_map(|line|line.strip_prefix("data: ")) {
                    if let Ok(value)=serde_json::from_str::<Value>(data) {
                        if let Some(id)=value.pointer("/response/id").or_else(||value.get("id")).and_then(Value::as_str).filter(|id|id.starts_with("resp_")) {vault.bind_response(id).map_err(std::io::Error::other)?;}
                    }
                }
                for event in codec.frame(&frame,&context).map_err(std::io::Error::other)? {yield axum::body::Bytes::from(event);}
            }
            scanned=pending.len().saturating_sub(3);
        }
        if !pending.is_empty(){Err(std::io::Error::other("Incomplete protected response event"))?;}
        if !codec.done {Err(std::io::Error::other("Protected stream ended without completion"))?;}
        for event in codec.finish(&context).map_err(std::io::Error::other)? {yield axum::body::Bytes::from(event);}
    });
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}
fn frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}
#[derive(Default)]
struct Events {
    done: bool,
    text: BTreeMap<String, (TextStream, Value, String)>,
    arguments: BTreeMap<String, (String, Value, String)>,
}
fn encode(value: &Value, event: &str) -> String {
    format!(
        "{}data: {}\n\n",
        if event.is_empty() {
            String::new()
        } else {
            format!("event: {event}\n")
        },
        value
    )
}
impl Events {
    fn frame(
        &mut self,
        frame: &str,
        context: &Arc<dyn SecretContext>,
    ) -> lr_types::AppResult<Vec<String>> {
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        let event = frame
            .lines()
            .find_map(|line| line.strip_prefix("event:").map(str::trim))
            .unwrap_or("");
        if data == "[DONE]" {
            self.done = true;
            let mut out = self.finish(context)?;
            out.push("data: [DONE]\n\n".into());
            return Ok(out);
        }
        if data.is_empty() {
            return Ok(vec![format!("{frame}\n\n")]);
        }
        let mut value: Value = serde_json::from_str(&data)
            .map_err(|_| lr_types::AppError::Internal("Invalid protected SSE event".into()))?;
        let kind = value["type"].as_str().unwrap_or("");
        let mut prefixes = Vec::new();
        let all = matches!(
            kind,
            "response.completed" | "response.failed" | "message_stop"
        );
        if all {
            self.done = true;
        }
        if kind == "content_block_stop" {
            prefixes.push(format!("messages:{}:", value["index"]));
        }
        if kind.ends_with(".done") {
            let identity = value
                .get("item_id")
                .or_else(|| value.pointer("/item/id"))
                .unwrap_or(&Value::Null);
            prefixes.push(format!("responses:{identity}:"));
        }
        if let Some(choices) = value["choices"].as_array() {
            for choice in choices {
                if !choice["finish_reason"].is_null() {
                    prefixes.push(format!("chat:{}:", choice["index"]));
                }
            }
        }
        let mut out = self.flush(context, |channel| {
            all || prefixes.iter().any(|prefix| channel.starts_with(prefix))
        })?;
        let mut paths = Vec::new();
        collect_deltas(&value, "", &mut paths);
        for (path, arguments) in paths {
            let arguments = arguments
                || value["type"].as_str().is_some_and(|kind| {
                    (kind.contains("arguments") || kind.contains("custom_tool_call_input"))
                        && kind.ends_with(".delta")
                });
            let Some(text) = value
                .pointer(&path)
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            // Include wire identities, not just array position, for interleaved calls.
            let channel = channel_key(&value, &path);
            let mut template = value.clone();
            clear_delta_strings(&mut template);
            if arguments {
                let entry = self
                    .arguments
                    .entry(channel)
                    .or_insert_with(|| (String::new(), template, path.clone()));
                entry.0.push_str(&text);
                if entry.0.len() > LIMIT {
                    return Err(lr_types::AppError::Internal(
                        "Protected tool arguments exceed limit".into(),
                    ));
                }
                *value.pointer_mut(&path).unwrap() = Value::String(String::new());
            } else {
                let entry = self
                    .text
                    .entry(channel)
                    .or_insert_with(|| (TextStream::default(), template, path.clone()));
                let restored = entry
                    .0
                    .push(&text, false, |s| context.restore_text(s, false))?;
                *value.pointer_mut(&path).unwrap() = Value::String(restored);
            }
        }
        // Deltas are handled incrementally above; full snapshots need a fresh pass.
        if !is_delta(&value) {
            context.restore(&mut value, false)?;
        }
        out.push(encode(&value, event));
        Ok(out)
    }
    fn finish(&mut self, context: &Arc<dyn SecretContext>) -> lr_types::AppResult<Vec<String>> {
        self.flush(context, |_| true)
    }
    fn flush(
        &mut self,
        context: &Arc<dyn SecretContext>,
        selected: impl Fn(&str) -> bool,
    ) -> lr_types::AppResult<Vec<String>> {
        let mut out = Vec::new();
        let keys: Vec<_> = self
            .text
            .keys()
            .filter(|key| selected(key))
            .cloned()
            .collect();
        for key in keys {
            let (mut stream, mut template, path) =
                self.text.remove(&key).expect("selected channel");
            let text = stream.push("", true, |s| context.restore_text(s, false))?;
            if !text.is_empty() {
                *template.pointer_mut(&path).unwrap() = Value::String(text);
                out.push(encode(&template, template["type"].as_str().unwrap_or("")));
            }
        }
        let keys: Vec<_> = self
            .arguments
            .keys()
            .filter(|key| selected(key))
            .cloned()
            .collect();
        for key in keys {
            let (arguments, mut template, path) = self
                .arguments
                .remove(&key)
                .expect("selected argument channel");
            let restored = if key.contains("custom_tool_call_input") {
                context.restore_text(&arguments, true)?
            } else {
                let mut args: Value = serde_json::from_str(&arguments).map_err(|_| {
                    lr_types::AppError::InvalidParams("Incomplete protected tool arguments".into())
                })?;
                context.restore(&mut args, true)?;
                serde_json::to_string(&args).map_err(|_| {
                    lr_types::AppError::Internal("Could not restore tool arguments".into())
                })?
            };
            *template.pointer_mut(&path).unwrap() = Value::String(restored);
            out.push(encode(&template, template["type"].as_str().unwrap_or("")));
        }
        Ok(out)
    }
}
fn channel_key(value: &Value, path: &str) -> String {
    if path.starts_with("/choices/") {
        let parts: Vec<_> = path.split('/').collect();
        let choice = value
            .pointer(&format!("/choices/{}/index", parts[2]))
            .unwrap_or(&Value::Null);
        let call = parts
            .iter()
            .position(|s| *s == "tool_calls")
            .and_then(|i| parts.get(i + 1))
            .and_then(|i| {
                value.pointer(&format!("/choices/{}/delta/tool_calls/{i}/index", parts[2]))
            })
            .unwrap_or(&Value::Null);
        let field = parts.last().unwrap_or(&"");
        return format!("chat:{choice}:{call}:{field}");
    }
    if value["type"]
        .as_str()
        .is_some_and(|s| s.starts_with("content_block"))
    {
        return format!("messages:{}:{path}", value["index"]);
    }
    format!(
        "responses:{}:{}:{}:{path}",
        value["item_id"], value["content_index"], value["type"]
    )
}
fn clear_delta_strings(value: &mut Value) {
    if let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            if let Some(delta) = choice.get_mut("delta") {
                fn clear(v: &mut Value) {
                    match v {
                        Value::String(s) => s.clear(),
                        Value::Array(a) => {
                            for v in a {
                                clear(v)
                            }
                        }
                        Value::Object(m) => {
                            m.remove("role");
                            m.remove("id");
                            m.remove("name");
                            for v in m.values_mut() {
                                clear(v)
                            }
                        }
                        _ => {}
                    }
                }
                clear(delta);
            }
        }
    } else if let Some(delta) = value.get_mut("delta") {
        fn clear(v: &mut Value) {
            match v {
                Value::String(s) => s.clear(),
                Value::Object(m) => {
                    for (k, v) in m {
                        if k != "type" {
                            clear(v)
                        }
                    }
                }
                _ => {}
            }
        }
        clear(delta);
    }
}
fn is_delta(value: &Value) -> bool {
    value
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.get("delta").is_some()))
        || value["type"]
            .as_str()
            .is_some_and(|s| s.ends_with(".delta") || s == "content_block_delta")
}
fn collect_deltas(value: &Value, path: &str, out: &mut Vec<(String, bool)>) {
    if value.pointer("/delta/type").and_then(Value::as_str) == Some("thinking_delta") {
        return;
    }
    if !is_delta(value) {
        return;
    }
    fn walk(value: &Value, path: &str, out: &mut Vec<(String, bool)>) {
        match value {
            Value::Object(map) => {
                for (key, v) in map {
                    let p = format!("{path}/{key}");
                    if v.is_string()
                        && matches!(
                            key.as_str(),
                            "content"
                                | "reasoning_content"
                                | "text"
                                | "thinking"
                                | "refusal"
                                | "delta"
                                | "arguments"
                                | "partial_json"
                        )
                    {
                        out.push((
                            p,
                            matches!(key.as_str(), "arguments" | "partial_json")
                                || value["type"]
                                    .as_str()
                                    .is_some_and(|s| s.contains("arguments")),
                        ))
                    } else {
                        walk(v, &p, out)
                    }
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    walk(v, &format!("{path}/{i}"), out)
                }
            }
            _ => {}
        }
    }
    walk(value, path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Arc<dyn SecretContext>, String, String) {
        let vault: Arc<dyn SecretContext> = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "client",
            "session",
            false,
        ));
        let secret = "synthetic original \"quoted\"\n🔐\\".to_owned();
        vault.register(&secret).unwrap();
        let mut value = json!({"content":secret});
        vault.protect(&mut value).unwrap();
        (vault, value["content"].as_str().unwrap().to_owned(), secret)
    }
    fn data(v: &Value) -> String {
        format!("data: {v}")
    }
    fn values(frames: Vec<String>) -> Vec<Value> {
        frames
            .into_iter()
            .filter_map(|s| {
                s.lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .and_then(|s| serde_json::from_str(s).ok())
            })
            .collect()
    }
    #[test]
    fn secret_placeholders_all_chat_splits_and_parallel_channels() {
        let (context, tag, secret) = fixture();
        for split in 0..=tag.len() {
            let mut events = Events::default();
            let mut output = Vec::new();
            output.extend(
                events
                    .frame(
                        &data(&json!({"choices":[{"index":0,"delta":{"content":&tag[..split]}}]})),
                        &context,
                    )
                    .unwrap(),
            );
            output.extend(
                events
                    .frame(
                        &data(&json!({"choices":[{"index":1,"delta":{"content":"unrelated"}}]})),
                        &context,
                    )
                    .unwrap(),
            );
            output.extend(
                events
                    .frame(
                        &data(&json!({"choices":[{"index":1,"delta":{},"finish_reason":"stop"}]})),
                        &context,
                    )
                    .unwrap(),
            );
            output.extend(
                events
                    .frame(
                        &data(&json!({"choices":[{"index":0,"delta":{"content":&tag[split..]}}]})),
                        &context,
                    )
                    .unwrap(),
            );
            output.extend(events.frame("data: [DONE]", &context).unwrap());
            let actual: String = values(output)
                .iter()
                .flat_map(|v| v["choices"].as_array().into_iter().flatten())
                .filter(|v| v["index"] == 0)
                .filter_map(|v| v.pointer("/delta/content").and_then(Value::as_str))
                .collect();
            assert_eq!(actual, secret, "split {split}");
        }
    }
    #[test]
    fn secret_placeholders_parallel_tool_calls_and_json_escaping() {
        let (context, tag, secret) = fixture();
        let args = json!({"content":tag}).to_string();
        let mut events = Events::default();
        let mut out = Vec::new();
        for (index, fragment) in [
            (0, &args[..args.len() / 2]),
            (1, args.as_str()),
            (0, &args[args.len() / 2..]),
        ] {
            out.extend(events.frame(&data(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":index,"function":{"arguments":fragment}}]}}]})),&context).unwrap());
        }
        out.extend(
            events
                .frame(
                    &data(
                        &json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
                    ),
                    &context,
                )
                .unwrap(),
        );
        let frames = values(out);
        for index in [0, 1] {
            let args: String = frames
                .iter()
                .flat_map(|v| v["choices"].as_array().into_iter().flatten())
                .flat_map(|c| {
                    c.pointer("/delta/tool_calls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter(|call| call["index"] == index)
                .filter_map(|call| call.pointer("/function/arguments").and_then(Value::as_str))
                .collect();
            assert_eq!(
                serde_json::from_str::<Value>(&args).unwrap()["content"],
                secret
            );
        }
    }
    #[test]
    fn secret_placeholders_messages_and_responses_deltas() {
        let (context, tag, secret) = fixture();
        for protocol in ["messages", "responses"] {
            let mut events = Events::default();
            let mut output = Vec::new();
            for chunk in tag.as_bytes().chunks(1) {
                let text = std::str::from_utf8(chunk).unwrap();
                let value = if protocol == "messages" {
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}})
                } else {
                    json!({"type":"response.output_text.delta","item_id":"item","content_index":0,"delta":text})
                };
                output.extend(events.frame(&data(&value), &context).unwrap());
            }
            output.extend(events.finish(&context).unwrap());
            let actual: String = values(output)
                .iter()
                .filter_map(|v| {
                    if protocol == "messages" {
                        v.pointer("/delta/text").and_then(Value::as_str)
                    } else {
                        v["delta"].as_str()
                    }
                })
                .collect();
            assert_eq!(actual, secret);
        }
    }
    #[test]
    fn secret_placeholders_metadata_preserved_and_nested_output_restored() {
        let (context, tag, secret) = fixture();
        let mut value = json!({"id":tag,"signed_block":{"signature":"opaque","thinking":tag},"choices":[{"message":{"content":json!({"password":tag}).to_string(),"reasoning_content":tag,"refusal":tag}}]});
        context.restore(&mut value, false).unwrap();
        assert!(value["id"].as_str().unwrap().starts_with("<localrouter"));
        assert_eq!(value["id"], value["signed_block"]["thinking"]);
        assert_eq!(
            value
                .pointer("/choices/0/message/reasoning_content")
                .unwrap(),
            &json!(secret)
        );
        assert_eq!(
            serde_json::from_str::<Value>(
                value
                    .pointer("/choices/0/message/content")
                    .unwrap()
                    .as_str()
                    .unwrap()
            )
            .unwrap()["password"],
            secret
        );
    }
    #[test]
    fn secret_placeholders_unknown_tool_argument_is_blocked() {
        let (context, _, _) = fixture();
        let mut value = json!({"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"content\":\"<localrouter-redacted id=unknown>\"}"}}]}}]});
        assert!(context.restore(&mut value, false).is_err());
    }
    #[tokio::test]
    async fn secret_placeholders_synthetic_file_write_and_echo() {
        let (context, tag, secret) = fixture();
        // The fake provider sees the protected view only.
        assert!(!tag.contains(&secret));
        let mut tool_args = json!({"content":tag});
        context.restore(&mut tool_args, true).unwrap();
        let dir =
            std::env::temp_dir().join(format!("localrouter-secret-proof-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join(".env");
        std::fs::write(&file, tool_args["content"].as_str().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), secret);
        let mut echoed =
            json!({"messages":[{"role":"tool","content":std::fs::read_to_string(&file).unwrap()}]});
        context.protect(&mut echoed).unwrap();
        assert!(!echoed.to_string().contains("synthetic original"));
        std::fs::remove_dir_all(dir).unwrap();
        secret_context::scope(Some(context), async {
            let task = lr_types::spawn_traced(async {
                assert!(secret_context::current().is_some());
            });
            task.await.unwrap();
        })
        .await;
    }
    #[tokio::test]
    async fn secret_placeholders_json_and_fragmented_sse_body() {
        let (context, tag, secret) = fixture();
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "client",
            "other",
            false,
        ));
        let json_response = Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(json!({"content":tag}).to_string()))
            .unwrap();
        let result = restore_response(json_response, context.clone(), vault.clone())
            .await
            .unwrap();
        let bytes = to_bytes(result.into_body(), LIMIT).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["content"],
            secret
        );
        let wire = format!(
            "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
            json!({"choices":[{"index":0,"delta":{"content":tag}}]})
        );
        let chunks = wire
            .as_bytes()
            .chunks(1)
            .map(|b| Ok::<_, std::io::Error>(axum::body::Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>();
        let response = Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(futures::stream::iter(chunks)))
            .unwrap();
        let result = restore_response(response, context, vault).await.unwrap();
        let bytes = to_bytes(result.into_body(), LIMIT).await.unwrap();
        let actual = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(actual.contains("[DONE]"));
        assert!(!actual.contains("localrouter-redacted"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use axum::{routing::post, Extension, Json};
    use lr_config::{AppConfig, ConfigManager};
    use tower::ServiceExt;
    const SYNTHETIC: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
    fn state(root: &std::path::Path) -> AppState {
        let mut config = AppConfig::default();
        config.secret_scanning.action = lr_config::SecretScanAction::Redact;
        let client =
            lr_config::Client::new_with_strategy("synthetic client".into(), "strategy".into());
        config.clients = vec![client.clone()];
        let config = Arc::new(ConfigManager::new(config, root.join("config.yaml")));
        let providers = Arc::new(lr_providers::registry::ProviderRegistry::new());
        let limits = Arc::new(lr_router::RateLimiterManager::new(None));
        let metrics = Arc::new(lr_monitoring::metrics::MetricsCollector::new(Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(root.join("metrics.db")).unwrap(),
        )));
        let router = Arc::new(lr_router::Router::new(
            config.clone(),
            providers.clone(),
            limits.clone(),
            metrics.clone(),
            Arc::new(lr_router::FreeTierManager::new(None)),
        ));
        let clients = Arc::new(lr_clients::ClientManager::with_keychain(
            vec![client],
            Arc::new(lr_api_keys::MockKeychain::new()),
        ));
        let state = AppState::new(
            router,
            limits,
            providers,
            config,
            clients,
            Arc::new(lr_clients::TokenStore::new()),
            metrics,
            None,
        );
        *state.secret_scanner.write() = Some(Arc::new(
            lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
                entropy_threshold: 3.0,
                allowlist: vec![],
                scan_system_messages: false,
            })
            .unwrap(),
        ));
        state
    }
    #[tokio::test]
    async fn secret_placeholders_http_boundary_hides_and_restores_without_prompt_injection() {
        let root = std::env::temp_dir().join(format!(
            "localrouter-redaction-boundary-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let state = state(&root);
        let id = state.config_manager.get().clients[0].id.clone();
        let store: Arc<dyn lr_api_keys::KeychainStorage> =
            Arc::new(lr_api_keys::MockKeychain::new());
        let test_store = store.clone();
        let app=axum::Router::new().route("/v1/chat/completions",post(|Json(request):Json<Value>|async move{
            let outgoing=request.to_string();assert!(!outgoing.contains(SYNTHETIC));assert!(outgoing.contains("localrouter-redacted"));assert_eq!(request["messages"].as_array().unwrap().len(),2);assert_eq!(request["messages"][0]["content"],"Keep this system message unchanged");
            let content=request["messages"][1]["content"].as_str().unwrap().to_owned();Json(serde_json::json!({"id":"chatcmpl-test","choices":[{"message":{"role":"assistant","content":content}}]}))
        })).layer(axum::middleware::from_fn_with_state(state,move |State(state):State<AppState>,request:Request,next:Next|{
            let storage=test_store.clone();async move{match run(state,request,next,storage).await{Ok(response)=>response,Err(error)=>error.into_response()}}
        })).layer(Extension(AuthContext{api_key_id:id,credential_scope:None,model_selection:None}));
        let body = serde_json::json!({"model":"fake/model","messages":[{"role":"system","content":"Keep this system message unchanged"},{"role":"user","content":format!("aws_secret_access_key={SYNTHETIC}")}],"extensions":{"localrouter.session":{"id":"synthetic-boundary"}}});
        let request = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
        let restored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            restored["choices"][0]["message"]["content"],
            format!("aws_secret_access_key={SYNTHETIC}")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use base64::Engine;
    #[test]
    fn secret_placeholders_protect_decoded_chat_and_responses_attachments() {
        for key in ["messages", "input"] {
            let secret = "synthetic attachment secret";
            let vault = Vault::new(
                Arc::new(lr_api_keys::MockKeychain::new()),
                "fixture",
                key,
                false,
            );
            vault.register(secret).unwrap();
            let data = format!(
                "data:text/plain;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(secret)
            );
            let part = if key == "messages" {
                serde_json::json!({"type":"file","file":{"filename":"config.txt","file_data":data}})
            } else {
                serde_json::json!({"type":"input_file","filename":"config.txt","file_data":data})
            };
            let mut request = serde_json::json!({key:[{"role":"user","content":[part]}]});
            let (projected, opaque) = crate::documents::scan_projection(&request).unwrap();
            assert!(!opaque);
            assert!(projected.to_string().contains(secret));
            crate::documents::protect_text_files(&mut request, &vault).unwrap();
            let (protected, _) = crate::documents::scan_projection(&request).unwrap();
            assert!(!protected.to_string().contains(secret));
            assert!(protected.to_string().contains("localrouter-redacted"));
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn secret_placeholders_use_the_existing_session_header_precedence() {
        let value = serde_json::json!({"extensions":{"localrouter.session":{"id":"body-scope"}}});
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            request_session(&value, &headers).unwrap().as_deref(),
            Some("body-scope")
        );
        headers.insert(
            "x-session-id",
            axum::http::HeaderValue::from_static("header-scope"),
        );
        assert_eq!(
            request_session(&value, &headers).unwrap().as_deref(),
            Some("header-scope")
        );
        headers.insert("x-session-id", axum::http::HeaderValue::from_static(""));
        assert!(request_session(&value, &headers).is_err());
    }
}
