//! Secret Scan on non-inference routes (`content_scan`) through the native
//! request boundary: every route that carries user text to a provider, in
//! Redact, Ask, Notify and Off.
use super::*;
use axum::{routing::post, Extension};
use lr_mcp::gateway::firewall::FirewallApprovalAction;
use serde_json::json;
use tower::ServiceExt;

const VALUE: &str = "Zq8vR2nL5tY7wB1xK4mP9sD3fH6jG0cV2bN8eT5a";

fn secret() -> String {
    format!("aws_secret_access_key={VALUE}")
}

type Seen = Arc<std::sync::Mutex<Vec<String>>>;

/// Every JSON content route behind the boundary. The endpoint records the
/// body it receives and echoes it back as JSON; `/v1/audio/speech` answers
/// with audio bytes that contain the received text.
fn app(
    state: AppState,
    storage: Arc<dyn lr_api_keys::KeychainStorage>,
    seen: Seen,
) -> axum::Router {
    let owner = state.config_manager.get().clients[0].id.clone();
    let echo_seen = seen.clone();
    let echo = move |body: axum::body::Bytes| {
        let seen = echo_seen.clone();
        async move {
            let text = String::from_utf8(body.to_vec()).unwrap();
            seen.lock().unwrap().push(text.clone());
            let value: Value = serde_json::from_str(&text).unwrap();
            axum::Json(json!({ "echo": value }))
        }
    };
    let speech = move |body: axum::body::Bytes| {
        let seen = seen.clone();
        async move {
            let text = String::from_utf8(body.to_vec()).unwrap();
            seen.lock().unwrap().push(text.clone());
            Response::builder()
                .header("content-type", "audio/mpeg")
                .header("content-length", text.len())
                .body(Body::from(text))
                .unwrap()
        }
    };
    let mut router = axum::Router::new();
    for path in [
        "/v1/embeddings",
        "/v1/moderations",
        "/v1/rerank",
        "/v1/search",
        "/v1/ocr",
        "/v1/images/generations",
        "/v1/videos",
        "/v1/messages/count_tokens",
        "/audio/speech",
        "/v1/files",
    ] {
        router = router.route(path, post(echo.clone()));
    }
    router
        .route("/v1/audio/speech", post(speech))
        .layer(axum::middleware::from_fn_with_state(
            state,
            move |State(state): State<AppState>, request: Request, next: Next| {
                let storage = storage.clone();
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
        }))
}

/// One request per route, with the secret in the field that carries the
/// user's text.
fn cases() -> Vec<(&'static str, Value)> {
    let secret = secret();
    vec![
        (
            "/v1/embeddings",
            json!({"model": "m", "input": ["harmless", secret]}),
        ),
        (
            "/v1/moderations",
            json!({"input": [{"type": "text", "text": secret}]}),
        ),
        (
            "/v1/rerank",
            json!({"model": "m", "query": "which", "documents": ["a", secret], "return_documents": true}),
        ),
        ("/v1/search", json!({"query": [secret]})),
        (
            "/v1/ocr",
            json!({"model": "m", "document": {"document_url": "https://example.com/d.pdf"},
                   "pages": "0", "filename": secret}),
        ),
        (
            "/v1/images/generations",
            json!({"model": "m", "prompt": secret}),
        ),
        (
            "/v1/videos",
            json!({"model": "p/m", "prompt": secret, "seconds": 4}),
        ),
        (
            "/v1/messages/count_tokens",
            json!({"model": "m", "messages": [{"role": "user", "content": secret}]}),
        ),
        (
            "/audio/speech",
            json!({"model": "m", "input": secret, "voice": "alloy"}),
        ),
    ]
}

