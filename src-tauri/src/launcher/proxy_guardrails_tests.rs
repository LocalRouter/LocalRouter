//! GuardRails over streamed proxy requests: the text projection built while
//! the body arrives gives the safety models exactly what the buffered path
//! gives them, with the same verdicts, popups, monitor events and refusals.

use std::sync::Mutex as StdMutex;

use super::secret_policy_tests::fixture;
use super::*;
use lr_guardrails::{
    FlaggedCategory, InferenceMode, SafetyCategory, SafetyCategoryInfo, SafetyCheckInput,
    SafetyMessage, SafetyModel, SafetyVerdict,
};
use lr_proxy::active::{Firewall, FirewallRequest, StreamRequest};
use lr_proxy::interceptor::{
    RequestAction, StreamMasking, StreamOpen, StreamPolicy, DEFAULT_STREAM_LAG,
};
use lr_proxy::request_stream::{RequestPipeline, StreamAbort, StreamSession, MAX_HOLD};
use serde_json::{json, Value};

/// Records what it was asked and flags any text containing `harmful`.
struct RecordingModel {
    seen: Seen,
}

#[async_trait::async_trait]
impl SafetyModel for RecordingModel {
    fn id(&self) -> &str {
        "recording"
    }
    fn model_type_id(&self) -> &str {
        "recording"
    }
    fn display_name(&self) -> &str {
        "Recording"
    }
    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        Vec::new()
    }
    fn inference_mode(&self) -> InferenceMode {
        InferenceMode::MultiCategory
    }
    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        self.seen.lock().unwrap().push(
            input
                .messages
                .iter()
                .map(|SafetyMessage { role, content }| (role.clone(), content.clone()))
                .collect(),
        );
        let harmful = input.messages.iter().any(|m| m.content.contains("harmful"));
        Ok(SafetyVerdict {
            model_id: "recording".into(),
            model_label: None,
            is_safe: !harmful,
            flagged_categories: if harmful {
                vec![FlaggedCategory {
                    category: SafetyCategory::ViolentCrimes,
                    confidence: Some(0.9),
                    native_label: "S1".into(),
                }]
            } else {
                Vec::new()
            },
            confidence: None,
            raw_output: String::new(),
            check_duration_ms: 0,
        })
    }
}

type Seen = Arc<StdMutex<Vec<Vec<(String, String)>>>>;

/// A firewall with GuardRails on (every category `action`) and the given
/// Secret Scan action.
fn guarded(
    root: &std::path::Path,
    action: &str,
    scan: lr_config::SecretScanAction,
) -> (AppFirewall, FirewallRequest, Seen) {
    let (firewall, base) = fixture(
        root,
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    let seen: Seen = Default::default();
    firewall
        .state
        .replace_safety_engine(Arc::new(lr_guardrails::SafetyEngine::new(
            vec![Arc::new(RecordingModel { seen: seen.clone() })],
            0.5,
        )));
    firewall
        .config_manager
        .update(|config| {
            config.secret_scanning.action = scan;
            config.guardrails.scan_requests = true;
            config.guardrails.category_actions = vec![lr_config::CategoryActionEntry {
                category: "__global".into(),
                action: action.into(),
            }];
        })
        .unwrap();
    (firewall, base, seen)
}

fn masking(scan: &lr_config::SecretScanAction) -> StreamMasking {
    match scan {
        lr_config::SecretScanAction::Redact => StreamMasking::Redact,
        lr_config::SecretScanAction::Notify => StreamMasking::Notify,
        lr_config::SecretScanAction::Off => StreamMasking::Off,
        lr_config::SecretScanAction::Ask => StreamMasking::Ask,
    }
}

fn policy(masking: StreamMasking) -> StreamPolicy {
    StreamPolicy {
        masking,
        needs_model: false,
        whole_document: false,
        lag: DEFAULT_STREAM_LAG,
        restore_only: false,
        strict: false,
    }
}

fn stream_request(base: &FirewallRequest) -> StreamRequest {
    StreamRequest {
        client_id: base.client_id.clone(),
        session_id: base.session_id.clone(),
        secret_session: base.secret_session.clone(),
        host: base.host.clone(),
        method: base.method.clone(),
        path: base.path.clone(),
        provider: base.provider.clone(),
        model: Some("local-test".into()),
        format: lr_proxy::wire::WireFormat::OpenAiResponses,
        preview: Some(lr_proxy::active::StreamPreview {
            text: r#"{"model":"local-test","input":["#.into(),
            received: 40 * 1024 * 1024,
            truncated: true,
        }),
        trace: base.trace.clone(),
        admission: Default::default(),
    }
}

/// Compact JSON with `model` first.
fn encode(body: &Value) -> Vec<u8> {
    let map = body.as_object().unwrap();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| *k != "model");
    let members: Vec<String> = keys
        .iter()
        .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), map[*k]))
        .collect();
    format!("{{{}}}", members.join(",")).into_bytes()
}

