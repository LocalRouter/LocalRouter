//! A stand-in engine for tests. Behaviour is controlled by environment
//! variables so supervisor tests can simulate slow starts, loading, crashes
//! and auth without real models.
//!
//! - `--port N` or the env var named by `FAKE_PORT_VAR` sets the port; the
//!   env var named by `FAKE_ADDR_VAR` sets it as `host:port`
//! - `FAKE_KEY_VAR`: name of the env var holding the required API key
//! - `FAKE_START_DELAY_MS`: wait before binding (port closed meanwhile)
//! - `FAKE_LOADING_MS`: `/health` answers 503 for this long after binding
//! - `FAKE_EXIT_AFTER_MS`: exit with code 7 after this long
//! - `FAKE_FAIL_START=1`: print an error and exit 2 immediately
//! - `FAKE_PULL_DELAY_MS`: `/api/pull` waits this long before answering
//!
//! Ollaya-style routes (`/`, `/api/pull`, `/api/delete`, `/api/decide`) keep
//! models as manifest files under `OLLAYA_MODELS`, like Ollaya does.

use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

#[derive(Clone)]
struct AppState {
    ready_at: Instant,
    key: Option<String>,
    /// `OLLAYA_MODELS`, for the Ollaya-style routes.
    models_dir: Option<std::path::PathBuf>,
}

fn env_ms(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()?
        .parse()
        .ok()
        .map(Duration::from_millis)
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    match &state.key {
        None => true,
        Some(key) => headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == format!("Bearer {key}")),
    }
}

async fn health(State(s): State<AppState>) -> impl IntoResponse {
    if Instant::now() < s.ready_at {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": {"message": "Loading model"}})),
        )
    } else {
        (StatusCode::OK, Json(json!({"status": "ok"})))
    }
}

async fn guarded(s: &AppState, headers: &HeaderMap, body: Value) -> (StatusCode, Json<Value>) {
    if !authorized(s, headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad key"})));
    }
    (StatusCode::OK, Json(body))
}

async fn chat(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let model = req.get("model").cloned().unwrap_or(Value::Null);
    guarded(
        &s,
        &headers,
        json!({
            "id": "fake-1", "object": "chat.completion", "created": 0, "model": model,
            "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": "hello from fake engine"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
        }),
    )
    .await
}

async fn systemone(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let answers: serde_json::Map<String, Value> = req
        .get("questions")
        .and_then(Value::as_object)
        .map(|q| {
            q.keys()
                .map(|id| (id.clone(), json!({"type": "noul", "noul": 0.9})))
                .collect()
        })
        .unwrap_or_default();
    guarded(
        &s,
        &headers,
        json!({"model": req.get("model").cloned().unwrap_or(json!("fake")), "answers": answers,
               "usage": {"input_tokens": 10, "output_tokens": 0}}),
    )
    .await
}

/// OpenAI-style image generation, as `sd-server` answers it (base64 only):
/// one image per `n`, whose bytes are the prompt.
async fn images(Json(req): Json<Value>) -> impl IntoResponse {
    use base64::Engine as _;
    let prompt = req.get("prompt").and_then(Value::as_str).unwrap_or("");
    let n = req.get("n").and_then(Value::as_u64).unwrap_or(1);
    let b64 = base64::engine::general_purpose::STANDARD.encode(prompt);
    let data: Vec<Value> = (0..n).map(|_| json!({"b64_json": b64})).collect();
    Json(json!({"created": 1, "output_format": "png", "data": data, "size": req.get("size")}))
}