fn post_json(path: &str, body: &Value) -> Request {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn text(response: Response) -> (axum::http::StatusCode, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn with_action(action: lr_config::SecretScanAction) -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = super::boundary_tests::state(root.path());
    state
        .config_manager
        .update(|config| config.secret_scanning.action = action)
        .unwrap();
    (root, state)
}

#[test]
fn the_synthetic_secret_is_detected() {
    let root = tempfile::tempdir().unwrap();
    let state = super::boundary_tests::state(root.path());
    let scanner = state.secret_scanner.read().clone().unwrap();
    let result = scanner.scan(&[lr_secret_scanner::ExtractedText {
        label: "user message".into(),
        text: secret(),
        message_index: 0,
    }]);
    assert!(
        result.findings.iter().any(|f| f.raw_text.contains(VALUE)),
        "test fixture must be detected"
    );
}

#[tokio::test]
async fn redact_masks_every_content_route_and_restores_text_responses() {
    for (path, body) in cases() {
        let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
        let seen = Seen::default();
        let storage: Arc<dyn lr_api_keys::KeychainStorage> =
            Arc::new(lr_api_keys::MockKeychain::new());
        let (status, response) = text(
            app(state, storage, seen.clone())
                .oneshot(post_json(path, &body))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200, "{path}: {response}");
        let forwarded = seen.lock().unwrap().clone();
        assert_eq!(forwarded.len(), 1, "{path}");
        assert!(!forwarded[0].contains(VALUE), "{path}: {}", forwarded[0]);
        assert!(
            forwarded[0].contains("localrouter-redacted"),
            "{path}: {}",
            forwarded[0]
        );
        // The echoed placeholder is restored for the client.
        assert!(response.contains(VALUE), "{path}: {response}");
    }
}

#[tokio::test]
async fn binary_responses_pass_untouched() {
    let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
    let seen = Seen::default();
    let response = app(
        state,
        Arc::new(lr_api_keys::MockKeychain::new()),
        seen.clone(),
    )
    .oneshot(post_json(
        "/v1/audio/speech",
        &json!({"model": "m", "input": secret(), "voice": "alloy"}),
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let length = response.headers().get("content-length").cloned();
    let (_, audio) = text(response).await;
    let forwarded = seen.lock().unwrap()[0].clone();
    assert!(!forwarded.contains(VALUE));
    // Audio is not rewritten: what the endpoint produced is delivered.
    assert_eq!(audio, forwarded);
    assert_eq!(
        length.unwrap().to_str().unwrap(),
        forwarded.len().to_string()
    );
}

#[tokio::test]
async fn notify_forwards_unchanged_and_opens_the_notification() {
    for (path, body) in cases() {
        let (_root, state) = with_action(lr_config::SecretScanAction::Notify);
        let seen = Seen::default();
        let (status, _) = text(
            app(
                state.clone(),
                Arc::new(lr_api_keys::MockKeychain::new()),
                seen.clone(),
            )
            .oneshot(post_json(path, &body))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, 200, "{path}");
        assert!(seen.lock().unwrap()[0].contains(VALUE), "{path}");
        let pending = state.mcp_gateway.firewall_manager.list_pending();
        assert!(
            pending
                .iter()
                .any(|p| p.is_secret_scan_request && p.is_notify_only),
            "{path}: a Notify popup is opened"
        );
    }
}

#[tokio::test]
async fn off_leaves_content_routes_alone() {
    for (path, body) in cases() {
        let (_root, state) = with_action(lr_config::SecretScanAction::Off);
        let seen = Seen::default();
        let (status, _) = text(
            app(
                state.clone(),
                Arc::new(lr_api_keys::MockKeychain::new()),
                seen.clone(),
            )
            .oneshot(post_json(path, &body))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, 200, "{path}");
        assert_eq!(seen.lock().unwrap()[0], body.to_string(), "{path}");
        assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
    }
}

async fn ask(
    path: &str,
    body: Value,
    decision: FirewallApprovalAction,
) -> (u16, String, Vec<String>) {
    let (_root, state) = with_action(lr_config::SecretScanAction::Ask);
    let seen = Seen::default();
    let app = app(
        state.clone(),
        Arc::new(lr_api_keys::MockKeychain::new()),
        seen.clone(),
    );
    let request = post_json(path, &body);
    let task = tokio::spawn(async move { text(app.oneshot(request).await.unwrap()).await });
    let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(pending) = state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .into_iter()
                .find(|p| p.is_secret_scan_request && !p.is_notify_only)
            {
                break pending;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("an approval is asked for");
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(&pending.request_id, decision, None)
        .unwrap();
    let (status, response) = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let forwarded = seen.lock().unwrap().clone();
    (status.as_u16(), response, forwarded)
}

#[tokio::test]
async fn ask_denial_stops_the_request_and_replacement_masks_it() {
    for (path, body) in [
        ("/v1/embeddings", json!({"model": "m", "input": secret()})),
        (
            "/v1/images/generations",
            json!({"model": "m", "prompt": secret()}),
        ),
    ] {
        let (status, response, forwarded) =
            ask(path, body.clone(), FirewallApprovalAction::Deny).await;
        assert_eq!(status, 403, "{path}: {response}");
        assert!(
            response.contains("secret_scan_denied_by_user"),
            "{response}"
        );
        assert!(forwarded.is_empty(), "{path}: nothing reaches the endpoint");

        let (status, _, forwarded) =
            ask(path, body.clone(), FirewallApprovalAction::ReplaceSecrets).await;
        assert_eq!(status, 200, "{path}");
        assert!(!forwarded[0].contains(VALUE), "{path}: {}", forwarded[0]);

        let (status, _, forwarded) = ask(path, body, FirewallApprovalAction::AllowOnce).await;
        assert_eq!(status, 200, "{path}");
        assert!(forwarded[0].contains(VALUE), "{path}: the user allowed it");
    }
}

#[tokio::test]
async fn compressed_content_route_bodies_are_refused() {
    let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
    let seen = Seen::default();
    let response = app(
        state,
        Arc::new(lr_api_keys::MockKeychain::new()),
        seen.clone(),
    )
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/embeddings")
            .header("content-type", "application/json")
            .header("content-encoding", "gzip")
            .body(Body::from(vec![0x1f, 0x8b, 0x08, 0x00]))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 415);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routes_without_provider_text_are_still_forwarded_unread() {
    // File uploads are stored locally; batch rows are scanned by the batch
    // policy before submission.
    let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
    let seen = Seen::default();
    let body = json!({"note": secret()});
    let (status, _) = text(
        app(
            state,
            Arc::new(lr_api_keys::MockKeychain::new()),
            seen.clone(),
        )
        .oneshot(post_json("/v1/files", &body))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(seen.lock().unwrap()[0], body.to_string());
}

/// A masked request in which a detected secret is still present is refused
/// instead of being forwarded.
#[tokio::test]
async fn a_secret_that_survived_masking_refuses_the_request() {
    let projection = |text: &str| json!({"messages": [{"role": "user", "content": text}]});
    // Redact: every new finding should have been masked.
    let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
    let owner = state.config_manager.get().clients[0].id.clone();
    let vault = Vault::new(
        Arc::new(lr_api_keys::MockKeychain::new()),
        &owner,
        "s",
        true,
    );
    let refused =
        crate::content_scan::verify_masked(&state, &owner, &vault, &projection(&secret()))
            .unwrap_err();
    assert_eq!(refused.status, axum::http::StatusCode::FORBIDDEN);
    assert_eq!(
        refused.error.error.code.as_deref(),
        Some("secret_scan_unmaskable")
    );
    let mut masked = projection(&secret());
    vault.register(&secret()).unwrap();
    vault.protect(&mut masked).unwrap();
    crate::content_scan::verify_masked(&state, &owner, &vault, &masked).unwrap();
    crate::content_scan::verify_masked(&state, &owner, &vault, &projection("nothing here"))
        .unwrap();

    // Ask: a value the user allowed is not in the vault and passes; one the
    // vault holds but that is still present does not.
    let (_root, state) = with_action(lr_config::SecretScanAction::Ask);
    let vault = Vault::new(
        Arc::new(lr_api_keys::MockKeychain::new()),
        &owner,
        "s",
        true,
    );
    crate::content_scan::verify_masked(&state, &owner, &vault, &projection(&secret())).unwrap();
    let scanner = state.secret_scanner.read().clone().unwrap();
    for finding in scanner
        .scan(&[lr_secret_scanner::ExtractedText {
            label: "user message".into(),
            text: secret(),
            message_index: 0,
        }])
        .findings
    {
        vault.register(&finding.raw_text).unwrap();
    }
    assert!(
        crate::content_scan::verify_masked(&state, &owner, &vault, &projection(&secret())).is_err()
    );
}

/// Multipart routes: the boundary sets up the vault without reading the
/// body; the handler scans its text fields.
#[tokio::test]
async fn form_fields_are_masked_inside_the_boundary_and_responses_restored() {
    let (_root, state) = with_action(lr_config::SecretScanAction::Redact);
    let owner = state.config_manager.get().clients[0].id.clone();
    let seen = Seen::default();
    let handler_seen = seen.clone();
    let handler_state = state.clone();
    let handler_owner = owner.clone();
    let handler = move |body: axum::body::Bytes| {
        let seen = handler_seen.clone();
        let state = handler_state.clone();
        let owner = handler_owner.clone();
        async move {
            // The boundary did not consume the form.
            assert!(String::from_utf8_lossy(&body).contains("multipart-marker"));
            let mut prompt = secret();
            let mut empty = String::new();
            crate::content_scan::scan_form_fields(
                &state,
                &owner,
                "m",
                &mut [&mut prompt, &mut empty],
            )
            .await
            .map_err(IntoResponse::into_response)?;
            seen.lock().unwrap().push(prompt.clone());
            Ok::<_, Response>(axum::Json(json!({"text": prompt})))
        }
    };
    let storage: Arc<dyn lr_api_keys::KeychainStorage> = Arc::new(lr_api_keys::MockKeychain::new());
    let app = axum::Router::new()
        .route("/v1/audio/transcriptions", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            move |State(state): State<AppState>, request: Request, next: Next| {
                let storage = storage.clone();
                async move {
                    match run(state, request, next, storage).await {
                        Ok(response) => response,
                        Err(error) => error.into_response(),
                    }
                }
            },
        ))
        .layer(Extension(AuthContext {
            api_key_id: owner.clone(),
            credential_scope: None,
            model_selection: None,
        }));
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("content-type", "multipart/form-data; boundary=b")
        .body(Body::from("--b\r\nmultipart-marker\r\n--b--\r\n"))
        .unwrap();
    let (status, response) = text(app.oneshot(request).await.unwrap()).await;
    assert_eq!(status, 200, "{response}");
    let forwarded = seen.lock().unwrap()[0].clone();
    assert!(!forwarded.contains(VALUE), "{forwarded}");
    assert!(forwarded.contains("localrouter-redacted"));
    assert!(response.contains(VALUE), "{response}");
}

#[tokio::test]
async fn form_scans_follow_each_action_outside_the_boundary() {
    // Redact/Ask without the boundary's vault cannot mask: refused.
    for action in [
        lr_config::SecretScanAction::Redact,
        lr_config::SecretScanAction::Ask,
    ] {
        let (_root, state) = with_action(action.clone());
        let owner = state.config_manager.get().clients[0].id.clone();
        let mut prompt = secret();
        let error = crate::content_scan::scan_form_fields(&state, &owner, "m", &mut [&mut prompt])
            .await
            .unwrap_err();
        assert_eq!(
            error.error.error.code.as_deref(),
            Some("secret_scan_unmaskable"),
            "{action:?}"
        );
        assert_eq!(prompt, secret());
    }
    // Notify: unchanged, notified.
    let (_root, state) = with_action(lr_config::SecretScanAction::Notify);
    let owner = state.config_manager.get().clients[0].id.clone();
    let mut prompt = secret();
    crate::content_scan::scan_form_fields(&state, &owner, "m", &mut [&mut prompt])
        .await
        .unwrap();
    assert_eq!(prompt, secret());
    assert!(state
        .mcp_gateway
        .firewall_manager
        .list_pending()
        .iter()
        .any(|p| p.is_notify_only));
    // Off: nothing happens.
    let (_root, state) = with_action(lr_config::SecretScanAction::Off);
    let mut prompt = secret();
    crate::content_scan::scan_form_fields(&state, &owner, "m", &mut [&mut prompt])
        .await
        .unwrap();
    assert_eq!(prompt, secret());
    assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
}

fn upload(fields: &[(&str, Option<&str>, &[u8])]) -> Request {
    let mut body = Vec::new();
    for (name, file_name, data) in fields {
        let file = file_name
            .map(|f| format!("; filename=\"{f}\""))
            .unwrap_or_default();
        body.extend_from_slice(
            format!("--b\r\nContent-Disposition: form-data; name=\"{name}\"{file}\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--b--\r\n");
    Request::builder()
        .method("POST")
        .header("content-type", "multipart/form-data; boundary=b")
        .body(Body::from(body))
        .unwrap()
}

/// The real audio and image edit handlers scan their prompt before anything
/// is dispatched: Notify opens its notification, Redact without the
/// boundary's vault refuses.
#[tokio::test]
async fn audio_and_image_edit_handlers_scan_their_prompts() {
    use axum::extract::FromRequest;
    for route in ["transcriptions", "translations", "edits"] {
        for action in [
            lr_config::SecretScanAction::Notify,
            lr_config::SecretScanAction::Redact,
        ] {
            let (_root, state) = with_action(action.clone());
            let secret = secret();
            let file: (&str, Option<&str>, &[u8]) = if route == "edits" {
                ("image", Some("a.png"), b"png")
            } else {
                ("file", Some("a.wav"), b"wav")
            };
            let request = upload(&[
                ("model", None, b"provider/model"),
                ("prompt", None, secret.as_bytes()),
                file,
            ]);
            let multipart = axum::extract::Multipart::from_request(request, &())
                .await
                .unwrap();
            let auth = AuthContext {
                api_key_id: "internal-test".into(),
                credential_scope: None,
                model_selection: None,
            };
            let result = match route {
                "transcriptions" => {
                    crate::routes::audio_transcriptions(
                        State(state.clone()),
                        Extension(auth),
                        None,
                        multipart,
                    )
                    .await
                }
                "translations" => {
                    crate::routes::audio_translations(
                        State(state.clone()),
                        Extension(auth),
                        None,
                        multipart,
                    )
                    .await
                }
                _ => {
                    crate::routes::image_edits(
                        State(state.clone()),
                        Extension(auth),
                        None,
                        multipart,
                    )
                    .await
                }
            };
            if action == lr_config::SecretScanAction::Redact {
                let error = result.expect_err("refused");
                assert_eq!(
                    error.error.error.code.as_deref(),
                    Some("secret_scan_unmaskable"),
                    "{route}"
                );
            } else {
                // No provider is configured, so dispatch fails afterwards;
                // the notification shows the prompt was scanned first.
                assert!(
                    state
                        .mcp_gateway
                        .firewall_manager
                        .list_pending()
                        .iter()
                        .any(|p| p.is_secret_scan_request && p.is_notify_only),
                    "{route}"
                );
            }
        }
    }
}

/// An image edit the client may not make is refused before its prompt is
/// scanned: no Ask or Notify popup opens for a request that is refused.
#[tokio::test]
async fn a_refused_image_edit_opens_no_secret_scan_popup() {
    use axum::extract::FromRequest;
    for action in [
        lr_config::SecretScanAction::Ask,
        lr_config::SecretScanAction::Notify,
    ] {
        let (_root, state) = with_action(action.clone());
        state
            .config_manager
            .update(|config| {
                config.strategies[0].model_permissions.global = lr_config::PermissionState::Off;
                config.strategies[0].model_permissions.providers.clear();
                config.strategies[0].model_permissions.models.clear();
            })
            .unwrap();
        let owner = state.config_manager.get().clients[0].id.clone();
        let secret = secret();
        let request = upload(&[
            ("model", None, b"provider/model"),
            ("prompt", None, secret.as_bytes()),
            ("image", Some("a.png"), b"png"),
        ]);
        let multipart = axum::extract::Multipart::from_request(request, &())
            .await
            .unwrap();
        let storage: Arc<dyn lr_api_keys::KeychainStorage> =
            Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Arc::new(Vault::for_client(storage, &owner));
        let auth = AuthContext {
            api_key_id: owner,
            credential_scope: None,
            model_selection: None,
        };
        // Inside the boundary's vault scope, so Ask could mask and would
        // open its popup if the scan ran; the popup would block the handler.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::content_scan::with_form_vault(
                vault,
                crate::routes::image_edits(State(state.clone()), Extension(auth), None, multipart),
            ),
        )
        .await
        .expect("refused without waiting for a popup");
        let error = result.expect_err("refused");
        assert_eq!(
            error.status,
            axum::http::StatusCode::FORBIDDEN,
            "{action:?}"
        );
        assert_ne!(
            error.error.error.code.as_deref(),
            Some("secret_scan_unmaskable"),
            "{action:?}: refused for access, not by the scan"
        );
        assert!(
            !state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .iter()
                .any(|p| p.is_secret_scan_request),
            "{action:?}: no secret scan popup"
        );
        // The unscanned prompt is not recorded with the refusal.
        for summary in state.monitor_store.list(0, 100, None).events {
            let event = state.monitor_store.get(&summary.id).unwrap();
            assert!(
                !serde_json::to_string(&event).unwrap().contains(VALUE),
                "{action:?}: the prompt reached a monitor event"
            );
        }
    }
}
