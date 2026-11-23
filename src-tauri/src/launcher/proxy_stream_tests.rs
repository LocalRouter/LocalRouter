//! Parity of streamed requests with the buffered firewall path, and the
//! decisions taken when a body outgrows its stream lag.

use super::secret_policy_tests::fixture;
use super::*;
use lr_proxy::active::{Firewall, StreamRequest};
use lr_proxy::interceptor::{
    RequestAction, StreamMasking, StreamOpen, StreamPolicy, DEFAULT_STREAM_LAG,
};
use lr_proxy::request_stream::{RequestPipeline, StreamSession, StreamSummary, MAX_HOLD};
use serde_json::{json, Value};

// Synthetic credentials, assembled at run time so no literal secret sits in
// the source.
fn aws_key() -> String {
    format!("{}{}", "AKIA", "Z7Q2XW5PLM4RT9VB")
}
fn github_token() -> String {
    format!("{}_{}", "ghp", "aB3dE5gH7jK9mN1pQ3sT5vW7yZ9bC1dF3hJ5")
}

fn request(firewall: &AppFirewall, base: &lr_proxy::active::FirewallRequest) -> StreamRequest {
    let _ = firewall;
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
            text: format!(
                r#"{{"model":"local-test","input":["token {}"#,
                github_token()
            ),
            received: 40 * 1024 * 1024,
            truncated: true,
        }),
        trace: None,
        admission: Default::default(),
    }
}

/// The next blocking (not notify-only) approval popup.
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

/// Compact JSON with `model` first (as Codex sends it).
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

/// Drive a session over a body. With `lag` at least the body size the
/// pipeline is opened only at the end, so every unit is inspected before any
/// is released (the buffered path's guarantee).
fn stream(
    session: &mut dyn StreamSession,
    body: &[u8],
    lag: usize,
    chunk: usize,
) -> (Vec<u8>, StreamSummary, bool) {
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut out = Vec::new();
    let mut open = false;
    let mut released_early = false;
    for piece in body.chunks(chunk) {
        if open {
            pipeline.push(piece, Some(&mut *session), &mut out).unwrap();
            released_early |= !out.is_empty();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                open = true;
                pipeline.push(&[], Some(&mut *session), &mut out).unwrap();
            }
        }
    }
    if !open {
        pipeline.open();
    }
    let summary = pipeline.finish(session, &mut out).unwrap();
    (out, summary, released_early)
}

async fn open(
    firewall: &AppFirewall,
    base: &lr_proxy::active::FirewallRequest,
    masking: StreamMasking,
) -> (
    Box<dyn StreamSession>,
    Option<Arc<lr_secret_scanner::placeholders::Vault>>,
) {
    match firewall
        .open_stream(&request(firewall, base), &policy(masking))
        .await
    {
        StreamOpen::Stream { session, vault, .. } => (session, vault),
        StreamOpen::Buffer => panic!("expected a stream, got Buffer"),
        StreamOpen::Reject(_) => panic!("expected a stream, got Reject"),
    }
}

