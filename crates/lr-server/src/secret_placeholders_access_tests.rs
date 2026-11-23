//! Access before the Secret Scan: the boundary runs each route's access,
//! model permission and rate-limit refusals (`routes::access::precheck`)
//! before it scans, so a refused request opens no Secret Scan popup, records
//! no unscanned text and reaches no handler. Admitted requests are scanned as
//! before, and approval popups keep their order.
use super::*;
use crate::middleware::client_auth::ClientAuthContext;
use axum::{routing::post, Extension};
use lr_mcp::gateway::firewall::FirewallApprovalAction;
use serde_json::json;
use std::time::Duration;
use tower::ServiceExt;

const VALUE: &str = "Zq8vR2nL5tY7wB1xK4mP9sD3fH6jG0cV2bN8eT5a";

fn secret() -> String {
    format!("aws_secret_access_key={VALUE}")
}

type Seen = Arc<std::sync::Mutex<Vec<String>>>;

/// Every route the boundary scans, with the secret where the user's text
/// goes.
fn cases() -> Vec<(&'static str, Value)> {
    let secret = secret();
    let messages = json!([{"role": "user", "content": secret}]);
    vec![
        (
            "/v1/chat/completions",
            json!({"model": "p/m", "messages": messages}),
        ),
        ("/v1/completions", json!({"model": "p/m", "prompt": secret})),
        ("/v1/responses", json!({"model": "p/m", "input": secret})),
        (
            "/v1/messages",
            json!({"model": "p/m", "max_tokens": 16, "messages": messages}),
        ),
        (
            "/v1/systemone",
            json!({"model": "p/m", "state": secret,
                   "questions": {"q": {"type": "noul", "instructions": "i"}}}),
        ),
        ("/v1/embeddings", json!({"model": "p/m", "input": secret})),
        ("/v1/moderations", json!({"input": secret})),
        (
            "/v1/rerank",
            json!({"model": "p/m", "query": "which", "documents": ["a", secret]}),
        ),
        ("/v1/search", json!({"model": "p/m", "query": secret})),
        (
            "/v1/ocr",
            json!({"model": "p/m", "document": {"document_url": "https://example.com/d.pdf"},
                   "pages": "0", "filename": secret}),
        ),
        (
            "/v1/images/generations",
            json!({"model": "p/m", "prompt": secret}),
        ),
        (
            "/v1/videos",
            json!({"model": "p/m", "prompt": secret, "seconds": 4}),
        ),
        (
            "/v1/messages/count_tokens",
            json!({"model": "p/m", "messages": messages}),
        ),
        (
            "/v1/audio/speech",
            json!({"model": "p/m", "input": secret, "voice": "alloy"}),
        ),
    ]
}

/// A state whose client may use every route: native operations and ranked
/// search enabled.
fn admitted(action: lr_config::SecretScanAction) -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = super::boundary_tests::state(root.path());
    state
        .config_manager
        .update(|config| {
            config.secret_scanning.action = action;
            let security = &mut config.clients[0].security;
            security.native_operations = vec![
                lr_config::NativeOperation::Rerank,
                lr_config::NativeOperation::Ocr,
                lr_config::NativeOperation::Video,
            ];
            security.upstream_tools.allowed_tools = vec![lr_config::UpstreamToolKind::WebSearch];
        })
        .unwrap();
    (root, state)
}

fn owner(state: &AppState) -> String {
    state.config_manager.get().clients[0].id.clone()
}

/// The boundary in front of handlers that record what they receive.
fn echo_app(state: AppState, seen: Seen) -> axum::Router {
    let owner = owner(&state);
    let echo = move |body: axum::body::Bytes| {
        let seen = seen.clone();
        async move {
            seen.lock()
                .unwrap()
                .push(String::from_utf8(body.to_vec()).unwrap());
            axum::Json(json!({"ok": true}))
        }
    };
    let mut router = axum::Router::new();
    for (path, _) in cases() {
        router = router.route(path, post(echo.clone()));
    }
    boundary(router, state, owner)
}