fn github_token() -> String {
    format!("{}_{}", "ghp", "aB3dE5gH7jK9mN1pQ3sT5vW7yZ9bC1dF3hJ5")
}

/// Responses and Chat bodies with every kind of text the projection reads,
/// `harmful` (when set) in the middle of the conversation.
fn corpus(harmful: bool) -> Vec<Value> {
    use base64::Engine;
    let filler = "lorem ipsum ".repeat(40);
    let flagged = if harmful {
        "something harmful"
    } else {
        "something fine"
    };
    let attachment = base64::engine::general_purpose::STANDARD
        .encode(format!("notes: {flagged} in a text file"));
    let mut input: Vec<Value> = (0..40)
        .map(|i| json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("{i} {filler}")}]}))
        .collect();
    input[7] =
        json!({"type":"function_call","call_id":"c1","name":"exec","arguments":"{\"cmd\":\"ls\"}"});
    input[8] = json!({"type":"function_call_output","call_id":"c1","output":format!("token {}", github_token())});
    input[20] =
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":flagged}]});
    input[25] = json!({"type":"reasoning","encrypted_content":"opaque","summary":[{"type":"summary_text","text":"why"}]});
    input[30] = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]});
    let mut with_file = input.clone();
    with_file[20] = json!({"type":"message","role":"user","content":[{"type":"input_file","filename":"notes.txt","file_data":format!("data:text/plain;base64,{attachment}")}]});
    let messages: Vec<Value> = (0..40)
        .map(|i| {
            let text = if i == 20 {
                flagged.to_owned()
            } else {
                format!("{i} {filler}")
            };
            json!({"role": if i % 2 == 0 { "user" } else { "assistant" }, "content": text})
        })
        .collect();
    vec![
        json!({
            "model": "local-test",
            "instructions": "Be brief.",
            "input": input,
            "tools": [{"type":"function","name":"exec","parameters":{"type":"object"}}],
            "text": {"format":{"type":"json_schema","name":"out","schema":{"type":"object"}}},
            "stream": true,
        }),
        json!({"model": "local-test", "input": with_file, "stream": true}),
        json!({"model": "local-test", "messages": messages, "system": "sys", "stream": false}),
    ]
}

/// Feed `body` through the session as the driver does, then ask its
/// verdict. `answer` runs each popup that opens meanwhile.
async fn stream_and_judge(
    firewall: &AppFirewall,
    session: &mut Box<dyn StreamSession>,
    bytes: &[u8],
    lag: usize,
    chunk: usize,
) -> (Vec<u8>, Result<(), StreamAbort>) {
    assert!(session.holds_until_verdict());
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut out = Vec::new();
    let mut opened = false;
    for piece in bytes.chunks(chunk) {
        if opened {
            pipeline
                .push(piece, Some(session.as_mut()), &mut out)
                .unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline
                    .push(&[], Some(session.as_mut()), &mut out)
                    .unwrap();
            }
        }
        // Nothing is judged before the end.
        assert!(session.question().is_none());
    }
    if !opened {
        pipeline.open();
    }
    pipeline.finish(session.as_mut(), &mut out).unwrap();
    let mut verdict = Ok(());
    while let Some(question) = session.question() {
        let _ = firewall;
        let answer = question.0.await;
        if let Err(abort) = session.answer(answer) {
            verdict = Err(abort);
            break;
        }
    }
    (out, verdict)
}

