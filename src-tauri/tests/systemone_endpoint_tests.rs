//! End-to-end tests for `POST /v1/systemone` (System One typed decisions).
//!
//! Covers the HTTP flow through the real server and router against wiremock
//! upstreams:
//! - native System One providers (Laya-style server): answers, headers,
//!   upstream error passthrough, prefix and unprefixed routes
//! - chat translation: letter mode (logprobs, via llama.cpp) and JSON mode
//!   (OpenAI-compatible without logprobs)
//! - validation and auth
//! - provider-level behaviour of each System One flavor (auth header,
//!   error mapping, request id, default model)

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::factory::{
    LlamaCppProviderFactory, OpenAICompatibleProviderFactory, SystemOneProviderFactory,
};
use localrouter::providers::registry::ProviderRegistry;
use localrouter::providers::systemone::{SystemOneFlavor, SystemOneProvider};
use localrouter::providers::{ModelProvider, SystemOneAnswer, SystemOneRequest};
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use wiremock::{
    matchers::{body_partial_json, header, header_exists, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

// ============================================================================
// Test infrastructure
// ============================================================================

fn create_test_client(id: &str, strategy_id: &str) -> Client {
    let mut client = Client::new_with_strategy("Test Client".to_string(), strategy_id.to_string());
    client.id = id.to_string();
    client
}

/// A provider instance to register: (instance name, provider type, config).
type Upstream = (&'static str, &'static str, Vec<(&'static str, String)>);

/// Start a server with the given provider instances. Returns the base URL
/// and the internal test secret (direct provider access, `provider/model`).
async fn start_server(upstreams: Vec<Upstream>) -> (String, String) {
    let test_client = create_test_client("test-api-key", "default");
    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![Strategy::new("Default".to_string())],
        ..Default::default()
    };
    let config_path =
        std::env::temp_dir().join(format!("test_systemone_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    for factory in SystemOneProviderFactory::all() {
        provider_registry.register_factory(Arc::new(factory));
    }
    provider_registry.register_factory(Arc::new(LlamaCppProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    for (name, provider_type, cfg) in upstreams {
        let cfg: HashMap<String, String> =
            cfg.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        provider_registry
            .create_provider(name.to_string(), provider_type.to_string(), cfg)
            .await
            .expect("create provider");
    }

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_systemone_{}.db", uuid::Uuid::new_v4()));
    let metrics_collector = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(metrics_db_path).unwrap(),
    )));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));

    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 43000 + (std::process::id() % 10000) as u16,
        enable_cors: true,
    };
    let (state, _handle, actual_port, _shutdown) = server::start_server(
        server_config,
        router,
        Arc::new(McpServerManager::new()),
        rate_limiter,
        provider_registry,
        config_manager,
        Arc::new(ClientManager::new(vec![test_client])),
        Arc::new(TokenStore::new()),
        metrics_collector,
        None,
    )
    .await
    .expect("Failed to start test server");
    sleep(Duration::from_millis(200)).await;
    (
        format!("http://127.0.0.1:{}", actual_port),
        state.get_internal_test_secret(),
    )
}

fn decision_body(model: &str) -> Value {
    json!({
        "model": model,
        "state": {"subject": "Refund", "body": "I was billed twice for October."},
        "questions": {
            "dept": {"type": "choice", "instructions": "Which team?",
                "criteria": {"billing": "refunds", "tech": "bugs"}},
            "urgency": {"type": "score", "instructions": "How urgent?",
                "criteria": ["low", "medium", "high"]},
            "human": {"type": "noul", "instructions": "Needs a human?"}
        }
    })
}

fn native_answer() -> Value {
    json!({
        "model": "english",
        "answers": {
            "dept": {"type": "choice", "choice": "billing", "confidence": 0.9,
                "probabilities": {"billing": 0.95, "tech": 0.05}},
            "urgency": {"type": "score", "score": 1.2, "confidence": 0.3,
                "legend": {"0": "low", "1": "medium", "2": "high"},
                "probabilities": {"0": 0.2, "1": 0.4, "2": 0.4}},
            "human": {"type": "noul", "noul": 0.2}
        },
        "usage": {"input_tokens": 120, "output_tokens": 0}
    })
}

