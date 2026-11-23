use super::*;
use axum::{routing::post, Router};
use std::sync::atomic::AtomicUsize;
use tower::ServiceExt;

const KIB: usize = 1024;
const MIB_U: usize = 1024 * 1024;

fn config(limit: u64) -> Arc<lr_config::ConfigManager> {
    let mut config = lr_config::AppConfig::default();
    config.server.max_request_body_bytes = limit;
    Arc::new(lr_config::ConfigManager::new(
        config,
        std::env::temp_dir().join(format!("lr-body-limits-{}.yaml", uuid::Uuid::new_v4())),
    ))
}

/// Routes that count calls; `/hold` keeps its response open until `release`.
fn app(limits: BodyLimits, calls: Arc<AtomicUsize>, release: Arc<tokio::sync::Notify>) -> Router {
    let echo_calls = calls.clone();
    let echo = move |body: Bytes| {
        let calls = echo_calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            body.len().to_string()
        }
    };
    let hold = move |body: Bytes| {
        let calls = calls.clone();
        let release = release.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let len = body.len();
            let stream = async_stream::stream! {
                release.notified().await;
                yield Ok::<_, std::io::Error>(Bytes::from(len.to_string()));
            };
            Body::from_stream(stream)
        }
    };
    Router::new()
        .route("/v1/chat/completions", post(echo.clone()))
        .route("/v1/files", post(echo))
        .route("/hold", post(hold))
        .layer(axum::middleware::from_fn_with_state(limits, middleware))
        .layer(axum::extract::DefaultBodyLimit::disable())
}

fn sized(path: &str, len: usize) -> Request {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-length", len)
        .body(Body::from(vec![b'a'; len]))
        .unwrap()
}

fn chunked(path: &str, len: usize) -> Request {
    let chunks: Vec<_> = (0..len)
        .step_by(16 * KIB)
        .map(|start| Ok::<_, std::io::Error>(Bytes::from(vec![b'a'; (len - start).min(16 * KIB)])))
        .collect();
    Request::builder()
        .method("POST")
        .uri(path)
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap()
}

async fn json(response: Response) -> (StatusCode, Option<String>, serde_json::Value) {
    let status = response.status();
    let retry = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().unwrap().to_owned());
    let body = axum::body::to_bytes(response.into_body(), 64 * KIB)
        .await
        .unwrap();
    (
        status,
        retry,
        serde_json::from_slice(&body).unwrap_or_default(),
    )
}

#[tokio::test]
async fn over_limit_bodies_get_a_structured_413_whether_declared_or_chunked() {
    let calls = Arc::new(AtomicUsize::new(0));
    let limits = BodyLimits::new(config(lr_config::MIN_REQUEST_BODY_BYTES));
    let app = app(limits, calls.clone(), Arc::new(tokio::sync::Notify::new()));
    let ok = app
        .clone()
        .oneshot(sized("/v1/chat/completions", MIB_U))
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let ok = app
        .clone()
        .oneshot(chunked("/v1/chat/completions", MIB_U))
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let (status, _, value) = json(
        app.clone()
            .oneshot(sized("/v1/chat/completions", MIB_U + 1))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 413);
    assert_eq!(value["error"]["code"], "request_too_large");
    assert_eq!(
        value["error"]["message"],
        "Request body exceeds the 1 MiB limit"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "declared overflow is refused unread"
    );

    // A chunked body reaches the handler, whose extractor fails; the
    // response is the structured 413, not the extractor's own error.
    let (status, _, value) = json(
        app.clone()
            .oneshot(chunked("/v1/chat/completions", MIB_U + 1))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 413);
    assert_eq!(value["error"]["code"], "request_too_large");
}

#[tokio::test]
async fn limit_changes_apply_to_the_next_request_without_a_restart() {
    let config = config(lr_config::MIN_REQUEST_BODY_BYTES);
    let app = app(
        BodyLimits::new(config.clone()),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(tokio::sync::Notify::new()),
    );
    let len = 3 * MIB_U / 2;
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", len))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 413);
    config
        .update(|c| c.server.max_request_body_bytes = 2 * MIB_U as u64)
        .unwrap();
    let status = app
        .oneshot(sized("/v1/chat/completions", len))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
}

