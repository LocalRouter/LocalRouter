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
use lr_secret_scanner::{
    placeholders::Vault,
    wire::{frame_end, ResponseRestorer},
};
use lr_types::secret_context::{self, SecretContext};
use serde_json::Value;
use std::sync::Arc;
const LIMIT: usize = 16 * 1024 * 1024; // Main API body limit.

pub async fn middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    match run(
        state,
        request,
        next,
        lr_api_keys::keychain_trait::redacted_secret_storage(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}
struct MonitorContext {
    request_id: String,
    conversation_id: Option<String>,
}
tokio::task_local! { static MONITOR_CONTEXT: std::cell::RefCell<MonitorContext>; }
pub(crate) fn monitor_request_session() -> String {
    MONITOR_CONTEXT
        .try_with(|context| context.borrow().request_id.clone())
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
}
pub(crate) fn monitor_conversation() -> Option<String> {
    MONITOR_CONTEXT
        .try_with(|context| context.borrow().conversation_id.clone())
        .ok()
        .flatten()
}
async fn run(
    state: AppState,
    request: Request,
    next: Next,
    storage: Arc<dyn lr_api_keys::KeychainStorage>,
) -> Result<Response, ApiErrorResponse> {
    let context = MonitorContext {
        request_id: uuid::Uuid::new_v4().to_string(),
        conversation_id: None,
    };
    MONITOR_CONTEXT
        .scope(
            std::cell::RefCell::new(context),
            run_scoped(state, request, next, storage),
        )
        .await
}
async fn run_scoped(
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
    let masking = matches!(
        action,
        lr_config::SecretScanAction::Redact | lr_config::SecretScanAction::Ask
    );
    let resource = lr_secret_scanner::wire::response_id(&path);
    let control = resource.is_some()
        && (matches!(
            *request.method(),
            axum::http::Method::GET | axum::http::Method::DELETE
        ) || (request.method() == axum::http::Method::POST && path.ends_with("/cancel")));
    let inference = request.method() == axum::http::Method::POST
        && matches!(
            path.as_str(),
            "/chat/completions"
                | "/completions"
                | "/responses"
                | "/messages"
                | "/systemone"
                | "/decisions"
        );
    if !inference && !control {
        if *action == lr_config::SecretScanAction::Redact
            && request.method() == axum::http::Method::POST
        {
            return Err(ApiErrorResponse::bad_request(
                "Automatic secret redaction is supported on text gateway endpoints only",
            ));
        }
        return Ok(next.run(request).await);
    }
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, LIMIT)
        .await
        .map_err(|_| ApiErrorResponse::bad_request("Could not read protected request"))?;
    let mut value: Value = if control {
        serde_json::json!({})
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) if !masking => {
                return Ok(next
                    .run(Request::from_parts(parts, Body::from(bytes)))
                    .await)
            }
            Err(_) => return Err(ApiErrorResponse::bad_request("Invalid request JSON")),
        }
    };
    let conversation = match request_session(&value, &parts.headers) {
        Ok(session) => session,
        Err(_) if !masking || control => None,
        Err(error) => return Err(error),
    };
    let _ = MONITOR_CONTEXT.try_with(|context| context.borrow_mut().conversation_id = conversation);
    let continuation = match Vault::find_client(
        storage.clone(),
        &owner,
        lr_secret_scanner::placeholders::RETENTION_SECS,
    ) {
        Ok(vault) => vault,
        Err(_) if !masking || (control && parts.method != axum::http::Method::GET) => {
            return Ok(next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await)
        }
        Err(error) => return Err(ApiErrorResponse::from(error)),
    };
    if !masking && continuation.is_none() {
        return Ok(next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await);
    }
    let session = monitor_request_session();
    let vault = Arc::new(continuation.unwrap_or_else(|| Vault::for_client(storage, &owner)));
    attach_monitor(&state, &vault, &owner, Some(&session));
    if !masking || control {
        if !masking {
            vault.set_restore_only();
        }
        if !control {
            vault
                .set_request_contract(&value)
                .map_err(ApiErrorResponse::from)?;
        }
        let context: Arc<dyn SecretContext> = vault.clone();
        return secret_context::scope(Some(context), async move {
            let body = if control {
                Body::empty()
            } else {
                Body::from(bytes)
            };
            let response = next.run(Request::from_parts(parts, body)).await;
            restore_response(response, vault).await
        })
        .await;
    }
    let context: Arc<dyn SecretContext> = vault.clone();
    secret_context::scope(Some(context.clone()), async {
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let (projection, _) =
            crate::documents::scan_projection(&value).map_err(ApiErrorResponse::from)?;
        match crate::routes::pipeline::scan_request_for_secrets_with_source(
            &state,
            &owner,
            &model,
            &projection,
            None,
            Some(&session),
        )
        .await
        {
            crate::routes::pipeline::SecretScanOutcome::Deny(message) => {
                return Err(ApiErrorResponse::forbidden(message))
            }
            crate::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                crate::documents::redact_text_files(&mut value, &redaction)
                    .map_err(ApiErrorResponse::from)?;
                redaction.apply(&mut value);
            }
            _ => {}
        }
        crate::documents::protect_text_files(&mut value, context.as_ref())
            .map_err(ApiErrorResponse::from)?;
        context
            .protect(&mut value)
            .map_err(ApiErrorResponse::from)?;
        vault
            .set_request_contract(&value)
            .map_err(ApiErrorResponse::from)?;
        context.mark_scanned();
        // Subsequent automatic tool iterations can introduce new credentials.
        // Reuse the exact scanner/exceptions rather than exposing them to the provider.
        install_followup_detector(
            state.clone(),
            owner.clone(),
            &vault,
            *action == lr_config::SecretScanAction::Redact,
            &value,
        );
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        let body = serde_json::to_vec(&value)
            .map_err(|_| ApiErrorResponse::internal_error("Could not encode protected request"))?;
        let response = next.run(Request::from_parts(parts, Body::from(body))).await;
        restore_response(response, vault).await
    })
    .await
}
/// Request-local lifecycle events use metadata only. Vault callbacks run after
/// releasing credential locks, so Monitor sanitization can safely read the map.
pub fn attach_monitor(state: &AppState, vault: &Vault, owner: &str, session: Option<&str>) {
    let store = state.monitor_store.clone();
    let after_sequence = store
        .list(0, 1, None)
        .events
        .first()
        .map_or(0, |event| event.sequence);
    let owner = owner.to_owned();
    let name = state
        .config_manager
        .get()
        .clients
        .iter()
        .find(|c| c.id == owner)
        .map(|c| c.name.clone());
    let session = session.map(str::to_owned);
    let events = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
        &'static str,
        String,
    >::new()));
    vault.set_operation_observer(Arc::new(move |operation| {
        use lr_monitor::{EventStatus, MonitorEventData, MonitorEventType};
        let Ok(mut events) = events.lock() else { return; };
        let tag = format!("<localrouter-redacted id='{}' usage='Unmasked when returned verbatim'/>", operation.reference_id);
        let existing = events.get(operation.action).and_then(|id| store.get(id))
            .or_else(|| if operation.action == "redact" { session.as_deref().and_then(|s| store.redaction_for_request(&owner, s, after_sequence)) } else { None });
        let mut findings = existing.as_ref().and_then(|event| {
            if let MonitorEventData::SecretScan { findings, .. } = &event.data { findings.clone() } else { None }
        }).and_then(|findings| findings.as_array().cloned()).unwrap_or_default();
        let matched = findings.iter_mut().find(|finding| {
            (finding["reference_id"].as_str() == Some(&operation.reference_id) || finding["replacement"].as_str().is_some_and(|tag| tag.to_ascii_lowercase().contains(&operation.reference_id))) &&
            finding["operation_location"].as_str().is_none_or(|location| location == operation.location)
        });
        if let Some(finding) = matched {
            let count = finding["occurrences"].as_u64().unwrap_or_default();
            finding["occurrences"] = serde_json::json!(count.saturating_add(operation.occurrences as u64));
            finding["reference_id"] = operation.reference_id.clone().into();
            finding["operation_location"] = operation.location.clone().into();
        } else {
            findings.push(serde_json::json!({"rule_id":"stored-secret-reference", "rule_description":"Stored secret", "category":"Secret Scan", "matched_text":"Secret held locally", "message_index":0, "value":tag, "redacted":true, "replacement":tag, "reference_id":operation.reference_id, "action":if operation.action == "restore" { "restored" } else { "redacted" }, "location":operation.location, "operation_location":operation.location, "occurrences":operation.occurrences}));
        }
        let count = findings.len();
        let findings = serde_json::Value::Array(findings);
        let updated = existing.as_ref().is_some_and(|event| store.update(&event.id, |event| {
            if let MonitorEventData::SecretScan { findings: stored, findings_count, .. } = &mut event.data { *stored = Some(findings.clone()); *findings_count = Some(count); }
        }));
        let id = if updated { existing.expect("updated event").id } else {
            store.push(MonitorEventType::SecretScan, Some(owner.clone()), name.clone(), session.clone(),
                MonitorEventData::SecretScan { text_preview:tag, rules_count:0, findings_count:Some(count), findings:Some(findings), action_taken:Some(operation.action.into()), latency_ms:None }, EventStatus::Complete, None)
        };
        events.insert(operation.action, id);
    }));
}