async fn open(
    firewall: &AppFirewall,
    base: &FirewallRequest,
    masking: StreamMasking,
) -> Box<dyn StreamSession> {
    match firewall
        .open_stream(&stream_request(base), &policy(masking))
        .await
    {
        StreamOpen::Stream { session, .. } => session,
        StreamOpen::Buffer => panic!("expected a stream, got Buffer"),
        StreamOpen::Reject(_) => panic!("expected a stream, got Reject"),
    }
}

/// The GuardRails monitor events (text preview, categories, action), in order.
fn guardrail_events(firewall: &AppFirewall) -> Vec<(String, String, String)> {
    let store = &firewall.state.monitor_store;
    let mut events: Vec<_> = store
        .list(0, 200, None)
        .events
        .iter()
        .filter_map(|summary| store.get(&summary.id))
        .collect();
    events.sort_by_key(|event| event.sequence);
    events
        .into_iter()
        .filter_map(|event| match event.data {
            lr_monitor::MonitorEventData::GuardrailScan {
                text_preview,
                flagged_categories,
                action_taken,
                ..
            } => Some((
                text_preview,
                format!(
                    "{:?}",
                    flagged_categories
                        .unwrap_or_default()
                        .iter()
                        .map(|c| (c.category.clone(), c.action.clone()))
                        .collect::<Vec<_>>()
                ),
                action_taken.unwrap_or_default(),
            )),
            _ => None,
        })
        .collect()
}

fn refusal_message(action: &RequestAction) -> Option<(u16, Value)> {
    match action {
        RequestAction::Reject { status, body, .. } => {
            Some((*status, serde_json::from_slice(body).unwrap()))
        }
        _ => None,
    }
}

/// Streamed and buffered judging of the same request: the safety model gets
/// the same messages (masked the same way), the verdict, the refusal and the
/// monitor events are the same. At a lag of at least the body the pipeline
/// opens only at the end; at a small one it releases (into the held output)
/// while the body arrives.
#[tokio::test]
async fn streamed_guardrails_match_the_buffered_path() {
    for scan in [
        lr_config::SecretScanAction::Redact,
        lr_config::SecretScanAction::Notify,
    ] {
        for harmful in [false, true] {
            for (index, body) in corpus(harmful).into_iter().enumerate() {
                let bytes = encode(&body);
                for (lag, chunk) in [(bytes.len(), 997), (4096, 333), (4096, 1)] {
                    let label =
                        format!("{scan:?} harmful {harmful} body {index} lag {lag} chunk {chunk}");
                    let root = tempfile::tempdir().unwrap();
                    let (firewall, mut base, seen) = guarded(root.path(), "block", scan.clone());
                    base.body = body.clone();
                    // Streamed first: the buffered run reuses its references.
                    let mut session = open(&firewall, &base, masking(&scan)).await;
                    let (_, streamed) =
                        stream_and_judge(&firewall, &mut session, &bytes, lag, chunk).await;
                    drop(session);
                    let streamed_events = guardrail_events(&firewall);
                    let buffered = firewall.evaluate(&base).await;
                    let all = seen.lock().unwrap().clone();
                    assert_eq!(all.len(), 2, "{label}");
                    assert_eq!(all[0], all[1], "the same projection ({label})");
                    let joined: String = all[0].iter().map(|(_, c)| c.as_str()).collect();
                    if scan == lr_config::SecretScanAction::Redact {
                        assert!(!joined.contains(&github_token()), "masked ({label})");
                    } else if index == 0 {
                        assert!(joined.contains(&github_token()), "{label}");
                    }
                    if index == 1 {
                        // The text attachment is judged decoded.
                        assert!(joined.contains("in a text file"), "{label}");
                    }
                    let events = guardrail_events(&firewall);
                    match (streamed, refusal_message(&buffered)) {
                        (Ok(()), None) => {
                            assert!(!harmful, "{label}");
                            assert!(events.is_empty());
                        }
                        (Err(abort), Some((status, json))) => {
                            assert!(harmful, "{label}");
                            assert_eq!(abort.status, status);
                            let streamed: Value =
                                serde_json::from_slice(abort.json.as_ref().unwrap()).unwrap();
                            assert_eq!(streamed, json, "{label}");
                            assert_eq!(
                                json["error"]["message"],
                                "Request blocked by safety guardrails"
                            );
                            assert_eq!(streamed_events.len(), 1, "{label}");
                            assert_eq!(events.len(), 2, "{label}");
                            assert_eq!(events[0], events[1], "{label}");
                        }
                        (streamed, buffered) => {
                            panic!("verdicts differ ({label}): {streamed:?} vs {buffered:?}")
                        }
                    }
                }
            }
        }
    }
}

