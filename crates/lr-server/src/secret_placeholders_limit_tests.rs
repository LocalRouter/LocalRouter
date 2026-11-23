//! Inspection-budget and memory tests for the native request boundary.
use super::*;
use crate::routes::gateway_json::GatewayJson;
use axum::{routing::post, Extension, Json};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use tower::ServiceExt;

/// Counts live and peak heap bytes per thread, so a single-threaded test can
/// measure its own request without noise from tests running in parallel.
struct Counting;
thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}
fn account(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get() + delta;
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}
// SAFETY: every call forwards to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        account(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        account(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        account(layout.size() as isize);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        account(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn reset_peak() -> isize {
    let live = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(live));
    live
}
fn peak() -> isize {
    PEAK.with(Cell::get)
}

fn boundary(state: AppState) -> axum::Router {
    let id = state.config_manager.get().clients[0].id.clone();
    let storage: Arc<dyn lr_api_keys::KeychainStorage> = Arc::new(lr_api_keys::MockKeychain::new());
    axum::Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |GatewayJson(request): GatewayJson<crate::types::ChatCompletionRequest>| async move {
                    Json(serde_json::json!({"messages": request.messages.len()}))
                },
            ),
        )
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
            api_key_id: id,
            credential_scope: None,
            model_selection: None,
        }))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
}

/// A synthetic conversation of `messages` turns of `turn` bytes each.
fn conversation(messages: usize, turn: usize) -> Vec<u8> {
    let text: String = "lorem ipsum dolor sit amet "
        .chars()
        .cycle()
        .take(turn)
        .collect();
    let messages: Vec<_> = (0..messages)
        .map(|i| {
            serde_json::json!({"role": if i % 2 == 0 { "user" } else { "assistant" }, "content": text})
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({"model":"fake/model","messages":messages})).unwrap()
}

/// Peak heap growth while the boundary and the typed extractor handle one
/// request, relative to the moment the request body already exists.
async fn measure(
    action: lr_config::SecretScanAction,
    messages: usize,
    turn: usize,
    chunked: bool,
) -> (usize, isize) {
    let root = tempfile::tempdir().unwrap();
    let state = boundary_tests::state(root.path());
    state
        .config_manager
        .update(|config| config.secret_scanning.action = action)
        .unwrap();
    let app = boundary(state);
    let body = axum::body::Bytes::from(conversation(messages, turn));
    let size = body.len();
    let builder = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    let request = if chunked {
        // Slices share the allocation, so the body is counted once.
        let chunks: Vec<_> = (0..size)
            .step_by(64 * 1024)
            .map(|start| Ok::<_, std::io::Error>(body.slice(start..(start + 64 * 1024).min(size))))
            .collect();
        builder.body(Body::from_stream(futures::stream::iter(chunks)))
    } else {
        builder
            .header("content-length", size)
            .body(Body::from(body))
    }
    .unwrap();
    let base = reset_peak();
    let response = app.oneshot(request).await.unwrap();
    let growth = peak() - base;
    assert_eq!(response.status(), 200);
    (size, growth)
}

/// A boundary whose only limit is its own inspection budget (no transport
/// limit in front): forces the divergence the transport layer normally hides.
fn unlimited_boundary(state: AppState, calls: Arc<std::sync::atomic::AtomicUsize>) -> axum::Router {
    let id = state.config_manager.get().clients[0].id.clone();
    let storage: Arc<dyn lr_api_keys::KeychainStorage> = Arc::new(lr_api_keys::MockKeychain::new());
    axum::Router::new()
        .route(
            "/v1/chat/completions",
            post(move |body: axum::body::Bytes| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { body.len().to_string() }
            }),
        )
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
            api_key_id: id,
            credential_scope: None,
            model_selection: None,
        }))
        .layer(axum::extract::DefaultBodyLimit::disable())
}