/// Multipart image edit, as `sd-server` answers it: one image per `n`, whose
/// bytes describe what arrived (`prompt|images=N|mask=yes|no|size=S`).
async fn image_edits(mut form: axum::extract::Multipart) -> impl IntoResponse {
    use base64::Engine as _;
    let (mut prompt, mut images, mut mask, mut n, mut size) =
        (String::new(), 0, false, 1u64, String::from("-"));
    while let Ok(Some(field)) = form.next_field().await {
        match field.name().unwrap_or("") {
            "prompt" => prompt = field.text().await.unwrap_or_default(),
            "n" => {
                n = field
                    .text()
                    .await
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(1)
            }
            "size" => size = field.text().await.unwrap_or_default(),
            "image[]" | "image" => {
                images += usize::from(!field.bytes().await.unwrap_or_default().is_empty())
            }
            "mask" => mask = !field.bytes().await.unwrap_or_default().is_empty(),
            _ => {}
        }
    }
    let summary = format!(
        "{prompt}|images={images}|mask={}|size={size}",
        if mask { "yes" } else { "no" }
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(summary);
    let data: Vec<Value> = (0..n).map(|_| json!({"b64_json": b64})).collect();
    Json(json!({"created": 1, "output_format": "png", "data": data}))
}

/// Manifest path of an Ollaya library model (`name[:tag]`).
fn manifest_path(s: &AppState, model: &str) -> Option<std::path::PathBuf> {
    let (name, tag) = model.split_once(':').unwrap_or((model, "latest"));
    Some(
        s.models_dir
            .as_ref()?
            .join("manifests/ollaya.dev/library")
            .join(name)
            .join(tag),
    )
}

/// Ollaya's `POST /api/pull`: NDJSON progress, then a manifest on disk.
/// Models whose name starts with `missing` are not in the registry.
async fn ollaya_pull(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> axum::response::Response {
    if !authorized(&s, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad key"}))).into_response();
    }
    let model = req.get("model").and_then(Value::as_str).unwrap_or("");
    if model.starts_with("missing") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("model \"{model}\" not found in registry ollaya.dev"),
                        "code": "MODEL_NOT_FOUND"})),
        )
            .into_response();
    }
    if let Some(delay) = env_ms("FAKE_PULL_DELAY_MS") {
        tokio::time::sleep(delay).await;
    }
    let Some(path) = manifest_path(&s, model) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "OLLAYA_MODELS not set").into_response();
    };
    let manifest = json!({"schemaVersion": 2, "config": {"digest": "sha256:00", "size": 10},
        "layers": [{"digest": "sha256:11", "size": 1000}, {"digest": "sha256:22", "size": 24}]});
    if std::fs::create_dir_all(path.parent().unwrap())
        .and_then(|_| std::fs::write(&path, manifest.to_string()))
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, "write failed").into_response();
    }
    let lines = [
        json!({"status": "pulling manifest"}),
        json!({"status": "pulling 111111111111", "digest": "sha256:11", "total": 1000, "completed": 250}),
        json!({"status": "pulling 111111111111", "digest": "sha256:11", "total": 1000, "completed": 1000}),
        json!({"status": "pulling 222222222222", "digest": "sha256:22", "total": 24, "completed": 24}),
        json!({"status": "verifying sha256 digest"}),
        json!({"status": "writing manifest"}),
        json!({"status": "success"}),
    ];
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    ([("content-type", "application/x-ndjson")], body).into_response()
}

/// Ollaya's `DELETE /api/delete`.
async fn ollaya_delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> axum::response::Response {
    if !authorized(&s, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad key"}))).into_response();
    }
    let model = req.get("model").and_then(Value::as_str).unwrap_or("");
    match manifest_path(&s, model) {
        Some(path) if path.is_file() => {
            let _ = std::fs::remove_file(path);
            StatusCode::OK.into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("model \"{model}\" not found"), "code": "MODEL_NOT_FOUND"})),
        )
            .into_response(),
    }
}

/// Ollaya's `POST /api/decide` without `state`: load (`keep_alive` < 0 or
/// absent) or unload (`keep_alive` 0).
async fn ollaya_decide(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> impl IntoResponse {
    let model = req.get("model").cloned().unwrap_or(Value::Null);
    let unload = req.get("keep_alive").and_then(Value::as_i64) == Some(0);
    guarded(
        &s,
        &headers,
        json!({"model": model, "answers": {}, "done_reason": if unload { "unload" } else { "load" }}),
    )
    .await
}

async fn models(State(s): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    guarded(&s, &headers, json!({"data": [{"id": "fake-model"}]})).await
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::var("FAKE_FAIL_START").as_deref() == Ok("1") {
        eprintln!("fake engine: failing on purpose");
        std::process::exit(2);
    }
    let args: Vec<String> = std::env::args().collect();
    let port = args
        .iter()
        .position(|a| a == "--port" || a == "--listen-port")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var(std::env::var("FAKE_PORT_VAR").ok()?).ok())
        .or_else(|| {
            let addr = std::env::var(std::env::var("FAKE_ADDR_VAR").ok()?).ok()?;
            Some(addr.rsplit_once(':')?.1.to_string())
        })
        .and_then(|p| p.parse::<u16>().ok())
        .expect("port");
    let key = std::env::var("FAKE_KEY_VAR")
        .ok()
        .and_then(|var| std::env::var(var).ok());

    if let Some(delay) = env_ms("FAKE_START_DELAY_MS") {
        tokio::time::sleep(delay).await;
    }
    if let Some(after) = env_ms("FAKE_EXIT_AFTER_MS") {
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            eprintln!("fake engine: crashing");
            std::process::exit(7);
        });
    }
    let state = AppState {
        ready_at: Instant::now() + env_ms("FAKE_LOADING_MS").unwrap_or_default(),
        key,
        models_dir: std::env::var_os("OLLAYA_MODELS").map(Into::into),
    };
    let app = Router::new()
        .route("/", get(|| async { "Ollaya is running" }))
        .route("/api/pull", post(ollaya_pull))
        .route("/api/delete", axum::routing::delete(ollaya_delete))
        .route("/api/decide", post(ollaya_decide))
        .route("/health", get(health))
        .route(
            "/openapi.json",
            get(|| async { Json(json!({"openapi": "3.1.0"})) }),
        )
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/systemone", post(systemone))
        .route("/v1/images/generations", post(images))
        .route("/v1/images/edits", post(image_edits))
        // The command line the engine was started with (tests check it).
        .route(
            "/fake/args",
            get(|| async { Json(json!(std::env::args().collect::<Vec<_>>())) }),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind");
    println!("fake engine listening on {port}");
    axum::serve(listener, app).await.expect("serve");
}
