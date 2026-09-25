//! A stand-in engine for tests. Behaviour is controlled by environment
//! variables so supervisor tests can simulate slow starts, loading, crashes
//! and auth without real models.
//!
//! - `--port N` or the env var named by `FAKE_PORT_VAR` sets the port
//! - `FAKE_KEY_VAR`: name of the env var holding the required API key
//! - `FAKE_START_DELAY_MS`: wait before binding (port closed meanwhile)
//! - `FAKE_LOADING_MS`: `/health` answers 503 for this long after binding
//! - `FAKE_EXIT_AFTER_MS`: exit with code 7 after this long
//! - `FAKE_FAIL_START=1`: print an error and exit 2 immediately

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
    };
    let app = Router::new()
        .route("/health", get(health))
        .route(
            "/openapi.json",
            get(|| async { Json(json!({"openapi": "3.1.0"})) }),
        )
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/systemone", post(systemone))
        .route("/v1/images/generations", post(images))
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