#[tokio::test]
async fn file_uploads_keep_their_own_limit() {
    // A 16 MiB file plus multipart framing must fit whatever the JSON limit
    // is; the former shared 16 MiB transport cap refused it.
    let app = app(
        BodyLimits::new(config(lr_config::MIN_REQUEST_BODY_BYTES)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(tokio::sync::Notify::new()),
    );
    let fits = app
        .clone()
        .oneshot(sized("/v1/files", lr_batches::MAX_FILE_BYTES + KIB))
        .await
        .unwrap()
        .status();
    assert_eq!(fits, 200);
    let over = app
        .oneshot(sized("/v1/files", FILE_UPLOAD_LIMIT + 1))
        .await
        .unwrap()
        .status();
    assert_eq!(over, 413);
}

#[tokio::test]
async fn fixed_route_limits_ignore_the_json_limit_and_share_the_budget() {
    let limits = BodyLimits::with_budget(
        config(lr_config::MIN_REQUEST_BODY_BYTES),
        1024 * KIB,
        64 * KIB,
        Duration::from_millis(100),
    );
    let upload = limits.with_fixed_limit(2 * MIB_U);
    let release = Arc::new(tokio::sync::Notify::new());
    let json_app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        release.clone(),
    );
    let upload_app = app(upload, Arc::new(AtomicUsize::new(0)), release.clone());
    let over_json = 3 * MIB_U / 2;
    let status = upload_app
        .clone()
        .oneshot(sized("/v1/chat/completions", over_json))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200, "the fixed limit replaces the 1 MiB JSON limit");
    let (status, _, value) = json(
        upload_app
            .clone()
            .oneshot(sized("/v1/chat/completions", 2 * MIB_U + 1))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 413);
    assert_eq!(
        value["error"]["message"],
        "Request body exceeds the 2 MiB limit"
    );

    // A held upload's reservation counts against JSON requests too.
    let held = upload_app.oneshot(sized("/hold", 768 * KIB)).await.unwrap();
    assert_eq!(limits.available_bytes(), 256 * KIB);
    let status = json_app
        .oneshot(sized("/v1/chat/completions", 512 * KIB))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 503);
    release.notify_one();
    drop(axum::body::to_bytes(held.into_body(), KIB).await.unwrap());
    assert_eq!(limits.available_bytes(), 1024 * KIB);
}

#[tokio::test]
async fn large_bodies_share_a_budget_held_until_the_response_ends() {
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let limits = BodyLimits::with_budget(
        config(lr_config::MIN_REQUEST_BODY_BYTES),
        1024 * KIB,
        64 * KIB,
        Duration::from_millis(100),
    );
    let app = app(limits.clone(), calls.clone(), release.clone());

    // Small bodies never touch the budget.
    let small = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 64 * KIB))
        .await
        .unwrap();
    assert_eq!(small.status(), 200);
    assert_eq!(limits.available_bytes(), 1024 * KIB);

    // A response still being sent keeps its request's reservation.
    let held = app
        .clone()
        .oneshot(sized("/hold", 768 * KIB))
        .await
        .unwrap();
    assert_eq!(held.status(), 200);
    assert_eq!(limits.available_bytes(), 256 * KIB);

    // Declared length: refused before the body is read.
    let before = calls.load(Ordering::SeqCst);
    let (status, retry, value) = json(
        app.clone()
            .oneshot(sized("/v1/chat/completions", 512 * KIB))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(retry.as_deref(), Some("2"));
    assert_eq!(value["error"]["code"], "request_body_budget_exhausted");
    assert_eq!(calls.load(Ordering::SeqCst), before);

    // Chunked: reserved as it grows, refused once the wait runs out.
    let (status, retry, value) = json(
        app.clone()
            .oneshot(chunked("/v1/chat/completions", 512 * KIB))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(retry.as_deref(), Some("2"));
    assert_eq!(value["error"]["code"], "request_body_budget_exhausted");
    assert_eq!(
        limits.available_bytes(),
        256 * KIB,
        "a refused body releases what it reserved"
    );

    // Finishing the held response returns its reservation.
    release.notify_one();
    let body = axum::body::to_bytes(held.into_body(), KIB).await.unwrap();
    assert_eq!(&body[..], (768 * KIB).to_string().as_bytes());
    assert_eq!(limits.available_bytes(), 1024 * KIB);
    for request in [
        chunked("/v1/chat/completions", 512 * KIB),
        sized("/v1/chat/completions", 512 * KIB),
    ] {
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), 200);
    }
    assert_eq!(limits.available_bytes(), 1024 * KIB);
}

#[tokio::test]
async fn a_body_larger_than_the_budget_runs_alone() {
    let limits = BodyLimits::with_budget(
        config(lr_config::MIN_REQUEST_BODY_BYTES),
        256 * KIB,
        64 * KIB,
        Duration::from_millis(100),
    );
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(tokio::sync::Notify::new()),
    );
    for request in [
        chunked("/v1/chat/completions", 512 * KIB),
        sized("/v1/chat/completions", 512 * KIB),
    ] {
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), 200);
    }
    assert_eq!(limits.available_bytes(), 256 * KIB);
}

#[tokio::test]
async fn a_waiting_body_proceeds_when_budget_frees_in_time() {
    let release = Arc::new(tokio::sync::Notify::new());
    let limits = BodyLimits::with_budget(
        config(lr_config::MIN_REQUEST_BODY_BYTES),
        1024 * KIB,
        64 * KIB,
        Duration::from_secs(10),
    );
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        release.clone(),
    );
    let held = app
        .clone()
        .oneshot(sized("/hold", 768 * KIB))
        .await
        .unwrap();
    let waiting = tokio::spawn(app.oneshot(chunked("/v1/chat/completions", 512 * KIB)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    release.notify_one();
    drop(axum::body::to_bytes(held.into_body(), KIB).await.unwrap());
    assert_eq!(waiting.await.unwrap().unwrap().status(), 200);
    assert_eq!(limits.available_bytes(), 1024 * KIB);
}
