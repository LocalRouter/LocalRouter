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

/// Threshold, budget and wait come from `server.large_body_*` and apply to
/// the next request without a restart.
#[tokio::test]
async fn large_body_settings_are_read_live_from_the_configuration() {
    let manager = config(32 * MIB_U as u64);
    manager
        .update(|c| {
            c.server.large_body_threshold_bytes = MIB_U as u64;
            c.server.large_body_budget_bytes = 16 * MIB_U as u64;
            c.server.large_body_wait_ms = 50;
        })
        .unwrap();
    let limits = BodyLimits::new(manager.clone());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        release.clone(),
    );
    // 12 of the 16 MiB are held; a 6 MiB body waits 50 ms, then 503.
    let held = app
        .clone()
        .oneshot(sized("/hold", 12 * MIB_U))
        .await
        .unwrap();
    assert_eq!(limits.available_bytes(), 4 * MIB_U);
    let started = std::time::Instant::now();
    let (status, retry, _) = json(
        app.clone()
            .oneshot(sized("/v1/chat/completions", 6 * MIB_U))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(retry.as_deref(), Some("2"));
    assert!(started.elapsed() < Duration::from_secs(2));

    // Raising the threshold above the body: nothing is reserved.
    manager
        .update(|c| c.server.large_body_threshold_bytes = 8 * MIB_U as u64)
        .unwrap();
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 6 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);

    // A larger budget applies to new requests at once; the held request
    // keeps its permits and returns them to the same budget.
    manager
        .update(|c| c.server.large_body_budget_bytes = 64 * MIB_U as u64)
        .unwrap();
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 10 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
    assert_eq!(limits.available_bytes(), 52 * MIB_U);
    release.notify_one();
    drop(axum::body::to_bytes(held.into_body(), KIB).await.unwrap());
    assert_eq!(limits.available_bytes(), 64 * MIB_U);
}

/// A live budget of `budget` MiB, a 1 MiB threshold and a short wait.
fn live(budget: usize) -> (Arc<lr_config::ConfigManager>, BodyLimits) {
    let manager = config(64 * MIB_U as u64);
    manager
        .update(|c| {
            c.server.large_body_threshold_bytes = MIB_U as u64;
            c.server.large_body_budget_bytes = (budget * MIB_U) as u64;
            c.server.large_body_wait_ms = 50;
        })
        .unwrap();
    let limits = BodyLimits::new(manager.clone());
    (manager, limits)
}

/// Shrinking the budget below what in-flight requests hold admits no new
/// large body until the held bytes fall under the new budget.
#[tokio::test]
async fn a_shrunk_budget_admits_nothing_until_held_bytes_fit() {
    let (manager, limits) = live(32);
    let release = Arc::new(tokio::sync::Notify::new());
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        release.clone(),
    );
    let first = app
        .clone()
        .oneshot(sized("/hold", 12 * MIB_U))
        .await
        .unwrap();
    let second = app
        .clone()
        .oneshot(sized("/hold", 12 * MIB_U))
        .await
        .unwrap();
    assert_eq!(limits.available_bytes(), 8 * MIB_U);

    // 24 MiB held against a 16 MiB budget: 8 MiB of debt, nothing free.
    manager
        .update(|c| c.server.large_body_budget_bytes = 16 * MIB_U as u64)
        .unwrap();
    for request in [
        sized("/v1/chat/completions", 2 * MIB_U),
        chunked("/v1/chat/completions", 2 * MIB_U),
    ] {
        let status = app.clone().oneshot(request).await.unwrap().status();
        assert_eq!(status, 503, "the old budget's free bytes are gone");
    }
    assert_eq!(limits.available_bytes(), 0);
    assert_eq!(limits.debt_bytes(), 8 * MIB_U);

    // Releasing 12 MiB pays the debt first: 4 MiB free.
    release.notify_one();
    drop(axum::body::to_bytes(first.into_body(), KIB).await.unwrap());
    assert_eq!(limits.debt_bytes(), 0);
    assert_eq!(limits.available_bytes(), 4 * MIB_U);
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 6 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 503);
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 3 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);

    release.notify_one();
    drop(axum::body::to_bytes(second.into_body(), KIB).await.unwrap());
    assert_eq!(limits.available_bytes(), 16 * MIB_U);
    assert_eq!(limits.debt_bytes(), 0);
}

/// Growing the budget pays an outstanding debt before adding free bytes.
#[tokio::test]
async fn a_grown_budget_pays_its_debt_before_adding_bytes() {
    let (manager, limits) = live(32);
    let release = Arc::new(tokio::sync::Notify::new());
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        release.clone(),
    );
    let held = app
        .clone()
        .oneshot(sized("/hold", 24 * MIB_U))
        .await
        .unwrap();
    limits.resize_budget(16 * MIB_U);
    assert_eq!(limits.debt_bytes(), 8 * MIB_U);

    // 40 MiB: 24 held, 16 free.
    manager
        .update(|c| c.server.large_body_budget_bytes = 40 * MIB_U as u64)
        .unwrap();
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 14 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
    assert_eq!(limits.debt_bytes(), 0);
    assert_eq!(limits.available_bytes(), 16 * MIB_U);
    let status = app
        .clone()
        .oneshot(sized("/v1/chat/completions", 17 * MIB_U))
        .await
        .unwrap()
        .status();
    assert_eq!(status, 503);

    release.notify_one();
    drop(axum::body::to_bytes(held.into_body(), KIB).await.unwrap());
    assert_eq!(limits.available_bytes(), 40 * MIB_U);
}