/// A wiremock server that behaves like laya-serve.
async fn laya_mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [{"id": "english"}, {"id": "multilingual"}]})),
        )
        .mount(&mock)
        .await;
    mock
}

async fn post(base_url: &str, route: &str, secret: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}{}", base_url, route))
        .bearer_auth(secret)
        .json(body)
        .send()
        .await
        .unwrap()
}

// ============================================================================
// Gateway: native providers
// ============================================================================

#[tokio::test]
async fn native_decision_end_to_end() {
    let mock = laya_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        // Provider prefix is stripped before the upstream sees the model.
        .and(body_partial_json(json!({"model": "english"})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req-abc")
                .set_body_json(native_answer()),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let (base_url, secret) = start_server(vec![(
        "laya",
        "systemone_compatible",
        vec![("base_url", mock.uri())],
    )])
    .await;
    let resp = post(
        &base_url,
        "/v1/systemone",
        &secret,
        &decision_body("laya/english"),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-localrouter-systemone-backend"], "native");
    assert_eq!(resp.headers()["x-typesafe-request-id"], "req-abc");
    assert!(resp.headers().contains_key("x-localrouter-generation-id"));
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["answers"]["dept"]["choice"], "billing");
    assert_eq!(body["usage"]["input_tokens"], 120);
    // Answer order is the request's question order.
    let ids: Vec<&String> = body["answers"].as_object().unwrap().keys().collect();
    assert_eq!(ids, vec!["dept", "urgency", "human"]);
}

#[tokio::test]
async fn unprefixed_route_works() {
    let mock = laya_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(native_answer()))
        .mount(&mock)
        .await;
    let (base_url, secret) = start_server(vec![(
        "laya",
        "systemone_compatible",
        vec![("base_url", mock.uri())],
    )])
    .await;
    let resp = post(
        &base_url,
        "/systemone",
        &secret,
        &decision_body("laya/english"),
    )
    .await;
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn upstream_422_is_passed_through_verbatim() {
    let mock = laya_mock().await;
    let detail = json!({"detail": [{"loc": ["body", "state"], "msg": "state exceeds 512 tokens"}]});
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(422).set_body_json(detail.clone()))
        .mount(&mock)
        .await;
    let (base_url, secret) = start_server(vec![(
        "laya",
        "systemone_compatible",
        vec![("base_url", mock.uri())],
    )])
    .await;
    let resp = post(
        &base_url,
        "/v1/systemone",
        &secret,
        &decision_body("laya/english"),
    )
    .await;
    assert_eq!(resp.status(), 422);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, detail);
}

#[tokio::test]
async fn upstream_server_error_is_a_502_envelope() {
    let mock = laya_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(529).set_body_string("overloaded"))
        .mount(&mock)
        .await;
    let (base_url, secret) = start_server(vec![(
        "laya",
        "systemone_compatible",
        vec![("base_url", mock.uri())],
    )])
    .await;
    let resp = post(
        &base_url,
        "/v1/systemone",
        &secret,
        &decision_body("laya/english"),
    )
    .await;
    assert_eq!(resp.status(), 502);
    let body: Value = resp.json().await.unwrap();
    assert!(body["error"]["message"].as_str().is_some());
}

// ============================================================================
// Gateway: validation and auth
// ============================================================================