/// The next blocking approval popup.
async fn next_approval(firewall: &AppFirewall) -> lr_mcp::gateway::firewall::PendingApprovalInfo {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(pending) = firewall
                .firewall
                .list_pending()
                .into_iter()
                .find(|p| !p.is_notify_only)
            {
                return pending;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("an approval popup")
}

/// Ask: the GuardRails popup of a streamed request shows the same details
/// as the buffered one, and its answer decides the same way. With the
/// model permission on Ask too, the model popup follows the GuardRails
/// verdict (the buffered path's order) and never shows after a denial.
#[tokio::test]
async fn guardrail_popups_match_the_buffered_path_and_precede_the_model_popup() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
    let body = corpus(true).remove(0);
    let bytes = encode(&body);
    for (guard_answer, model_ask) in [
        (A::AllowOnce, false),
        (A::Deny, false),
        (A::AllowOnce, true),
        (A::Deny, true),
    ] {
        let label = format!("{guard_answer:?} model ask {model_ask}");
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut base, _) =
            guarded(root.path(), "ask", lr_config::SecretScanAction::Redact);
        if model_ask {
            firewall
                .config_manager
                .update(|config| {
                    config.strategies[0]
                        .auto_config
                        .get_or_insert_with(Default::default)
                        .permission = lr_config::PermissionState::Ask;
                })
                .unwrap();
        }
        base.body = body.clone();
        let firewall = Arc::new(firewall);
        // Streamed: no popup when the stream opens.
        let mut session = open(&firewall, &base, StreamMasking::Redact).await;
        assert!(firewall.firewall.list_pending().is_empty(), "{label}");
        let judged = {
            let firewall = firewall.clone();
            let bytes = bytes.clone();
            tokio::spawn(async move {
                let result = stream_and_judge(&firewall, &mut session, &bytes, 4096, 4096).await;
                result.1
            })
        };
        let streamed_popup = next_approval(&firewall).await;
        assert!(
            streamed_popup.is_guardrail_request,
            "{label}: GuardRails first"
        );
        firewall
            .firewall
            .submit_response(&streamed_popup.request_id, guard_answer.clone(), None)
            .unwrap();
        if model_ask && guard_answer == A::AllowOnce {
            let model_popup = next_approval(&firewall).await;
            assert!(model_popup.is_model_request, "{label}");
            firewall
                .firewall
                .submit_response(&model_popup.request_id, A::AllowOnce, None)
                .unwrap();
        }
        let streamed = judged.await.unwrap();
        assert!(firewall.firewall.list_pending().is_empty(), "{label}");
        // Buffered, answered the same way.
        let buffering = {
            let firewall = firewall.clone();
            let base_body = base.body.clone();
            let mut request = fixture_request(&base);
            request.body = base_body;
            tokio::spawn(async move { firewall.evaluate(&request).await })
        };
        let buffered_popup = next_approval(&firewall).await;
        assert!(buffered_popup.is_guardrail_request, "{label}");
        let details = |p: &lr_mcp::gateway::firewall::PendingApprovalInfo| {
            let d = p.guardrail_details.clone().unwrap();
            (d.flagged_text, d.actions_required, d.scan_direction)
        };
        assert_eq!(
            details(&streamed_popup),
            details(&buffered_popup),
            "{label}"
        );
        firewall
            .firewall
            .submit_response(&buffered_popup.request_id, guard_answer.clone(), None)
            .unwrap();
        if model_ask && guard_answer == A::AllowOnce {
            let model_popup = next_approval(&firewall).await;
            assert!(model_popup.is_model_request, "{label}");
            firewall
                .firewall
                .submit_response(&model_popup.request_id, A::AllowOnce, None)
                .unwrap();
        }
        let buffered = buffering.await.unwrap();
        match (streamed, refusal_message(&buffered)) {
            (Ok(()), None) => assert_eq!(guard_answer, A::AllowOnce, "{label}"),
            (Err(abort), Some((status, json))) => {
                assert_eq!(guard_answer, A::Deny, "{label}");
                assert_eq!(status, 403);
                assert_eq!(abort.status, 403);
                assert_eq!(
                    serde_json::from_slice::<Value>(abort.json.as_ref().unwrap()).unwrap(),
                    json
                );
                assert_eq!(json["error"]["message"], "Request blocked by safety check");
            }
            (streamed, buffered) => {
                panic!("verdicts differ ({label}): {streamed:?} vs {buffered:?}")
            }
        }
    }
}