fn conversation(secret: &str, token: &str) -> Value {
    let filler = "lorem ipsum ".repeat(40);
    let mut input: Vec<Value> = (0..40)
        .map(|i| json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("{i} {filler}")}]}))
        .collect();
    input[5] = json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("aws_access_key_id={secret}")}]});
    input[12] = json!({"type":"function_call","call_id":"call_1","name":"exec","arguments":serde_json::to_string(&json!({"cmd":format!("export GITHUB_TOKEN={token}")})).unwrap()});
    input[13] = json!({"type":"function_call_output","call_id":"call_1","output":format!("token {token} and {secret}")});
    input[20] =
        json!({"type":"reasoning","encrypted_content":format!("opaque-{secret}"),"summary":[]});
    input[30] = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":format!("I saw {secret}")}]});
    json!({
        "model": "local-test",
        "instructions": "Be brief.",
        "input": input,
        "tools": [{"type":"function","name":"exec","parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}],
        "text": {"format":{"type":"json_schema","name":"out","schema":{"type":"object"}}},
        "extensions": {"localrouter.session":{"id":"conversation-1"}},
        "stream": true,
    })
}

fn secret_scan_findings(firewall: &AppFirewall) -> Vec<Vec<(String, String)>> {
    let store = &firewall.state.monitor_store;
    let mut events: Vec<_> = store
        .list(0, 100, None)
        .events
        .iter()
        .filter_map(|summary| store.get(&summary.id))
        .collect();
    events.sort_by_key(|event| event.sequence);
    events
        .into_iter()
        .filter_map(|event| match event.data {
            lr_monitor::MonitorEventData::SecretScan {
                findings: Some(Value::Array(findings)),
                ..
            } => {
                let mut found: Vec<(String, String)> = findings
                    .iter()
                    .map(|f| {
                        (
                            f["rule_id"].as_str().unwrap_or_default().to_owned(),
                            f["location"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect();
                found.sort();
                Some(found)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn streamed_redaction_matches_the_buffered_path() {
    // The AWS key id alone is not detected (both paths forward it); the
    // GitHub token is, and also sits in opaque reasoning state.
    let token = github_token();
    let secret = token.clone();
    let body = conversation(&token, &token);
    let bytes = encode(&body);
    for (lag, chunk) in [
        (bytes.len(), 997),
        (4096, 1),
        (4096, 333),
        (16 * 1024, 4096),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (firewall, mut base) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        base.body = body.clone();
        // Streamed first: the buffered run then reuses its references, so
        // both outputs carry the same reference ids.
        let (mut session, vault) = open(&firewall, &base, StreamMasking::Redact).await;
        let (streamed, summary, early) = stream(session.as_mut(), &bytes, lag, chunk);
        drop(session);
        assert_eq!(early, lag < bytes.len(), "lag {lag}");
        let RequestAction::Protect {
            body: buffered,
            vault: _,
        } = firewall.evaluate(&base).await
        else {
            panic!("Redact protects")
        };
        let streamed_value: Value = serde_json::from_slice(&streamed).unwrap();
        let buffered_value: Value = serde_json::from_slice(&buffered).unwrap();
        assert_eq!(streamed_value, buffered_value, "lag {lag} chunk {chunk}");
        // Nothing detected leaves in any spelling, except inside the opaque
        // reasoning state, which both paths forward unchanged.
        let text = String::from_utf8(streamed.clone()).unwrap();
        assert_eq!(text.matches(secret.as_str()).count(), 1, "lag {lag}");
        assert!(text.contains(&format!("opaque-{secret}")));
        assert!(streamed_value.get("extensions").is_none());
        // Same findings, recorded once per request.
        let findings = secret_scan_findings(&firewall);
        assert_eq!(findings.len(), 2, "lag {lag}");
        assert_eq!(findings[0], findings[1], "lag {lag}");
        assert!(!findings[0].is_empty());
        // Conversation correlation sees the same forwarded messages.
        let whole = lr_monitor::CorrelationParts::from_body(&buffered_value);
        assert_eq!(
            format!("{:?}", summary.correlation),
            format!("{whole:?}"),
            "lag {lag}"
        );
        // References restore to the originals.
        let vault = vault.unwrap();
        let mut restored = streamed_value.clone();
        use lr_types::secret_context::SecretContext;
        vault.restore(&mut restored, false).unwrap();
        assert!(restored.to_string().contains(&token));
        // The contract was captured from the forwarded schemas.
        assert!(vault.structured_output());
    }
}

#[tokio::test]
async fn notify_forwards_the_original_bytes_and_records_once() {
    let secret = aws_key();
    let token = github_token();
    let body = conversation(&secret, &token);
    let bytes = encode(&body);
    let root = tempfile::tempdir().unwrap();
    let (firewall, mut base) = fixture(
        root.path(),
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    firewall
        .config_manager
        .update(|config| {
            config.secret_scanning.action = lr_config::SecretScanAction::Notify;
        })
        .unwrap();
    base.body = body.clone();
    let (mut session, vault) = open(&firewall, &base, StreamMasking::Notify).await;
    assert!(vault.is_none());
    let (streamed, _, early) = stream(session.as_mut(), &bytes, 4096, 512);
    assert!(early);
    assert_eq!(streamed, bytes);
    let findings = secret_scan_findings(&firewall);
    assert_eq!(findings.len(), 1);
    assert!(!findings[0].is_empty());
}

#[tokio::test]
async fn open_stream_buffers_what_needs_the_whole_request_and_refuses_what_is_not_allowed() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
    let root = tempfile::tempdir().unwrap();
    let (firewall, base) = fixture(
        root.path(),
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    let firewall = Arc::new(firewall);
    let req = Arc::new(request(&firewall, &base));
    // The masking mode changed since the head was read.
    assert!(matches!(
        firewall
            .open_stream(&req, &policy(StreamMasking::Notify))
            .await,
        StreamOpen::Buffer
    ));
    // So did strict mode.
    let mut strict = policy(StreamMasking::Redact);
    strict.strict = true;
    assert!(matches!(
        firewall.open_stream(&req, &strict).await,
        StreamOpen::Buffer
    ));
    // Secret Scan Ask streams: its popups hold the upload per finding.
    firewall
        .config_manager
        .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Ask)
        .unwrap();
    assert!(matches!(
        firewall
            .open_stream(&req, &policy(StreamMasking::Ask))
            .await,
        StreamOpen::Stream {
            tee: false,
            vault: Some(_),
            ..
        }
    ));
    firewall
        .config_manager
        .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Redact)
        .unwrap();
    // A model approval without a standing grant asks with a read-only,
    // masked preview: approval opens the stream, anything else refuses it.
    firewall
        .config_manager
        .update(|config| {
            config.strategies[0]
                .auto_config
                .get_or_insert_with(Default::default)
                .permission = lr_config::PermissionState::Ask;
        })
        .unwrap();
    for (action, streams) in [(A::AllowOnce, true), (A::Deny, false)] {
        let opening = {
            let firewall = firewall.clone();
            let req = req.clone();
            tokio::spawn(async move {
                firewall
                    .open_stream(&req, &policy(StreamMasking::Redact))
                    .await
            })
        };
        let pending = next_approval(&firewall).await;
        assert!(pending.is_model_request);
        assert!(pending.full_arguments.is_none(), "not editable");
        let preview = pending.request_preview.clone().expect("a preview");
        assert!(preview.truncated);
        assert_eq!(preview.total_bytes, 40 * 1024 * 1024);
        assert!(preview.text.starts_with(r#"{"model":"local-test""#));
        assert!(!preview.text.contains(&github_token()), "{}", preview.text);
        firewall
            .firewall
            .submit_response(&pending.request_id, action, Some(json!({"edited": true})))
            .unwrap();
        match opening.await.unwrap() {
            StreamOpen::Stream { .. } => assert!(streams),
            StreamOpen::Reject(RequestAction::Reject { status, .. }) => {
                assert!(!streams);
                assert_eq!(status, 403);
            }
            _ => panic!("unexpected decision"),
        }
    }
    // Access off: refused before anything is sent.
    firewall
        .config_manager
        .update(|config| {
            config.strategies[0]
                .auto_config
                .get_or_insert_with(Default::default)
                .permission = lr_config::PermissionState::Off;
        })
        .unwrap();
    assert!(matches!(
        firewall
            .open_stream(&req, &policy(StreamMasking::Redact))
            .await,
        StreamOpen::Reject(RequestAction::Reject { status: 403, .. })
    ));
}

#[tokio::test]
async fn off_and_notify_tee_unless_a_remembered_redaction_may_apply() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, base) = fixture(
        root.path(),
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    let req = request(&firewall, &base);
    for (action, masking) in [
        (lr_config::SecretScanAction::Off, StreamMasking::Off),
        (lr_config::SecretScanAction::Notify, StreamMasking::Notify),
    ] {
        firewall
            .config_manager
            .update(|config| config.secret_scanning.action = action.clone())
            .unwrap();
        assert!(
            matches!(
                firewall.open_stream(&req, &policy(masking)).await,
                StreamOpen::Stream { tee: true, .. }
            ),
            "{action:?}"
        );
    }
    // Redact never tees.
    firewall
        .config_manager
        .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Redact)
        .unwrap();
    assert!(matches!(
        firewall
            .open_stream(&req, &policy(StreamMasking::Redact))
            .await,
        StreamOpen::Stream { tee: false, .. }
    ));
    // A remembered "always redact" value can change a Notify body.
    firewall
        .config_manager
        .update(|config| {
            config.secret_scanning.action = lr_config::SecretScanAction::Notify;
            config.clients[0]
                .secret_scanning
                .dismissed_secrets
                .push(lr_config::DismissedSecret {
                    redact: true,
                    id: "d1".into(),
                    hash: "00".repeat(32),
                    iterations: 1,
                    rule_id: "r".into(),
                    rule_description: "r".into(),
                    hint: "x".into(),
                    dismissed_at: "2026-11-22T00:00:00Z".into(),
                });
        })
        .unwrap();
    firewall
        .state
        .client_manager
        .sync_clients(firewall.config_manager.get().clients.clone());
    assert!(matches!(
        firewall
            .open_stream(&req, &policy(StreamMasking::Notify))
            .await,
        StreamOpen::Stream { tee: false, .. }
    ));
}

/// Secret Scan Ask on a streamed body: each new finding raises one popup at
/// a time while nothing more is released; the answer decides how the held
/// content goes out, and a denial ends the upload.
#[tokio::test]
async fn ask_findings_are_asked_one_at_a_time_and_the_answers_apply() {
    use lr_mcp::gateway::firewall::FirewallApprovalAction as A;
    use lr_proxy::request_stream::StreamAbort;
    let token = github_token();
    let other = format!("{}_{}", "ghp", "Zx9Yw8Vu7Ts6Rq5Po4Nm3Lk2Jh1Gf0Ed9Cb8");
    let mut body = conversation(&aws_key(), &token);
    body["input"][30] = json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("export GITHUB_TOKEN={other}")}]});
    let bytes = encode(&body);
    // (answer for the first token, answer for the second)
    for (for_token, for_other) in [
        (A::RedactOnce, A::AllowOnce),
        (A::ReplaceSecrets, A::ReplaceSecrets),
        (A::AllowOnce, A::Deny),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (firewall, base) = fixture(
            root.path(),
            lr_config::LlmMode::Proxy,
            Arc::new(lr_api_keys::MockKeychain::new()),
        );
        firewall
            .config_manager
            .update(|config| config.secret_scanning.action = lr_config::SecretScanAction::Ask)
            .unwrap();
        let (mut session, vault) = open(&firewall, &base, StreamMasking::Ask).await;
        assert!(vault.is_some());
        let mut pipeline = RequestPipeline::new(4096, MAX_HOLD);
        let mut out = Vec::new();
        let mut asked = Vec::new();
        let mut result: Result<(), StreamAbort> = Ok(());
        let mut opened = false;
        'feed: for piece in bytes.chunks(700) {
            let pushed = if opened {
                pipeline.push(piece, Some(session.as_mut()), &mut out)
            } else {
                let pushed = pipeline.push(piece, None, &mut out);
                if pipeline.ready_to_open() {
                    pipeline.open();
                    opened = true;
                    pipeline.push(&[], Some(session.as_mut()), &mut out)
                } else {
                    pushed
                }
            };
            if let Err(abort) = pushed {
                result = Err(abort);
                break;
            }
            while let Some(question) = session.question() {
                assert!(session.pending());
                assert!(pipeline.gated(), "release stops while asking");
                let held = out.len();
                let waiting = tokio::spawn(question.0);
                let pending = next_approval(&firewall).await;
                assert!(pending.is_secret_scan_request);
                let details = pending.secret_scan_details.clone().unwrap();
                assert_eq!(details.findings.len(), 1, "one finding per popup");
                assert!(details.request_json.is_none());
                let raw = details.findings[0].raw_text.clone();
                let action = if raw.contains(&token) {
                    for_token.clone()
                } else if raw.contains(&other) {
                    for_other.clone()
                } else {
                    A::AllowOnce
                };
                asked.push(raw);
                firewall
                    .firewall
                    .submit_response(&pending.request_id, action, None)
                    .unwrap();
                let answer = waiting.await.unwrap();
                match session.answer(answer) {
                    Ok(registered) => pipeline.registered(&registered, session.as_mut()),
                    Err(abort) => {
                        let json: Value =
                            serde_json::from_slice(abort.json.as_ref().unwrap()).unwrap();
                        assert_eq!(json["error"]["code"], "secret_scan_denied_by_user");
                        result = Err(abort);
                        break 'feed;
                    }
                }
                assert_eq!(out.len(), held, "nothing was released while asking");
                if let Err(abort) = pipeline.push(&[], Some(session.as_mut()), &mut out) {
                    result = Err(abort);
                    break 'feed;
                }
            }
        }
        // Each value is asked about once, however often it occurs.
        let about = |value: &str| asked.iter().filter(|raw| raw.contains(value)).count();
        assert_eq!(about(&token), 1, "{for_token:?}/{for_other:?}");
        assert_eq!(about(&other), 1, "{for_token:?}/{for_other:?}");
        let text = String::from_utf8_lossy(&out).into_owned();
        if for_other == A::Deny {
            let abort = result.expect_err("a denial ends the upload");
            assert_eq!(abort.status, 403);
            // Nothing from the denied finding's unit on was released.
            assert!(!text.contains(&other), "denied content was released");
            continue;
        }
        result.unwrap();
        pipeline.finish(session.as_mut(), &mut out).unwrap();
        let forwarded: Value = serde_json::from_slice(&out).unwrap();
        let call = forwarded["input"][12].to_string();
        let output = forwarded["input"][13].to_string();
        let later = forwarded["input"][30].to_string();
        match (for_token, for_other) {
            (A::RedactOnce, A::AllowOnce) => {
                assert!(!call.contains(&token), "{call}");
                assert!(!output.contains(&token), "{output}");
                assert!(later.contains(&other), "{later}");
            }
            (A::ReplaceSecrets, A::ReplaceSecrets) => {
                assert!(!call.contains(&token), "{call}");
                assert!(!output.contains(&token), "{output}");
                assert!(!later.contains(&other), "{later}");
            }
            _ => unreachable!(),
        }
    }
}

#[path = "proxy_ws_stream_tests.rs"]
mod websocket;