#[tokio::test]
async fn validation_errors_are_400() {
    let (base_url, secret) = start_server(vec![]).await;
    let cases = vec![
        json!({"model": "laya/english", "state": null,
            "questions": {"q": {"type": "noul", "instructions": "?"}}}),
        json!({"model": "laya/english", "state": "s", "questions": {}}),
        json!({"model": "laya/english", "state": "s",
            "questions": {"s": {"type": "score", "instructions": "?", "criteria": ["only one"]}}}),
        json!({"model": "laya/english", "state": "s", "stream": true,
            "questions": {"q": {"type": "noul", "instructions": "?"}}}),
    ];
    for body in cases {
        let resp = post(&base_url, "/v1/systemone", &secret, &body).await;
        assert_eq!(resp.status(), 400, "body: {body}");
        let err: Value = resp.json().await.unwrap();
        assert!(err["error"]["message"].as_str().is_some());
    }

    let mut many = serde_json::Map::new();
    for i in 0..256 {
        many.insert(format!("o{i}"), Value::Null);
    }
    let too_many = json!({"model": "laya/english", "state": "s",
        "questions": {"c": {"type": "choice", "instructions": "?", "criteria": many}}});
    let resp = post(&base_url, "/v1/systemone", &secret, &too_many).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn malformed_json_is_rejected() {
    let (base_url, secret) = start_server(vec![]).await;
    // Unknown question type does not deserialize.
    let body = json!({"state": "s", "questions": {"q": {"type": "rank", "instructions": "?"}}});
    let resp = post(&base_url, "/v1/systemone", &secret, &body).await;
    assert!(resp.status().is_client_error(), "got {}", resp.status());
}

#[tokio::test]
async fn requires_auth() {
    let (base_url, _secret) = start_server(vec![]).await;
    for route in ["/v1/systemone", "/systemone"] {
        let resp = reqwest::Client::new()
            .post(format!("{}{}", base_url, route))
            .json(&decision_body("laya/english"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "{route}");
    }
}

#[tokio::test]
async fn openapi_documents_systemone() {
    let (base_url, _secret) = start_server(vec![]).await;
    let spec: Value = reqwest::get(format!("{}/openapi.json", base_url))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/systemone"]["post"].is_object());
    assert!(spec["components"]["schemas"]["SystemOneRequest"].is_object());
}

// ============================================================================
// Gateway: translation onto chat models
// ============================================================================

/// llama-server style chat response answering with a letter plus logprobs.
fn letter_response(letter: &str, p: f64) -> Value {
    let other = if letter == "A" { "B" } else { "A" };
    json!({
        "id": "c1", "object": "chat.completion", "created": 0, "model": "tev1",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": letter},
            "finish_reason": "stop",
            "logprobs": {"content": [{
                "token": letter, "logprob": p.ln(),
                "top_logprobs": [
                    {"token": letter, "logprob": p.ln()},
                    {"token": other, "logprob": (1.0 - p).ln()}
                ]
            }]}
        }],
        "usage": {"prompt_tokens": 50, "completion_tokens": 1, "total_tokens": 51}
    })
}

#[tokio::test]
async fn chat_model_letter_mode_via_logprobs() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "tev1"}]})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(
            json!({"logprobs": true, "temperature": 0.0}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(letter_response("A", 0.8)))
        .expect(2)
        .mount(&mock)
        .await;

    let (base_url, secret) = start_server(vec![(
        "llama",
        "llamacpp",
        vec![("base_url", format!("{}/v1", mock.uri()))],
    )])
    .await;
    let body = json!({
        "model": "llama/tev1",
        "state": "Customer was charged twice.",
        "questions": {
            "dept": {"type": "choice", "instructions": "Which team?",
                "criteria": {"billing": "refunds", "tech": "bugs"}},
            "human": {"type": "noul", "instructions": "Needs a human?"}
        }
    });
    let resp = post(&base_url, "/v1/systemone", &secret, &body).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["x-localrouter-systemone-backend"],
        "letter_logprobs"
    );
    let out: Value = resp.json().await.unwrap();
    assert_eq!(out["answers"]["dept"]["choice"], "billing");
    let p = out["answers"]["dept"]["probabilities"]["billing"]
        .as_f64()
        .unwrap();
    assert!((p - 0.8).abs() < 1e-6, "p = {p}");
    let yes = out["answers"]["human"]["noul"].as_f64().unwrap();
    assert!((yes - 0.8).abs() < 1e-6);
    assert_eq!(out["usage"]["input_tokens"], 100);
}

#[tokio::test]
async fn chat_model_json_mode_without_logprobs() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "gpt"}]})))
        .mount(&mock)
        .await;
    let answer = json!({"answers": {
        "dept": {"probabilities": {"billing": 0.3, "tech": 0.7}},
        "human": {"noul": 0.6}
    }});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(
            json!({"response_format": {"type": "json_object"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c1", "object": "chat.completion", "created": 0, "model": "gpt",
            "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": answer.to_string()}}],
            "usage": {"prompt_tokens": 80, "completion_tokens": 20, "total_tokens": 100}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let (base_url, secret) = start_server(vec![(
        "compat",
        "openai_compatible",
        vec![("base_url", mock.uri()), ("api_key", "k".to_string())],
    )])
    .await;
    let body = json!({
        "model": "compat/gpt",
        "state": "Customer was charged twice.",
        "questions": {
            "dept": {"type": "choice", "instructions": "Which team?",
                "criteria": {"billing": "refunds", "tech": "bugs"}},
            "human": {"type": "noul", "instructions": "Needs a human?"}
        }
    });
    let resp = post(&base_url, "/v1/systemone", &secret, &body).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-localrouter-systemone-backend"], "json");
    let out: Value = resp.json().await.unwrap();
    assert_eq!(out["answers"]["dept"]["choice"], "tech");
    assert_eq!(out["model"], "compat/gpt");
}