fn fixture_request(base: &FirewallRequest) -> FirewallRequest {
    FirewallRequest {
        client_id: base.client_id.clone(),
        session_id: base.session_id.clone(),
        secret_session: base.secret_session.clone(),
        host: base.host.clone(),
        method: base.method.clone(),
        path: base.path.clone(),
        provider: base.provider.clone(),
        model: base.model.clone(),
        has_tools: base.has_tools,
        message_count: base.message_count,
        body: Value::Null,
        trace: base.trace.clone(),
        admission: Default::default(),
    }
}

/// A refused model approval after an Allow verdict refuses the request with
/// the buffered path's message.
#[tokio::test]
async fn a_deferred_model_denial_refuses_after_the_verdict() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
    let body = corpus(false).remove(0);
    let bytes = encode(&body);
    let root = tempfile::tempdir().unwrap();
    let (firewall, mut base, _) = guarded(root.path(), "ask", lr_config::SecretScanAction::Redact);
    firewall
        .config_manager
        .update(|config| {
            config.strategies[0]
                .auto_config
                .get_or_insert_with(Default::default)
                .permission = lr_config::PermissionState::Ask;
        })
        .unwrap();
    base.body = body;
    let firewall = Arc::new(firewall);
    let mut session = open(&firewall, &base, StreamMasking::Redact).await;
    let judged = {
        let firewall = firewall.clone();
        tokio::spawn(async move {
            stream_and_judge(&firewall, &mut session, &bytes, 4096, 4096)
                .await
                .1
        })
    };
    // Nothing flagged: no GuardRails popup, straight to the model.
    let popup = next_approval(&firewall).await;
    assert!(popup.is_model_request);
    let preview = popup.request_preview.clone().expect("a read-only preview");
    assert!(preview.truncated);
    firewall
        .firewall
        .submit_response(&popup.request_id, A::Deny, None)
        .unwrap();
    let abort = judged.await.unwrap().expect_err("a denied model refuses");
    assert_eq!(abort.status, 403);
    let json: Value = serde_json::from_slice(abort.json.as_ref().unwrap()).unwrap();
    assert_eq!(json["error"]["message"], "Denied by user");
}