/// A body larger than a budget that shrank while it was being read still
/// runs alone instead of waiting for permits the budget no longer has.
#[tokio::test]
async fn a_body_outgrowing_a_shrunk_budget_still_runs_alone() {
    let (manager, limits) = live(32);
    let app = app(
        limits.clone(),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(tokio::sync::Notify::new()),
    );
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .body(Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(receiver),
        ))
        .unwrap();
    let task = tokio::spawn(app.clone().oneshot(request));
    sender
        .send(Ok(Bytes::from(vec![b'a'; 2 * MIB_U])))
        .await
        .unwrap();
    // Wait until the first reservation is taken.
    while limits.available_bytes() == 32 * MIB_U {
        tokio::task::yield_now().await;
    }
    manager
        .update(|c| c.server.large_body_budget_bytes = 16 * MIB_U as u64)
        .unwrap();
    limits.resize_budget(16 * MIB_U);
    sender
        .send(Ok(Bytes::from(vec![b'a'; 22 * MIB_U])))
        .await
        .unwrap();
    drop(sender);
    let response = task.await.unwrap().unwrap();
    assert_eq!(response.status(), 200);
    // The whole (shrunk) budget is held until the response is finished.
    assert_eq!(limits.available_bytes(), 0);
    let body = axum::body::to_bytes(response.into_body(), KIB)
        .await
        .unwrap();
    assert_eq!(&body[..], (24 * MIB_U).to_string().as_bytes());
    assert_eq!(limits.available_bytes(), 16 * MIB_U);
    assert_eq!(limits.debt_bytes(), 0);
}

/// Under concurrent reservations and resizes the bytes held never exceed
/// the largest budget configured, and everything is accounted for at the
/// end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_resizes_never_overcommit_the_budget() {
    const BIG: usize = 64;
    const SMALL: usize = 16;
    let limits = BodyLimits::with_budget(config(MIB_U as u64), BIG * UNIT, UNIT, Duration::ZERO);
    let held = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut workers = Vec::new();
    for worker in 0..8usize {
        let (limits, held, peak, stop) = (limits.clone(), held.clone(), peak.clone(), stop.clone());
        workers.push(tokio::spawn(async move {
            let mut round = 0usize;
            while !stop.load(Ordering::SeqCst) {
                round += 1;
                let units = 1 + (worker * 7 + round) % 12;
                let semaphore = limits.budget.lock().semaphore.clone();
                let Ok(Ok(permit)) = tokio::time::timeout(
                    Duration::from_millis(5),
                    semaphore.acquire_many_owned(units as u32),
                )
                .await
                else {
                    continue;
                };
                let reservation = Reservation {
                    permit: Some(permit),
                    budget: limits.budget.clone(),
                };
                let now = held.fetch_add(units, Ordering::SeqCst) + units;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::task::yield_now().await;
                held.fetch_sub(units, Ordering::SeqCst);
                drop(reservation);
            }
        }));
    }
    for step in 0..400 {
        let units = if step % 2 == 0 { SMALL } else { BIG };
        limits.budget.lock().resize(units);
        tokio::task::yield_now().await;
    }
    stop.store(true, Ordering::SeqCst);
    for worker in workers {
        worker.await.unwrap();
    }
    assert!(
        peak.load(Ordering::SeqCst) <= BIG,
        "held {} permits against a budget of at most {BIG}",
        peak.load(Ordering::SeqCst)
    );
    let budget = limits.budget.lock();
    assert_eq!(budget.units, BIG);
    assert_eq!(budget.debt, 0);
    assert_eq!(budget.semaphore.available_permits(), BIG);
}

/// Permits partly assigned to a waiter that gives up return to the
/// semaphore directly; they pay an outstanding debt before the next request
/// can take them.
#[tokio::test]
async fn permits_of_an_abandoned_waiter_pay_the_debt_first() {
    let limits = BodyLimits::with_budget(config(MIB_U as u64), 10 * UNIT, UNIT, Duration::ZERO);
    let reserve = |units: u32| {
        let semaphore = limits.budget.lock().semaphore.clone();
        Reservation {
            permit: Some(semaphore.try_acquire_many_owned(units).unwrap()),
            budget: limits.budget.clone(),
        }
    };
    let held = reserve(6);
    let other = reserve(4);
    let semaphore = limits.budget.lock().semaphore.clone();
    let mut waiter = Box::pin(semaphore.clone().acquire_many_owned(8));
    assert!(futures::poll!(waiter.as_mut()).is_pending());
    // The 4 released permits go to the waiter, not to the free pool.
    drop(other);
    assert_eq!(limits.available_bytes(), 0);
    // Shrink to 4 while 6 are held: nothing free to remove, 6 of debt.
    limits.budget.lock().resize(4);
    assert_eq!(limits.debt_bytes(), 6 * UNIT);
    // The waiter gives up; its 4 permits come back free despite the debt.
    drop(waiter);
    assert_eq!(limits.available_bytes(), 4 * UNIT);
    // The next request settles them against the debt first.
    limits.resize_budget(4 * UNIT);
    assert_eq!(limits.available_bytes(), 0);
    assert_eq!(limits.debt_bytes(), 2 * UNIT);
    drop(held);
    assert_eq!(limits.available_bytes(), 4 * UNIT);
    assert_eq!(limits.debt_bytes(), 0);
}