fn install_followup_detector(
    state: AppState,
    owner: String,
    vault: &Arc<Vault>,
    automatic: bool,
    value: &Value,
) {
    let parent_session = monitor_request_session();
    let scan_state = state;
    let scan_owner = owner;
    let allowed_plaintext = scan_state
        .secret_scanner
        .read()
        .as_ref()
        .map(|scanner| {
            let texts = lr_guardrails::text_extractor::extract_request_text(value)
                .into_iter()
                .enumerate()
                .map(|(i, t)| lr_secret_scanner::ExtractedText {
                    label: t.label,
                    text: t.text,
                    message_index: i,
                })
                .collect::<Vec<_>>();
            scanner
                .scan(&texts)
                .findings
                .into_iter()
                .map(|f| f.raw_text)
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let allowed_plaintext = Arc::new(parking_lot::Mutex::new(allowed_plaintext));
    vault.set_async_detector(Arc::new(move |mut body| {
        let state = scan_state.clone();
        let owner = scan_owner.clone();
        let session = parent_session.clone();
        let allowed = allowed_plaintext.clone();
        Box::pin(async move {
            let mut strings = Vec::new();
            let (mut scan, _) = crate::documents::scan_projection(&body)?;
            lr_secret_scanner::placeholders::visit(
                &mut scan,
                &mut |text, _| {
                    strings.push(text.to_owned());
                    Ok(text.to_owned())
                },
                false,
            )?;
            let mut projection =
                serde_json::json!({"messages":[{"role":"tool","content":strings.join("\n")} ]});
            if !automatic {
                let mut ignored = lr_secret_scanner::redaction::Redaction::default();
                for text in allowed.lock().iter() {
                    ignored.add(text);
                }
                ignored.apply(&mut projection);
            }
            match crate::routes::pipeline::scan_request_for_secrets_fresh(
                &state,
                &owner,
                "",
                &projection,
                None,
                Some(&session),
            )
            .await
            {
                crate::routes::pipeline::SecretScanOutcome::Deny(message) => {
                    return Err(lr_types::AppError::InvalidParams(message))
                }
                crate::routes::pipeline::SecretScanOutcome::Redact(redaction) => {
                    crate::documents::redact_text_files(&mut body, &redaction)?;
                    redaction.apply(&mut body)
                }
                _ => {
                    if !automatic {
                        if let Some(scanner) = state.secret_scanner.read().clone() {
                            let result = scanner.scan(&[lr_secret_scanner::ExtractedText {
                                label: "tool".into(),
                                text: strings.join("\n"),
                                message_index: 0,
                            }]);
                            allowed.lock().extend(
                                result.findings.into_iter().map(|finding| finding.raw_text),
                            );
                        }
                    }
                }
            }
            if let Some(context) = secret_context::current() {
                crate::documents::protect_text_files(&mut body, context.as_ref())?;
            }
            Ok(body)
        })
    }));
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
    vault: Arc<Vault>,
) -> Result<Response, ApiErrorResponse> {
    let context: Arc<dyn SecretContext> = vault.clone();
    let streaming = response
        .headers()
        .get("content-type")
        .is_some_and(|v| v.as_bytes().starts_with(b"text/event-stream"));
    let (mut parts, body) = response.into_parts();
    for header in [
        "content-length",
        "etag",
        "content-md5",
        "digest",
        "content-digest",
        "trailer",
    ] {
        parts.headers.remove(header);
    }
    parts.headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if !streaming {
        let bytes = to_bytes(body, LIMIT)
            .await
            .map_err(|_| ApiErrorResponse::internal_error("Protected response exceeds limit"))?;
        let mut codec = ResponseRestorer::new(vault);
        let encoded = if bytes.is_empty()
            && matches!(
                parts.status,
                axum::http::StatusCode::NO_CONTENT | axum::http::StatusCode::NOT_MODIFIED
            ) {
            bytes.to_vec()
        } else {
            codec.json(&bytes).map_err(ApiErrorResponse::from)?
        };
        return Ok(Response::from_parts(parts, Body::from(encoded)));
    }
    let stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send>,
    > = Box::pin(async_stream::try_stream! {
        let mut body=body.into_data_stream();let mut pending=Vec::new();let mut scanned=0;let mut codec=ResponseRestorer::new(vault);
        while let Some(chunk)=secret_context::scope(Some(context.clone()),body.next()).await {
            let chunk=chunk.map_err(std::io::Error::other)?;pending.extend_from_slice(&chunk);
            if pending.len()>LIMIT {Err(std::io::Error::other("Protected response frame exceeds limit"))?;}
            while let Some((relative,separator))=frame_end(&pending[scanned..]){
                let end=scanned+relative;scanned=0;
                let frame=String::from_utf8(pending.drain(..end).collect()).map_err(std::io::Error::other)?;
                pending.drain(..separator);
                for event in codec.frame(&frame).map_err(std::io::Error::other)? {yield axum::body::Bytes::from(event);}
            }
            scanned=pending.len().saturating_sub(3);
        }
        if !pending.is_empty(){Err(std::io::Error::other("Incomplete protected response event"))?;}
        for event in codec.finish().map_err(std::io::Error::other)? {yield axum::body::Bytes::from(event);}
    });
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}
#[cfg(test)]
mod tests {
    use super::*;
    use lr_secret_scanner::wire::Events;
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
    async fn empty_resource_responses_keep_their_status_and_body() {
        for status in [204, 304, 200] {
            let vault = Arc::new(Vault::new(
                Arc::new(lr_api_keys::MockKeychain::new()),
                "empty-resource",
                "scope",
                false,
            ));
            let response = Response::builder()
                .status(status)
                .body(Body::empty())
                .unwrap();
            let result = restore_response(response, vault).await;
            if status == 200 {
                assert!(result.is_err(), "empty inference responses remain invalid");
            } else {
                let response = result.unwrap();
                assert_eq!(response.status().as_u16(), status);
                assert!(to_bytes(response.into_body(), LIMIT)
                    .await
                    .unwrap()
                    .is_empty());
            }
        }
    }
    #[tokio::test]
    async fn secret_placeholders_json_and_fragmented_sse_body() {
        let (_, _, secret) = fixture();
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "client",
            "other",
            false,
        ));
        vault.register(&secret).unwrap();
        let mut protected = json!({"content":secret});
        vault.protect(&mut protected).unwrap();
        let tag = protected["content"].as_str().unwrap().to_owned();
        let json_response = Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(json!({"content":tag}).to_string()))
            .unwrap();
        let result = restore_response(json_response, vault.clone())
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
        let result = restore_response(response, vault).await.unwrap();
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
    pub(super) fn state(root: &std::path::Path) -> AppState {
        let mut config = AppConfig::default();
        config.secret_scanning.action = lr_config::SecretScanAction::Redact;
        let client =
            lr_config::Client::new_with_strategy("synthetic client".into(), "strategy".into());
        config.clients = vec![client.clone()];
        let mut strategy = lr_config::Strategy::new("Synthetic".into());
        strategy.id = "strategy".into();
        config.strategies = vec![strategy];
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
    async fn gateway_redaction_and_restoration_events_share_requests_without_merging_turns() {
        let root = std::env::temp_dir().join(format!("lr-phase-events-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let state = state(&root);
        let id = state.config_manager.get().clients[0].id.clone();
        let monitor = state.monitor_store.clone();
        let storage: Arc<dyn lr_api_keys::KeychainStorage> =
            Arc::new(lr_api_keys::MockKeychain::new());
        let handler_state = state.clone();
        let handler_id = id.clone();
        let boundary_storage = storage.clone();
        let app = axum::Router::new().route("/v1/chat/completions", post(move |Json(request):Json<Value>| {
            let state = handler_state.clone(); let owner = handler_id.clone();
            async move {
                assert!(!request.to_string().contains(SYNTHETIC));
                let session = crate::routes::monitor_helpers::new_request_session();
                let auth = Extension(crate::middleware::client_auth::ClientAuthContext { client_id:owner });
                let guard = crate::routes::monitor_helpers::emit_llm_call(&state, Some(&auth), Some(&session), "/v1/chat/completions", "fake/model", request["stream"] == true, &request);
                let event = guard.into_event_id();
                let text = request["messages"][0]["content"].as_str().unwrap();
                let body = if request["stream"] == true {
                    let mut wire = String::new();
                    for part in text.as_bytes().chunks(7) {
                        wire.push_str(&format!("data: {}\n\n",serde_json::json!({"choices":[{"index":0,"delta":{"content":std::str::from_utf8(part).unwrap()}}]})));
                    }
                    wire.push_str("data: [DONE]\n\n");
                    Response::builder().header("content-type","text/event-stream").body(Body::from(wire)).unwrap()
                } else {
                    let response = serde_json::json!({"choices":[{"message":{"role":"assistant","content":text,"tool_calls":[{"id":"call","type":"function","function":{"name":"write","arguments":serde_json::json!({"content":text}).to_string()}}]}}]});
                    Response::builder().header("content-type","application/json").body(Body::from(response.to_string())).unwrap()
                };
                state.monitor_store.update(&event, |event| event.status = lr_monitor::EventStatus::Complete);
                body
            }
        })).layer(axum::middleware::from_fn_with_state(state,move |State(state):State<AppState>,request:Request,next:Next| {
            let storage = boundary_storage.clone(); async move { match run(state,request,next,storage).await { Ok(response)=>response, Err(error)=>error.into_response() } }
        })).layer(Extension(AuthContext { api_key_id:id.clone(), credential_scope:None, model_selection:None }));
        for stream in [false, true] {
            let request = serde_json::json!({"model":"fake/model","stream":stream,"messages":[{"role":"user","content":format!("aws_secret_access_key={SYNTHETIC}")}], "extensions":{"localrouter.session":{"id":"one-conversation"}}, "tools":[{"type":"function","function":{"name":"write","parameters":{"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}}}]});
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/chat/completions")
                        .method("POST")
                        .header("content-type", "application/json")
                        .body(Body::from(request.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains(SYNTHETIC));
        }
        let events: Vec<_> = monitor
            .list(0, 100, None)
            .events
            .iter()
            .map(|event| monitor.get(&event.id).unwrap())
            .collect();
        assert_eq!(
            events.len(),
            6,
            "one redaction, LLM and restoration event per turn"
        );
        let sessions: std::collections::HashSet<_> = events
            .iter()
            .map(|event| event.session_id.clone().unwrap())
            .collect();
        assert_eq!(sessions.len(), 2);
        let conversations: std::collections::HashSet<_> = events
            .iter()
            .map(|event| event.conversation.as_ref().unwrap().id.clone())
            .collect();
        assert_eq!(conversations.len(), 1);
        for session in sessions {
            let request: Vec<_> = events
                .iter()
                .filter(|event| event.session_id.as_deref() == Some(&session))
                .collect();
            assert_eq!(request.len(), 3);
            let actions: Vec<_> = request
                .iter()
                .filter_map(|event| lr_monitor::to_summary(event).secret_action)
                .collect();
            assert!(actions.iter().any(|action| action == "redact"));
            assert!(actions.iter().any(|action| action == "restore"));
        }
        assert!(!serde_json::to_string(&events).unwrap().contains(SYNTHETIC));
        assert!(events.iter().any(|event| serde_json::to_string(event)
            .unwrap()
            .contains("Tool arguments")));
        std::fs::remove_dir_all(root).unwrap();
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
    #[tokio::test]
    async fn gateway_untracked_then_opaque_continuation_restores_json_and_sse() {
        for streaming in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let state = state(root.path());
            let owner = state.config_manager.get().clients[0].id.clone();
            let storage = Arc::new(lr_api_keys::MockKeychain::new());
            let stored = storage.clone();
            let marker = Arc::new(std::sync::Mutex::new(None::<String>));
            let app=axum::Router::new().route("/v1/responses",post(move |Json(body):Json<Value>|{
                let marker=marker.clone();async move {
                    assert!(!body.to_string().contains(SYNTHETIC),"gateway must dispatch only placeholders");
                    if let Some(text)=body.pointer("/input/0/content").and_then(Value::as_str) {
                        *marker.lock().unwrap()=Some(text.split_once('=').unwrap().1.into());
                    }
                    let tag=marker.lock().unwrap().clone().unwrap();
                    if streaming {
                        let events=[serde_json::json!({"type":"response.created","response":{"id":"opaque-gateway-id"}}),serde_json::json!({"type":"response.output_item.done","item":{"type":"message","role":"assistant","content":[{"type":"output_text","text":tag}]}}),serde_json::json!({"type":"response.completed","response":{"id":"opaque-gateway-id"}})];
                        Response::builder().header("content-type","text/event-stream").body(Body::from(events.iter().map(|event|format!("data: {event}\n\n")).collect::<String>())).unwrap()
                    } else {
                        Json(serde_json::json!({"object":"response","id":"opaque-gateway-id","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":tag}]}]})).into_response()
                    }
                }
            })).layer(axum::middleware::from_fn_with_state(state,move |State(state):State<AppState>,request:Request,next:Next| {
                let storage=stored.clone();async move {match run(state,request,next,storage).await {Ok(response)=>response,Err(error)=>error.into_response()}}
            })).layer(Extension(AuthContext{api_key_id:owner,credential_scope:None,model_selection:None}));
            for first in [true, false] {
                let input = if first {
                    serde_json::json!([{"role":"user","content":format!("aws_secret_access_key={SYNTHETIC}")}])
                } else {
                    serde_json::json!([{"type":"function_call_output","call_id":"call-1","output":"done without the original"}])
                };
                let request = serde_json::json!({"model":"local-test","stream":streaming,"previous_response_id":if first {"opaque-before-redaction"} else {"opaque-gateway-id"},"input":input,"tools":[{"type":"web_search"},{"type":"function","name":"write","parameters":{"type":"object"}}]});
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/v1/responses")
                            .header("content-type", "application/json")
                            .body(Body::from(request.to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
                let text = String::from_utf8(bytes.to_vec()).unwrap();
                assert_eq!(status, 200, "streaming={streaming}, first={first}: {text}");
                assert!(
                    text.contains(SYNTHETIC),
                    "continuation must recover its owned native-store scope"
                );
                assert!(!text.contains("localrouter-redacted"));
            }
        }
    }
    #[tokio::test]
    async fn gateway_resource_reads_cancel_and_delete_restore_the_owned_scope_after_mode_changes() {
        use axum::routing::get;
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let manager = state.config_manager.clone();
        let owner = manager.get().clients[0].id.clone();
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Arc::new(Vault::new(storage.clone(), &owner, "resource-scope", false));
        vault.register(SYNTHETIC).unwrap();
        let mut tag = Value::String(SYNTHETIC.into());
        vault.protect(&mut tag).unwrap();
        let mut codec = ResponseRestorer::new(vault);
        codec
            .json(
                serde_json::json!({"object":"response","id":"opaque/id:ABC","output":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        drop(codec);
        let stored = storage.clone();
        let handler = move || {
            let tag = tag.clone();
            async move {
                Json(
                    serde_json::json!({"object":"response","id":"opaque/id:ABC","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":tag}]}]}),
                )
            }
        };
        let app = axum::Router::new()
            .route(
                "/v1/responses/{id}",
                get(handler.clone()).delete(handler.clone()),
            )
            .route("/v1/responses/{id}/input_items", get(handler.clone()))
            .route("/v1/responses/{id}/cancel", post(handler))
            .layer(axum::middleware::from_fn_with_state(
                state,
                move |State(state): State<AppState>, request: Request, next: Next| {
                    let storage = stored.clone();
                    async move {
                        match run(state, request, next, storage).await {
                            Ok(response) => response,
                            Err(error) => error.into_response(),
                        }
                    }
                },
            ))
            .layer(Extension(AuthContext {
                api_key_id: owner,
                credential_scope: None,
                model_selection: None,
            }));
        for action in [
            lr_config::SecretScanAction::Redact,
            lr_config::SecretScanAction::Notify,
            lr_config::SecretScanAction::Off,
        ] {
            manager
                .update(|c| c.secret_scanning.action = action)
                .unwrap();
            for (method, suffix) in [
                ("GET", ""),
                ("GET", "/input_items"),
                ("POST", "/cancel"),
                ("DELETE", ""),
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(format!("/v1/responses/opaque%2Fid%3AABC{suffix}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), 200);
                let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&bytes).contains(SYNTHETIC),
                    "{method} {suffix} must restore only its owned scope"
                );
            }
        }
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

#[cfg(test)]
mod executable_boundary_tests {
    use super::*;
    use lr_mcp::{
        gateway::{access_control::FirewallCheckResult, virtual_server::*, FirewallDecisionResult},
        protocol::{JsonRpcRequest, McpTool},
    };
    use serde_json::json;
    use std::{any::Any, path::PathBuf, sync::Mutex};
    #[derive(Clone)]
    struct ToolState;
    impl VirtualSessionState for ToolState {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn VirtualSessionState> {
            Box::new(self.clone())
        }
    }
    struct FileTool {
        file: PathBuf,
        edit: Value,
        seen: Arc<Mutex<Vec<Value>>>,
        deny_effective: bool,
    }
    #[async_trait::async_trait]
    impl VirtualMcpServer for FileTool {
        fn id(&self) -> &str {
            "_secret_fixture"
        }
        fn display_name(&self) -> &str {
            "Synthetic file tool"
        }
        fn owns_tool(&self, name: &str) -> bool {
            name == "fixture_write"
        }
        fn is_enabled(&self, _: &lr_config::Client) -> bool {
            true
        }
        fn list_tools(&self, _: &dyn VirtualSessionState) -> Vec<McpTool> {
            vec![McpTool {
                name: "fixture_write".into(),
                description: None,
                input_schema: json!({"type":"object","required":["content"],"properties":{"content":{"type":"string"}},"additionalProperties":false}),
            }]
        }
        fn check_permissions(
            &self,
            _: &dyn VirtualSessionState,
            _: &str,
            args: Option<&Value>,
            _: bool,
            _: bool,
        ) -> VirtualFirewallResult {
            let args = args.cloned().unwrap_or(Value::Null);
            self.seen.lock().unwrap().push(args.clone());
            if self.deny_effective && args["content"] == "synthetic file secret" {
                VirtualFirewallResult::Standard(FirewallCheckResult::Deny)
            } else {
                VirtualFirewallResult::Handled(FirewallDecisionResult::ProceedWithEdits {
                    edited_arguments: Some(self.edit.clone()),
                })
            }
        }
        async fn handle_tool_call(
            &self,
            _: Box<dyn VirtualSessionState>,
            _: &str,
            args: Value,
            _: &str,
            _: &str,
        ) -> VirtualToolCallResult {
            std::fs::write(&self.file, args["content"].as_str().unwrap()).unwrap();
            VirtualToolCallResult::Success(
                json!({"content":[{"type":"text","text":args["content"].as_str().unwrap().repeat(512)}]}),
            )
        }
        fn build_instructions(&self, _: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
            None
        }
        fn create_session_state(&self, _: &lr_config::Client) -> Box<dyn VirtualSessionState> {
            Box::new(ToolState)
        }
        fn update_session_state(&self, _: &mut dyn VirtualSessionState, _: &lr_config::Client) {}
        fn all_tool_names(&self) -> Vec<String> {
            vec!["fixture_write".into()]
        }
    }
    async fn case(deny: bool, invalid: bool) {
        let root =
            std::env::temp_dir().join(format!("lr-secret-executable-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let state = super::boundary_tests::state(&root);
        let owner = state.config_manager.get().clients[0].id.clone();
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &owner,
            "file",
            true,
        ));
        vault.register("synthetic file secret").unwrap();
        let mut edit = json!({"content":"synthetic file secret"});
        vault.protect(&mut edit).unwrap();
        if invalid {
            edit["content"] = json!(123);
        }
        let file = root.join(".env");
        let seen = Arc::new(Mutex::new(Vec::new()));
        state
            .mcp_gateway
            .register_virtual_server(Arc::new(FileTool {
                file: file.clone(),
                edit,
                seen: seen.clone(),
                deny_effective: deny,
            }));
        let context: Arc<dyn SecretContext> = vault;
        state.mcp_gateway.register_virtual_server(Arc::new(
            lr_mcp::gateway::context_mode::ContextModeVirtualServer::new(
                lr_config::ContextManagementConfig {
                    response_threshold_bytes: 512,
                    catalog_compression: false,
                    ..Default::default()
                },
            ),
        ));
        let result=secret_context::scope(Some(context),async {
            let init=JsonRpcRequest::new(Some(json!(1)),"initialize".into(),Some(json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}})));
            state.mcp_gateway.handle_request(&owner,vec![],vec![],init).await.unwrap();
            let call=JsonRpcRequest::new(Some(json!(2)),"tools/call".into(),Some(json!({"name":"fixture_write","arguments":{"content":"initial harmless content"}})));
            state.mcp_gateway.handle_request(&owner,vec![],vec![],call).await
        }).await;
        if deny || invalid {
            assert!(result.is_err() || result.unwrap().error.is_some());
            assert!(!file.exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                "synthetic file secret"
            );
            assert!(seen
                .lock()
                .unwrap()
                .iter()
                .any(|a| a["content"] == "synthetic file secret"));
            let response = serde_json::to_string(&result.unwrap()).unwrap();
            assert!(!response.contains("synthetic file secret"));
            assert!(response.contains("localrouter-redacted"));
            let session = state.mcp_gateway.get_session(&owner).unwrap();
            let store = {
                let session = session.read().await;
                session
                    .virtual_server_state
                    .get("_context_mode")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<lr_mcp::gateway::context_mode::ContextModeSessionState>()
                    .unwrap()
                    .store
                    .clone()
            };
            let sources = store.list_sources(None, None).unwrap();
            assert!(
                !sources.is_empty(),
                "The response indexing path must actually run"
            );
            for source in sources {
                let mut offset = None;
                loop {
                    let read = store
                        .read(&source.label, offset.as_deref(), Some(usize::MAX))
                        .unwrap();
                    assert!(!read.content.contains("synthetic file secret"));
                    let Some(next) = read.next_offset else {
                        break;
                    };
                    offset = Some(next);
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn edited_placeholder_is_restored_validated_and_written_by_actual_mcp_dispatch() {
        case(false, false).await;
    }
    #[tokio::test]
    async fn effective_permission_denial_prevents_file_write() {
        case(true, false).await;
    }
    #[tokio::test]
    async fn invalid_edited_schema_prevents_file_write() {
        case(false, true).await;
    }
}

#[cfg(test)]
mod followup_approval_tests {
    use super::*;
    use lr_mcp::gateway::firewall::FirewallApprovalAction;
    use serde_json::json;
    async fn case(action: FirewallApprovalAction) {
        let root =
            std::env::temp_dir().join(format!("lr-followup-approval-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let state = super::boundary_tests::state(&root);
        state
            .config_manager
            .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Ask)
            .unwrap();
        let owner = state.config_manager.get().clients[0].id.clone();
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &owner,
            "followup",
            true,
        ));
        install_followup_detector(
            state.clone(),
            owner,
            &vault,
            false,
            &json!({"messages":[{"role":"user","content":"harmless"}]}),
        );
        vault.mark_scanned();
        let context: Arc<dyn SecretContext> = vault;
        let allowed = matches!(action, FirewallApprovalAction::AllowOnce);
        let denied = matches!(action, FirewallApprovalAction::Deny);
        let task = tokio::spawn(async move {
            secret_context::scope(Some(context.clone()),async move{
            let mut body=json!({"content":[{"type":"text","text":"aws_secret_access_key=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr"}]});
            context.prepare(&mut body).await?;
            if allowed {tokio::time::timeout(std::time::Duration::from_secs(2),context.prepare(&mut body)).await.map_err(|_|lr_types::AppError::Internal("Duplicate secret approval".into()))??;}
            Ok::<_,lr_types::AppError>(body)
        }).await
        });
        let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(pending) = state
                    .mcp_gateway
                    .firewall_manager
                    .list_pending()
                    .into_iter()
                    .next()
                {
                    break pending;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(pending.is_secret_scan_request);
        assert!(!serde_json::to_string(&pending)
            .unwrap()
            .contains("6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr"));
        state
            .mcp_gateway
            .firewall_manager
            .submit_response(&pending.request_id, action, None)
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        if denied {
            assert!(result.is_err());
        } else if allowed {
            assert!(result
                .unwrap()
                .to_string()
                .contains("6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr"));
        } else {
            assert!(result.unwrap().to_string().contains("localrouter-redacted"));
        }
        assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn new_tool_secret_opens_real_approval_and_can_use_placeholder() {
        case(FirewallApprovalAction::ReplaceSecrets).await;
    }
    #[tokio::test]
    async fn new_tool_secret_denial_stops_dispatch() {
        case(FirewallApprovalAction::Deny).await;
    }
    #[tokio::test]
    async fn allow_once_does_not_ask_twice_within_one_request() {
        case(FirewallApprovalAction::AllowOnce).await;
    }
}

#[cfg(test)]
mod followup_attachment_tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;
    #[tokio::test]
    async fn new_text_attachment_is_decoded_scanned_and_masked_before_dispatch() {
        let root = std::env::temp_dir().join(format!("lr-followup-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let state = super::boundary_tests::state(&root);
        let owner = state.config_manager.get().clients[0].id.clone();
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &owner,
            "file",
            true,
        ));
        install_followup_detector(state, owner, &vault, true, &json!({}));
        let context: Arc<dyn SecretContext> = vault;
        let text = "aws_secret_access_key=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
        let mut body = json!({"messages":[{"role":"tool","content":[{"type":"file","file":{"filename":"tool.env","file_data":format!("data:text/plain;base64,{}",base64::engine::general_purpose::STANDARD.encode(text))}}]}]});
        secret_context::scope(Some(context.clone()), context.prepare(&mut body))
            .await
            .unwrap();
        let (projected, _) = crate::documents::scan_projection(&body).unwrap();
        let projected = projected.to_string();
        assert!(projected.contains("localrouter-redacted"));
        assert!(!projected.contains("6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