fn boundary(router: axum::Router, state: AppState, owner: String) -> axum::Router {
    let storage: Arc<dyn lr_api_keys::KeychainStorage> = Arc::new(lr_api_keys::MockKeychain::new());
    router
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
        .layer(Extension(ClientAuthContext {
            client_id: owner.clone(),
        }))
        .layer(Extension(AuthContext {
            api_key_id: owner,
            credential_scope: None,
            model_selection: None,
        }))
}

fn post_json(path: &str, body: &Value) -> Request {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn text(response: Response) -> (u16, String) {
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Refused {
    ClientDisabled,
    LlmOff,
    ModelNotAllowed,
    RateLimited,
}

impl Refused {
    fn apply(self, state: &AppState) {
        let owner = owner(state);
        match self {
            Refused::ClientDisabled => state
                .config_manager
                .update(|config| config.clients[0].enabled = false)
                .unwrap(),
            Refused::LlmOff => state
                .config_manager
                .update(|config| config.clients[0].llm_mode = lr_config::LlmMode::Off)
                .unwrap(),
            Refused::ModelNotAllowed => state
                .config_manager
                .update(|config| {
                    let permissions = &mut config.strategies[0].model_permissions;
                    permissions.global = lr_config::PermissionState::Off;
                    permissions.providers.clear();
                    permissions.models.clear();
                })
                .unwrap(),
            Refused::RateLimited => state.rate_limiter.add_api_key_limiters(
                owner,
                vec![lr_router::rate_limit::RateLimiter::new(
                    lr_router::rate_limit::RateLimitType::Requests,
                    0.0,
                    60,
                )],
            ),
        }
    }

    /// Whether the handler of `path` makes this check at all.
    fn applies_to(self, path: &str) -> bool {
        match self {
            Refused::ClientDisabled | Refused::LlmOff => true,
            Refused::ModelNotAllowed => path != "/v1/moderations",
            Refused::RateLimited => !matches!(
                path,
                "/v1/moderations" | "/v1/images/generations" | "/v1/audio/speech"
            ),
        }
    }
}

fn monitor_holds_the_secret(state: &AppState) -> bool {
    state
        .monitor_store
        .list(0, 1000, None)
        .events
        .iter()
        .any(|summary| {
            let event = state.monitor_store.get(&summary.id).unwrap();
            serde_json::to_string(&event).unwrap().contains(VALUE)
        })
}

fn secret_popups(state: &AppState) -> usize {
    state
        .mcp_gateway
        .firewall_manager
        .list_pending()
        .iter()
        .filter(|p| p.is_secret_scan_request)
        .count()
}

/// For every route and every refusal it makes, in Ask, Notify and Redact:
/// the boundary answers with the refusal, without a popup (Ask would block
/// the request), without the secret in any monitor event, and without
/// reaching the handler.
#[tokio::test]
async fn refused_requests_open_no_popup_and_record_no_secret() {
    for refused in [
        Refused::ClientDisabled,
        Refused::LlmOff,
        Refused::ModelNotAllowed,
        Refused::RateLimited,
    ] {
        for action in [
            lr_config::SecretScanAction::Ask,
            lr_config::SecretScanAction::Notify,
            lr_config::SecretScanAction::Redact,
        ] {
            for (path, body) in cases() {
                if !refused.applies_to(path) {
                    continue;
                }
                let inference = matches!(
                    path,
                    "/v1/chat/completions"
                        | "/v1/completions"
                        | "/v1/responses"
                        | "/v1/messages"
                        | "/v1/systemone"
                );
                if inference && action == lr_config::SecretScanAction::Notify {
                    // Not read by the boundary in Notify: the handler checks
                    // access before its own scan.
                    continue;
                }
                let (_root, state) = admitted(action.clone());
                refused.apply(&state);
                let seen = Seen::default();
                let (status, response) = tokio::time::timeout(
                    Duration::from_secs(5),
                    echo_app(state.clone(), seen.clone()).oneshot(post_json(path, &body)),
                )
                .await
                .unwrap_or_else(|_| panic!("{refused:?} {action:?} {path}: waited for a popup"))
                .map(text)
                .unwrap()
                .await;
                let case = format!("{refused:?} {action:?} {path}");
                assert!((400..500).contains(&status), "{case}: {status} {response}");
                if refused == Refused::RateLimited {
                    assert_eq!(status, 429, "{case}: {response}");
                }
                assert!(!response.contains("secret_scan"), "{case}: {response}");
                assert!(seen.lock().unwrap().is_empty(), "{case}: forwarded");
                assert_eq!(secret_popups(&state), 0, "{case}: a popup opened");
                assert!(!monitor_holds_the_secret(&state), "{case}: secret recorded");
            }
        }
    }
}

/// A refusal is recorded as the handler records it, without the content:
/// the LLM call with its error, and a rate-limit event for a rate limit.
#[tokio::test]
async fn a_refusal_is_recorded_without_the_request_content() {
    let (_root, state) = admitted(lr_config::SecretScanAction::Ask);
    Refused::ModelNotAllowed.apply(&state);
    let (status, _) = text(
        echo_app(state.clone(), Seen::default())
            .oneshot(post_json("/v1/embeddings", &cases()[5].1))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 403);
    let events = state.monitor_store.list(0, 100, None).events;
    let call = events
        .iter()
        .find(|e| e.event_type == lr_monitor::MonitorEventType::LlmCall)
        .expect("the refused call is recorded");
    let call = serde_json::to_string(&state.monitor_store.get(&call.id).unwrap()).unwrap();
    assert!(call.contains("/v1/embeddings"), "{call}");
    assert!(call.contains("is not enabled for this client"), "{call}");

    let (_root, state) = admitted(lr_config::SecretScanAction::Ask);
    Refused::RateLimited.apply(&state);
    let (status, _) = text(
        echo_app(state.clone(), Seen::default())
            .oneshot(post_json("/v1/chat/completions", &cases()[0].1))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 429);
    let events = state.monitor_store.list(0, 100, None).events;
    assert!(events
        .iter()
        .any(|e| e.event_type == lr_monitor::MonitorEventType::RateLimitEvent));
    assert!(events
        .iter()
        .any(|e| e.event_type == lr_monitor::MonitorEventType::LlmCall));
    assert!(!monitor_holds_the_secret(&state));
}

/// Admitted requests are still scanned: Ask asks on every route (and a
/// denial stops the request), Redact masks before the handler.
#[tokio::test]
async fn admitted_requests_are_still_asked_and_masked() {
    for (path, body) in cases() {
        let (_root, state) = admitted(lr_config::SecretScanAction::Ask);
        let seen = Seen::default();
        let app = echo_app(state.clone(), seen.clone());
        let request = post_json(path, &body);
        let task = tokio::spawn(async move { text(app.oneshot(request).await.unwrap()).await });
        let pending = tokio::time::timeout(Duration::from_secs(5), async {
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
        .unwrap_or_else(|_| panic!("{path}: an approval is asked for"));
        state
            .mcp_gateway
            .firewall_manager
            .submit_response(&pending.request_id, FirewallApprovalAction::Deny, None)
            .unwrap();
        let (status, response) = task.await.unwrap();
        assert_eq!(status, 403, "{path}: {response}");
        assert!(response.contains("secret_scan_denied_by_user"), "{path}");
        assert!(seen.lock().unwrap().is_empty(), "{path}");

        let (_root, state) = admitted(lr_config::SecretScanAction::Redact);
        let seen = Seen::default();
        let (status, response) = text(
            echo_app(state, seen.clone())
                .oneshot(post_json(path, &body))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200, "{path}: {response}");
        let forwarded = seen.lock().unwrap()[0].clone();
        assert!(!forwarded.contains(VALUE), "{path}: {forwarded}");
    }
}

async fn next_popup(state: &AppState) -> lr_mcp::gateway::firewall::PendingApprovalInfo {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(pending) = state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .into_iter()
                .find(|p| !p.is_notify_only)
            {
                break pending;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a popup opens")
}

/// Where a request needs both, the Secret Scan Ask popup comes before the
/// model's Ask popup (the real chat handler).
#[tokio::test]
async fn the_secret_scan_popup_comes_before_the_model_popup() {
    let (_root, state) = admitted(lr_config::SecretScanAction::Ask);
    state
        .config_manager
        .update(|config| {
            config.strategies[0].auto_config = Some(lr_config::AutoModelConfig {
                permission: lr_config::PermissionState::Ask,
                ..Default::default()
            });
        })
        .unwrap();
    let app = boundary(
        axum::Router::new()
            .route(
                "/v1/chat/completions",
                post(crate::routes::chat_completions),
            )
            .with_state(state.clone()),
        state.clone(),
        owner(&state),
    );
    let request = post_json("/v1/chat/completions", &cases()[0].1);
    let mut task = tokio::spawn(async move { text(app.oneshot(request).await.unwrap()).await });

    let first = next_popup(&state).await;
    assert!(first.is_secret_scan_request, "{first:?}");
    assert!(!first.is_model_request);
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &first.request_id,
            FirewallApprovalAction::ReplaceSecrets,
            None,
        )
        .unwrap();

    let second = tokio::select! {
        popup = next_popup(&state) => popup,
        done = &mut task => panic!("answered without a model popup: {done:?}"),
    };
    assert!(second.is_model_request, "{second:?}");
    assert!(!second.is_secret_scan_request);
    // The model popup shows the masked request.
    assert!(
        !second
            .full_arguments
            .as_deref()
            .unwrap_or("")
            .contains(VALUE),
        "{second:?}"
    );
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(&second.request_id, FirewallApprovalAction::Deny, None)
        .unwrap();
    let (status, response) = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, 403, "{response}");
    assert!(!monitor_holds_the_secret(&state));
}

/// In the handler too, refusals come before approval popups: a client
/// without LLM access is refused before the auto-router popup would open
/// (Notify, where the boundary does not read chat requests).
#[tokio::test]
async fn the_handler_refuses_before_the_auto_router_popup() {
    let (_root, state) = admitted(lr_config::SecretScanAction::Notify);
    state
        .config_manager
        .update(|config| {
            config.clients[0].llm_mode = lr_config::LlmMode::Off;
            config.strategies[0].auto_config = Some(lr_config::AutoModelConfig {
                permission: lr_config::PermissionState::Ask,
                prioritized_models: vec![("p".into(), "m".into())],
                ..Default::default()
            });
        })
        .unwrap();
    let app = boundary(
        axum::Router::new()
            .route(
                "/v1/chat/completions",
                post(crate::routes::chat_completions),
            )
            .with_state(state.clone()),
        state.clone(),
        owner(&state),
    );
    let body =
        json!({"model": "localrouter/auto", "messages": [{"role": "user", "content": secret()}]});
    let (status, response) = tokio::time::timeout(
        Duration::from_secs(5),
        app.oneshot(post_json("/v1/chat/completions", &body)),
    )
    .await
    .expect("refused without waiting for a popup")
    .map(text)
    .unwrap()
    .await;
    assert_eq!(status, 403, "{response}");
    assert!(state.mcp_gateway.firewall_manager.list_pending().is_empty());
}

/// Notify on a content route whose handler runs the turn pipeline notifies
/// once (the pipeline's scan), not at the boundary and again in the
/// pipeline (the real token-counting handler).
#[tokio::test]
async fn notify_on_a_pipeline_content_route_notifies_once() {
    let (_root, state) = admitted(lr_config::SecretScanAction::Notify);
    let app = boundary(
        axum::Router::new()
            .route(
                "/v1/messages/count_tokens",
                post(crate::routes::messages::count_tokens),
            )
            .with_state(state.clone()),
        state.clone(),
        owner(&state),
    );
    // No provider answers; the scan runs before dispatch.
    let (status, response) = text(
        app.oneshot(post_json("/v1/messages/count_tokens", &cases()[12].1))
            .await
            .unwrap(),
    )
    .await;
    // No provider is configured: refused at dispatch, after the scan.
    assert_ne!(status, 200, "{response}");
    assert!(!response.contains("Access denied"), "{response}");
    let notifications = state
        .mcp_gateway
        .firewall_manager
        .list_pending()
        .iter()
        .filter(|p| p.is_secret_scan_request && p.is_notify_only)
        .count();
    assert_eq!(notifications, 1);
    let scans = state
        .monitor_store
        .list(0, 100, None)
        .events
        .iter()
        .filter(|e| e.event_type == lr_monitor::MonitorEventType::SecretScan)
        .count();
    assert_eq!(scans, 1);
}

/// The per-model firewall reads the permissions of the strategy the client
/// uses (`strategy_id`), as the model allow-list check does. A strategy the
/// client does not own (no `parent`) that allows the model is not turned
/// into a denial by the firewall.
#[tokio::test]
async fn the_model_firewall_reads_the_clients_strategy() {
    let (_root, state) = admitted(lr_config::SecretScanAction::Off);
    assert!(state.config_manager.get().strategies[0].parent.is_none());
    let app = boundary(
        axum::Router::new()
            .route(
                "/v1/chat/completions",
                post(crate::routes::chat_completions),
            )
            .with_state(state.clone()),
        state.clone(),
        owner(&state),
    );
    let (status, response) = text(
        app.oneshot(post_json("/v1/chat/completions", &cases()[0].1))
            .await
            .unwrap(),
    )
    .await;
    // No provider answers, so the request fails at dispatch, not in the
    // firewall.
    assert_ne!(status, 200, "{response}");
    assert!(!response.contains("Access denied"), "{status}: {response}");
}

fn video_form(model: &str, prompt: &str) -> Request {
    let mut body = String::new();
    for (name, value) in [("model", model), ("prompt", prompt), ("seconds", "4")] {
        body.push_str(&format!(
            "--b\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str("--b--\r\n");
    Request::builder()
        .method("POST")
        .uri("/v1/videos")
        .header("content-type", "multipart/form-data; boundary=b")
        .header("idempotency-key", "k")
        .body(Body::from(body))
        .unwrap()
}

/// Multipart video (scanned by its handler): a request whose model the
/// client may not use is refused before its prompt is scanned, in Ask and
/// Notify.
#[tokio::test]
async fn a_refused_multipart_video_opens_no_popup() {
    for action in [
        lr_config::SecretScanAction::Ask,
        lr_config::SecretScanAction::Notify,
    ] {
        let (_root, state) = admitted(action.clone());
        Refused::ModelNotAllowed.apply(&state);
        let app = boundary(
            axum::Router::new()
                .route("/v1/videos", post(crate::video_jobs::create))
                .with_state(state.clone()),
            state.clone(),
            owner(&state),
        );
        let (status, response) = tokio::time::timeout(
            Duration::from_secs(5),
            app.oneshot(video_form("p/m", &secret())),
        )
        .await
        .unwrap_or_else(|_| panic!("{action:?}: waited for a popup"))
        .map(text)
        .unwrap()
        .await;
        assert_eq!(status, 403, "{action:?}: {response}");
        assert!(
            response.contains("is not enabled for this client"),
            "{response}"
        );
        assert_eq!(secret_popups(&state), 0, "{action:?}");
        assert!(!monitor_holds_the_secret(&state), "{action:?}");
    }
}