/// Off/Notify GuardRails sessions tee (the original bytes are held, not
/// re-assembled), and only GuardRails sessions hold until a verdict.
#[tokio::test]
async fn guardrails_hold_every_mode_and_tee_off_and_notify() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, base, _) = guarded(root.path(), "block", lr_config::SecretScanAction::Off);
    let req = stream_request(&base);
    match firewall
        .open_stream(&req, &policy(StreamMasking::Off))
        .await
    {
        StreamOpen::Stream { session, tee, .. } => {
            assert!(tee);
            assert!(session.holds_until_verdict());
        }
        _ => panic!("GuardRails stream"),
    }
    // Without active GuardRails nothing holds.
    firewall
        .config_manager
        .update(|config| {
            config.guardrails.category_actions = vec![lr_config::CategoryActionEntry {
                category: "__global".into(),
                action: "allow".into(),
            }];
        })
        .unwrap();
    match firewall
        .open_stream(&req, &policy(StreamMasking::Off))
        .await
    {
        StreamOpen::Stream { session, .. } => assert!(!session.holds_until_verdict()),
        _ => panic!("stream"),
    }
    // The head-time policy streams GuardRails requests (bodies within the
    // lag stay buffered in the transport).
    firewall
        .config_manager
        .update(|config| {
            config.guardrails.category_actions = vec![lr_config::CategoryActionEntry {
                category: "__global".into(),
                action: "block".into(),
            }];
        })
        .unwrap();
    let headers = hyper::HeaderMap::new();
    let head = lr_proxy::interceptor::RequestHead {
        client_id: &base.client_id,
        strategy_id: "",
        host: "chatgpt.com",
        port: 443,
        method: "POST",
        path: "/backend-api/codex/responses",
        headers: &headers,
        trace: None,
        provider_override: None,
        source: lr_proxy::interceptor::ExchangeSource::Proxy,
    };
    assert!(matches!(
        firewall.body_policy(&head),
        lr_proxy::interceptor::BodyPolicy::Stream(_)
    ));
}

/// The projection holds only text: a body that is mostly image data keeps a
/// projection a fraction of its size, and the pipeline holds about the lag.
#[tokio::test]
async fn the_projection_holds_only_the_text() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, base, seen) = guarded(root.path(), "block", lr_config::SecretScanAction::Off);
    let image = "A".repeat(256 * 1024);
    let text = "t".repeat(1024);
    let input: Vec<Value> = (0..400)
        .map(|i| {
            json!({"type":"message","role":"user","content":[
                {"type":"input_text","text":format!("{i} {text}")},
                {"type":"input_image","image_url":format!("data:image/png;base64,{image}")},
            ]})
        })
        .collect();
    let body = json!({"model":"local-test","input":input});
    let bytes = encode(&body);
    let lag = 4 * 1024 * 1024;
    let mut session = crate::launcher::proxy_stream::AppStreamSession::new(
        firewall.state.clone(),
        base.client_id.clone(),
        base.session_id.clone(),
        None,
        None,
        false,
    )
    .with_guardrails(crate::launcher::proxy_stream::GuardrailsHold::new(None));
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    pipeline.inspect_only();
    let mut out = Vec::new();
    let mut opened = false;
    for piece in bytes.chunks(64 * 1024) {
        if opened {
            pipeline
                .push(
                    piece,
                    Some(&mut session as &mut dyn StreamSession),
                    &mut out,
                )
                .unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline
                    .push(&[], Some(&mut session as &mut dyn StreamSession), &mut out)
                    .unwrap();
            }
        }
    }
    pipeline.finish(&mut session, &mut out).unwrap();
    let projected = session.projection_bytes().unwrap();
    let texts: usize = (0..400).map(|i| format!("{i} {text}").len()).sum();
    assert_eq!(projected, texts);
    assert!(bytes.len() > 100 * 1024 * 1024);
    assert!(projected < bytes.len() / 200);
    assert!(
        pipeline.max_held() < lag + 2 * 1024 * 1024,
        "{}",
        pipeline.max_held()
    );
    let question = session.question().expect("the verdict");
    let answer = question.0.await;
    session.answer(answer).unwrap();
    let asked = seen.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].len(), 400);
}

/// The buffered path judges text attachments decoded (as the gateway does).
#[tokio::test]
async fn buffered_guardrails_read_text_attachments() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, mut base, _) = guarded(root.path(), "block", lr_config::SecretScanAction::Off);
    base.body = corpus(true).remove(1);
    let (status, json) = refusal_message(&firewall.evaluate(&base).await).expect("refused");
    assert_eq!(status, 403);
    assert_eq!(
        json["error"]["message"],
        "Request blocked by safety guardrails"
    );
}