// ============================================================================
// Provider level: each System One flavor against a mock upstream
// ============================================================================

fn provider_request(model: Option<&str>) -> SystemOneRequest {
    let mut body = decision_body("unused");
    match model {
        Some(m) => body["model"] = json!(m),
        None => {
            body.as_object_mut().unwrap().remove("model");
        }
    }
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn typesafe_sends_bearer_and_default_model() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer ts-key"))
        .and(body_partial_json(json!({"model": "jev-latest"})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "r-1")
                .set_body_json(native_answer()),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let provider = SystemOneProvider::new(
        SystemOneFlavor::TypeSafe,
        Some(mock.uri()),
        Some("ts-key".into()),
    )
    .unwrap();
    let resp = provider.systemone(provider_request(None)).await.unwrap();
    assert_eq!(resp.request_id.as_deref(), Some("r-1"));
    assert!(matches!(
        resp.answers["dept"],
        SystemOneAnswer::Choice { .. }
    ));
}

#[tokio::test]
async fn laya_without_key_sends_no_auth_and_omits_model() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(move |req: &Request| {
            assert!(!req.headers.contains_key("authorization"));
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert!(
                body.get("model").is_none(),
                "laya-serve routes by language itself"
            );
            ResponseTemplate::new(200).set_body_json(native_answer())
        })
        .expect(1)
        .mount(&mock)
        .await;
    let provider = SystemOneProvider::new(SystemOneFlavor::Laya, Some(mock.uri()), None).unwrap();
    provider.systemone(provider_request(None)).await.unwrap();
}

#[tokio::test]
async fn provider_error_mapping() {
    use lr_types::AppError;
    for (status, check) in [
        (401u16, "unauthorized"),
        (429, "rate"),
        (422, "status"),
        (529, "provider"),
    ] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(ResponseTemplate::new(status).set_body_string(r#"{"detail":"x"}"#))
            .mount(&mock)
            .await;
        let provider =
            SystemOneProvider::new(SystemOneFlavor::Kev, Some(mock.uri()), Some("k".into()))
                .unwrap();
        let err = provider
            .systemone(provider_request(Some("kev-latest")))
            .await
            .unwrap_err();
        let ok = match (check, &err) {
            ("unauthorized", AppError::Unauthorized) => true,
            ("rate", AppError::RateLimitExceeded) => true,
            (
                "status",
                AppError::ProviderStatus {
                    status: 422,
                    message,
                },
            ) => message == r#"{"detail":"x"}"#,
            ("provider", AppError::Provider(_)) => true,
            _ => false,
        };
        assert!(ok, "status {status} mapped to {err:?}");
    }
}

#[tokio::test]
async fn kev_lists_models_and_health() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header_exists("authorization"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models": [{"name": "jaredpalmer/kev-4b"}]})),
        )
        .mount(&mock)
        .await;
    let provider =
        SystemOneProvider::new(SystemOneFlavor::Kev, Some(mock.uri()), Some("k".into())).unwrap();
    let models = provider.list_models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["kev-latest", "jaredpalmer/kev-4b"]);
    let health = provider.health_check().await;
    assert_eq!(health.status, localrouter::providers::HealthStatus::Healthy);
}

#[tokio::test]
async fn typesafe_health_probe_treats_validation_error_as_healthy() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"detail": []})))
        .mount(&mock)
        .await;
    let provider = SystemOneProvider::new(
        SystemOneFlavor::TypeSafe,
        Some(mock.uri()),
        Some("k".into()),
    )
    .unwrap();
    assert_eq!(
        provider.health_check().await.status,
        localrouter::providers::HealthStatus::Healthy
    );
}