fn oversized_secret_request(limit: usize, chunked: bool) -> Request {
    let secret = "aws_secret_access_key=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    let padding = "x".repeat(limit);
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "fake/model",
        "messages": [{"role": "user", "content": format!("{secret} {padding}")}],
    }))
    .unwrap();
    let builder = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    if chunked {
        let chunks: Vec<_> = body
            .chunks(64 * 1024)
            .map(|chunk| Ok::<_, std::io::Error>(axum::body::Bytes::copy_from_slice(chunk)))
            .collect();
        builder.body(Body::from_stream(futures::stream::iter(chunks)))
    } else {
        builder
            .header("content-length", body.len())
            .body(Body::from(body))
    }
    .unwrap()
}

#[tokio::test]
async fn over_budget_body_is_refused_not_forwarded_unscanned_when_limits_diverge() {
    let limit = lr_config::MIN_REQUEST_BODY_BYTES as usize;
    for action in [
        lr_config::SecretScanAction::Redact,
        lr_config::SecretScanAction::Ask,
        lr_config::SecretScanAction::Notify,
        lr_config::SecretScanAction::Off,
    ] {
        for chunked in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let state = boundary_tests::state(root.path());
            state
                .config_manager
                .update(|config| {
                    config.secret_scanning.action = action.clone();
                    config.server.max_request_body_bytes = limit as u64;
                })
                .unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let response = unlimited_boundary(state, calls.clone())
                .oneshot(oversized_secret_request(limit, chunked))
                .await
                .unwrap();
            assert_eq!(response.status(), 413, "{action:?} chunked={chunked}");
            let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap();
            let error: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["error"]["code"], "request_too_large");
            assert!(!String::from_utf8_lossy(&body).contains("wJalrXUtnFEMI"));
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{action:?} chunked={chunked}: nothing reaches the endpoint"
            );
        }
    }
}

#[tokio::test]
async fn compressed_inference_body_is_refused_before_buffering() {
    let root = tempfile::tempdir().unwrap();
    let state = boundary_tests::state(root.path());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = unlimited_boundary(state, calls.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("content-encoding", "gzip")
                .body(Body::from(vec![0x1f, 0x8b, 0x08, 0x00]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 415);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "unsupported_content_encoding");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unreadable_body_is_refused_not_forwarded_as_a_prefix() {
    let root = tempfile::tempdir().unwrap();
    let state = boundary_tests::state(root.path());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let chunks = vec![
        Ok(axum::body::Bytes::from_static(br#"{"model":"m","messages":[{"role":"user","content":"aws_secret_access_key=wJalrXUtnFEMI/K7MDENG"#)),
        Err(std::io::Error::other("connection reset")),
    ];
    let response = unlimited_boundary(state, calls.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from_stream(futures::stream::iter(chunks)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// Authentication runs before the body limit and budget: an unauthenticated
/// caller cannot occupy the large-body budget or learn the limit.
#[tokio::test]
async fn unauthenticated_oversized_request_is_refused_by_auth_first() {
    let root = tempfile::tempdir().unwrap();
    let state = boundary_tests::built_state(root.path());
    let app = crate::build_app(state, false, tokio_util::sync::CancellationToken::new());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .header("content-length", 512 * 1024 * 1024)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

/// Prints peak memory per request for review; asserts a loose upper bound
/// so a regression to several whole-body copies fails.
#[tokio::test(flavor = "current_thread")]
async fn native_boundary_peak_memory_per_request() {
    for (messages, turn) in [(512, 8 * 1024), (8 * 1024, 512)] {
        for chunked in [false, true] {
            for action in [
                lr_config::SecretScanAction::Redact,
                lr_config::SecretScanAction::Notify,
            ] {
                let (size, growth) = measure(action.clone(), messages, turn, chunked).await;
                let ratio = growth as f64 / size as f64;
                println!(
                    "native boundary peak: action={action:?} messages={messages} turn={turn} chunked={chunked} body={size} B peak_growth={growth} B ratio={ratio:.2}x"
                );
                // Measured at 1.05-2.77x (Notify) and 2.16-5.29x (Redact,
                // which also extracts and scans the text); the earlier
                // triple-copy boundary measured 2.10-5.41x and 4.21-8.94x.
                let bound = if action == lr_config::SecretScanAction::Notify {
                    3.0
                } else {
                    6.0
                };
                assert!(ratio < bound, "{action:?}: {ratio:.2}x >= {bound}x");
            }
        }
    }
}
