//! End-to-end tests for `POST /v1/systemone` (System One typed decisions).
//!
//! Covers the HTTP flow through the real server and router against wiremock
//! upstreams:
//! - native System One providers (Laya-style server): answers, headers,
//!   upstream error passthrough, prefix and unprefixed routes
//! - chat translation: letter mode (logprobs, via llama.cpp) and JSON mode
//!   (OpenAI-compatible without logprobs)
//! - Ollama: decision models (`decision` capability) answered natively, chat
//!   models by letter mode on `/api/chat` logprobs
//! - validation and auth
//! - provider-level behaviour of each System One flavor (auth header,
//!   error mapping, request id, default model)

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::factory::{
    LlamaCppProviderFactory, OllamaProviderFactory, OpenAICompatibleProviderFactory,
    SystemOneProviderFactory,
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

struct TestGatewayFactory;
impl localrouter::providers::factory::ProviderFactory for TestGatewayFactory {
    fn provider_type(&self) -> &str {
        "llmgateway"
    }
    fn display_name(&self) -> &str {
        "Test Gateway"
    }
    fn category(&self) -> localrouter::providers::factory::ProviderCategory {
        localrouter::providers::factory::ProviderCategory::ThirdParty
    }
    fn description(&self) -> &str {
        "Loopback fixture"
    }
    fn setup_parameters(&self) -> Vec<localrouter::providers::factory::SetupParameter> {
        vec![]
    }
    fn validate_config(&self, _: &HashMap<String, String>) -> lr_types::AppResult<()> {
        Ok(())
    }
    fn create(
        &self,
        _: String,
        config: HashMap<String, String>,
    ) -> lr_types::AppResult<Arc<dyn ModelProvider>> {
        let base = config.get("base_url").unwrap();
        let key = config.get("api_key").unwrap();
        let gateway = localrouter::providers::systemone::SystemOneGateway::llmgateway(base, key)?;
        Ok(Arc::new(
            localrouter::providers::openai_compatible::OpenAICompatibleProvider::new(
                "llmgateway".into(),
                base.clone(),
                Some(key.clone()),
            )
            .with_systemone_gateway(gateway),
        ))
    }
}

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
    start_configured_server(upstreams, |_| {}, false).await
}

async fn start_configured_server(
    upstreams: Vec<Upstream>,
    configure: impl FnOnce(&mut AppConfig),
    normal_token: bool,
) -> (String, String) {
    let (base, secret, _) = start_replay_test_server(upstreams, configure, normal_token).await;
    (base, secret)
}
async fn start_replay_test_server(
    upstreams: Vec<Upstream>,
    configure: impl FnOnce(&mut AppConfig),
    normal_token: bool,
) -> (String, String, server::state::AppState) {
    let mut test_client = create_test_client("test-api-key", "default");
    let mut config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![Strategy::new("Default".to_string())],
        ..Default::default()
    };
    configure(&mut config);
    test_client = config.clients[0].clone();
    test_client.strategy_id = config.strategies[0].id.clone();
    config.clients[0] = test_client.clone();
    if normal_token {
        config.strategies[0].parent = Some(test_client.id.clone());
    }
    let tokens = Arc::new(TokenStore::new());
    let clients = Arc::new(ClientManager::with_keychain(
        Vec::new(),
        Arc::new(localrouter::api_keys::MockKeychain::new()),
    ));
    let normal_secret = clients.add_client_with_secret(test_client.clone()).unwrap();
    let config_path =
        std::env::temp_dir().join(format!("test_systemone_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    for factory in SystemOneProviderFactory::all() {
        provider_registry.register_factory(Arc::new(factory));
    }
    provider_registry.register_factory(Arc::new(LlamaCppProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    provider_registry.register_factory(Arc::new(OllamaProviderFactory));
    provider_registry.register_factory(Arc::new(TestGatewayFactory));
    provider_registry.register_factory(Arc::new(
        documents_endpoint_cases::TestDocumentGatewayFactory,
    ));
    provider_registry
        .register_factory(Arc::new(messages_endpoint_cases::TestNativeMessagesFactory));
    provider_registry.register_factory(Arc::new(batches_endpoint_cases::TestNativeBatchFactory(
        false,
    )));
    provider_registry.register_factory(Arc::new(batches_endpoint_cases::TestNativeBatchFactory(
        true,
    )));
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
        clients,
        tokens,
        metrics_collector,
        None,
    )
    .await
    .expect("Failed to start test server");
    sleep(Duration::from_millis(200)).await;
    (
        format!("http://127.0.0.1:{}", actual_port),
        if normal_token {
            normal_secret
        } else {
            state.get_internal_test_secret()
        },
        state,
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

/// An Ollama 0.35 server with a decision model (tev1) and a chat model.
async fn ollama_mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "tev1:0.8b", "modified_at": "", "size": 1, "digest": "a"},
            {"name": "llama3.2:latest", "modified_at": "", "size": 1, "digest": "b"}
        ]})))
        .mount(&mock)
        .await;
    for (name, caps) in [
        (
            "tev1:0.8b",
            json!(["decision", "tools", "thinking", "completion"]),
        ),
        ("llama3.2:latest", json!(["completion", "tools"])),
    ] {
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_partial_json(json!({"name": name})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities": caps})))
            .mount(&mock)
            .await;
    }
    // Native System One, only for the decision model.
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({"model": "tev1:0.8b"})))
        .respond_with(ResponseTemplate::new(200).set_body_json({
            let mut a = native_answer();
            a["model"] = json!("tev1:0.8b");
            a
        }))
        .expect(1)
        .mount(&mock)
        .await;
    // Letter mode for the chat model: Ollama's native /api/chat logprobs.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(
            json!({"model": "llama3.2:latest", "logprobs": true}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "llama3.2:latest",
            "message": {"role": "assistant", "content": "A"},
            "done": true,
            "logprobs": [{"token": "A", "logprob": 0.8f64.ln(), "top_logprobs": [
                {"token": "A", "logprob": 0.8f64.ln()},
                {"token": "B", "logprob": 0.2f64.ln()}
            ]}],
            "prompt_eval_count": 50,
            "eval_count": 1
        })))
        .expect(1)
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn ollama_decision_models_are_native_and_chat_models_use_letters() {
    let mock = ollama_mock().await;
    let (base_url, secret) =
        start_server(vec![("ollama", "ollama", vec![("base_url", mock.uri())])]).await;

    let resp = post(
        &base_url,
        "/v1/systemone",
        &secret,
        &decision_body("ollama/tev1:0.8b"),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-localrouter-systemone-backend"], "native");
    let out: Value = resp.json().await.unwrap();
    assert_eq!(out["answers"]["dept"]["choice"], "billing");
    assert_eq!(out["model"], "tev1:0.8b");

    let body = json!({
        "model": "ollama/llama3.2:latest",
        "state": "Customer was charged twice.",
        "questions": {"dept": {"type": "choice", "instructions": "Which team?",
            "criteria": {"billing": "refunds", "tech": "bugs"}}}
    });
    let resp = post(&base_url, "/v1/systemone", &secret, &body).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["x-localrouter-systemone-backend"],
        "letter_logprobs"
    );
    let out: Value = resp.json().await.unwrap();
    let p = out["answers"]["dept"]["probabilities"]["billing"]
        .as_f64()
        .unwrap();
    assert!((p - 0.8).abs() < 1e-6, "p = {p}");
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
        (429, "retry"),
        (503, "retry_unavailable"),
    ] {
        let mock = MockServer::start().await;
        let mut reply = ResponseTemplate::new(status).set_body_string(r#"{"detail":"x"}"#);
        if check.starts_with("retry") {
            reply = reply.insert_header("Retry-After", "9");
        }
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(reply)
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
            ("provider", AppError::ProviderUnavailable { status: 529, .. }) => true,
            (
                "retry",
                AppError::RateLimitWithRetryAfter {
                    retry_after_secs: 9,
                },
            ) => true,
            (
                "retry_unavailable",
                AppError::ProviderUnavailable {
                    status: 503,
                    retry_after_secs: Some(9),
                    ..
                },
            ) => true,
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

// OpenAI Decisions ingress uses the same policies while preserving its wire shape.
#[tokio::test]
async fn decisions_translates_text_to_legacy_and_rejects_images() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({"state":"test evidence"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model":"local","answers":{"decision_0":{"type":"noul","noul":0.8}},
            "usage":{"input_tokens":10,"output_tokens":0}
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let (base, secret) = start_server(vec![(
        "legacy",
        "systemone_compatible",
        vec![("base_url", format!("{}/v1", mock.uri()))],
    )])
    .await;
    let body = json!({"model":"legacy/local","input":[{"role":"user","content":[{"type":"input_text","text":"test evidence"}]}],"questions":[{"type":"predicate","instructions":"yes?"}]});
    let response = post(&base, "/v1/decisions", &secret, &body).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["x-localrouter-decisions-backend"],
        "native"
    );
    let result: Value = response.json().await.unwrap();
    assert_eq!(
        result["answers"][0],
        json!({"type":"predicate","name":null,"probability":0.8})
    );
    let mut image = body;
    image["input"] = json!([{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}]);
    let image_response = post(&base, "/decisions", &secret, &image).await;
    let image_status = image_response.status();
    let image_error = image_response.text().await.unwrap();
    assert_eq!(image_status, 400, "{image_error}");
    let mut invalid = image;
    invalid["questions"] = json!([]);
    assert_eq!(
        post(&base, "/v1/decisions", &secret, &invalid)
            .await
            .status(),
        400
    );
    let spec: Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(spec["paths"]["/v1/decisions"]["post"].is_object());
    assert!(spec["components"]["schemas"]["DecisionAnswer"].is_object());
    for schema in [
        "GatewayPreferences",
        "GatewayAttribution",
        "GatewayRoutingAttempt",
        "RoutingExplanation",
        "CandidateExplanation",
        "SessionOptions",
        "PromptCacheAccounting",
    ] {
        assert!(spec["components"]["schemas"][schema].is_object());
    }
    for schema in [
        "ChatCompletionRequest",
        "CompletionRequest",
        "CreateResponseRequest",
    ] {
        assert!(spec["components"]["schemas"][schema]["properties"]["extensions"].is_object());
    }
}

#[tokio::test]
async fn decisions_gateway_preserves_images_booleans_and_refusals() {
    let mock = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"image-decider","architecture":{"output_modalities":["decision"],"input_modalities":["text","image"]}}]})))
        .mount(&mock).await;
    Mock::given(method("POST")).and(path("/v1/decisions"))
        .and(header("authorization","Bearer fake-key"))
        .and(body_partial_json(json!({"model":"image-decider","input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model":"image-decider","answers":[{"type":"choice","name":"damage","choice":true,"confidence":0.8,"probabilities":[{"value":true,"probability":0.9},{"value":"true","probability":0.1}]},{"type":"refusal","name":null}],
            "usage":{"input_tokens":10,"output_tokens":0,"total_tokens":10},
            "metadata":{"routing":[{"provider":"typesafe","succeeded":true,"apiKeyHash":"private-key-fingerprint","providerKeyLabel":"private-key-label","error":"private-error"}]}
        }))).expect(1).mount(&mock).await;
    let (base, secret) = start_server(vec![(
        "gateway",
        "llmgateway",
        vec![
            ("base_url", format!("{}/v1", mock.uri())),
            ("api_key", "fake-key".into()),
        ],
    )])
    .await;
    let request = json!({"model":"gateway/image-decider","input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}],"questions":[{"type":"choice","name":"damage","instructions":"choose","choices":[{"value":true},{"value":"true"}]},{"type":"predicate","instructions":"private?"}]});
    let response = post(&base, "/v1/decisions", &secret, &request).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["answers"][0]["choice"], true);
    assert_eq!(body["answers"][0]["probabilities"][1]["value"], "true");
    assert_eq!(body["answers"][1], json!({"type":"refusal","name":null}));
    assert!(!body.to_string().contains("private-key"));
    assert!(!body.to_string().contains("private-error"));
    let mut unsupported = request.clone();
    unsupported["service_tier"] = json!("priority");
    assert_eq!(
        post(&base, "/v1/decisions", &secret, &unsupported)
            .await
            .status(),
        400
    );
    let mut raw_controls = request.clone();
    raw_controls["provider"] = json!({"zdr":true});
    assert_eq!(
        post(&base, "/v1/decisions", &secret, &raw_controls)
            .await
            .status(),
        400
    );
    mock.verify().await;
}

#[tokio::test]
async fn gateway_preferences_and_attribution_survive_every_nonstreaming_llm_surface() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
        .mount(&mock)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .and(body_partial_json(json!({"routing":"price","stream":false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"gateway-call", "object":"chat.completion", "created":1, "model":"test",
            "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02},
            "metadata":{"routing":[{"provider":"openai","model":"test","succeeded":true,"apiKeyHash":"private"}]}
        }))).expect(3).mount(&mock).await;
    let (base, secret) = start_server(vec![(
        "gw",
        "llmgateway",
        vec![("base_url", mock.uri()), ("api_key", "k".into())],
    )])
    .await;
    for (route, input) in [
        (
            "/v1/chat/completions",
            json!({"messages":[{"role":"user","content":"hi"}]}),
        ),
        ("/v1/completions", json!({"prompt":"hi"})),
        ("/v1/responses", json!({"input":"hi","store":false})),
    ] {
        let mut body = input;
        body["model"] = json!("gw/test");
        body["extensions"] = json!({"localrouter.gateway":{"sort":"price"}});
        let response = post(&base, route, &secret, &body).await;
        assert_eq!(response.status(), 200, "{route}");
        let value: Value = response.json().await.unwrap();
        let attribution = &value["extensions"]["localrouter.gateway"];
        assert_eq!(
            attribution["inference_provider"], "openai",
            "{route}: {value}"
        );
        assert_eq!(attribution["cost_usd"], 0.02, "{route}: {value}");
        assert!(!attribution.to_string().contains("private"));
        for raw_key in ["provider", "providerOptions", "routing", "models", "zdr"] {
            let mut raw_controls = body.clone();
            raw_controls[raw_key] = json!({"zdr":true});
            let response = post(&base, route, &secret, &raw_controls).await;
            assert_eq!(response.status(), 400, "{route}/{raw_key}");
            let error: Value = response.json().await.unwrap();
            assert!(error.to_string().contains("extensions.localrouter.gateway"));
        }
    }
    for extra in [
        json!({"service_tier":"priority"}),
        json!({"extensions":{"localrouter.gateway":{"sort":"price","zdr":true}}}),
    ] {
        let mut body = json!({"model":"gw/test","messages":[{"role":"user","content":"must not be sent"}],"extensions":{"localrouter.gateway":{"sort":"price"}}});
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        assert_eq!(
            post(&base, "/v1/chat/completions", &secret, &body)
                .await
                .status(),
            400
        );
    }
    mock.verify().await;
}

#[tokio::test]
async fn final_gateway_metadata_is_delivered_with_or_without_requested_stream_usage() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
        .mount(&mock)
        .await;
    let content = json!({"id":"gateway-stream","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]});
    let usage = json!({"id":"gateway-stream","object":"chat.completion.chunk","created":1,"model":"test","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02},"metadata":{"routing":[{"provider":"openai","model":"test","succeeded":true}]}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"routing":"price","stream":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(
                    "data: {content}\n\ndata: {usage}\n\ndata: [DONE]\n\n"
                ))
                .insert_header("Content-Type", "text/event-stream"),
        )
        .expect(6)
        .mount(&mock)
        .await;
    let (base, secret) = start_server(vec![(
        "gw",
        "llmgateway",
        vec![("base_url", mock.uri()), ("api_key", "k".into())],
    )])
    .await;
    for include_usage in [false, true] {
        for (route, input) in [
            (
                "/v1/chat/completions",
                json!({"messages":[{"role":"user","content":"hi"}]}),
            ),
            ("/v1/completions", json!({"prompt":"hi"})),
            ("/v1/responses", json!({"input":"hi","store":false})),
        ] {
            let mut body = input;
            body["model"] = json!("gw/test");
            body["stream"] = json!(true);
            body["extensions"] = json!({"localrouter.gateway":{"sort":"price"}});
            if route != "/v1/responses" {
                body["stream_options"] = json!({"include_usage":include_usage});
            }
            let response = post(&base, route, &secret, &body).await;
            assert_eq!(response.status(), 200, "{route}");
            let raw = response.text().await.unwrap();
            let frames: Vec<Value> = raw
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|data| serde_json::from_str(data).ok())
                .collect();
            let last_attribution = frames
                .iter()
                .filter_map(|frame| frame.get("response").unwrap_or(frame).get("extensions"))
                .filter_map(|extensions| extensions.get("localrouter.gateway"))
                .next_back()
                .unwrap_or_else(|| panic!("{route}: {raw}"));
            assert_eq!(
                last_attribution["inference_provider"], "openai",
                "{route}: {raw}"
            );
            assert_eq!(last_attribution["cost_usd"], 0.02, "{route}: {raw}");
            assert_eq!(
                last_attribution["cost_scope"], "request_total",
                "{route}: {raw}"
            );
            if route != "/v1/responses" && !include_usage {
                assert!(
                    frames.iter().all(|frame| frame.get("usage").is_none()),
                    "{route}: {raw}"
                );
            }
        }
    }
    mock.verify().await;
}

#[tokio::test]
async fn no_route_http_explanation_survives_chat_completions_and_responses() {
    let mock = MockServer::start().await;
    let (base, token) = start_configured_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "k".into())],
        )],
        |config| {
            let strategy = &mut config.strategies[0];
            strategy.auto_config.as_mut().unwrap().prioritized_models =
                vec![("gw".into(), "test".into())];
            strategy.model_permissions.global = localrouter::config::PermissionState::Off;
        },
        true,
    )
    .await;
    for (path, input) in [
        (
            "/v1/chat/completions",
            json!({"messages":[{"role":"user","content":"private prompt"}]}),
        ),
        ("/v1/completions", json!({"prompt":"private prompt"})),
        (
            "/v1/responses",
            json!({"input":"private prompt","store":false}),
        ),
    ] {
        let mut body = input;
        body["model"] = json!("localrouter/auto");
        let response = post(&base, path, &token, &body).await;
        assert_eq!(response.status(), 502, "{path}");
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["error"]["code"], "no_route", "{path}: {value}");
        let facts = &value["extensions"]["localrouter.routing"];
        assert_eq!(facts["explanation"]["no_route"], true, "{path}: {value}");
        assert_eq!(facts["attempts"][0]["outcome"], "model_not_allowed");
        assert!(!facts.to_string().contains("private prompt"));
    }
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn successful_auto_route_explanations_reach_normal_and_streaming_clients() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
        .mount(&mock)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .and(body_partial_json(json!({"stream":false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"route-normal","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02}}))).expect(3).mount(&mock).await;
    let content = json!({"id":"route-stream","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"stream":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("data: {content}\n\ndata: [DONE]\n\n"))
                .insert_header("Content-Type", "text/event-stream"),
        )
        .expect(3)
        .mount(&mock)
        .await;
    let (base, token) = start_configured_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "k".into())],
        )],
        |config| {
            config.strategies[0]
                .auto_config
                .as_mut()
                .unwrap()
                .prioritized_models = vec![("gw".into(), "test".into())];
        },
        true,
    )
    .await;
    for streaming in [false, true] {
        for (path, input) in [
            (
                "/v1/chat/completions",
                json!({"messages":[{"role":"user","content":"hi"}]}),
            ),
            ("/v1/completions", json!({"prompt":"hi"})),
            ("/v1/responses", json!({"input":"hi","store":false})),
        ] {
            let mut body = input;
            body["model"] = json!("localrouter/auto");
            body["stream"] = json!(streaming);
            let response = post(&base, path, &token, &body).await;
            assert_eq!(response.status(), 200, "{path}");
            let raw = response.text().await.unwrap();
            if streaming {
                // Text APIs send a routing namespace on their initial content chunk;
                // Responses preserves it in response.completed.
                assert!(raw.contains("localrouter.routing"), "{path}: {raw}");
                assert!(raw.contains("config_fingerprint"), "{path}: {raw}");
                let final_trace = raw
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|data| serde_json::from_str::<Value>(data).ok())
                    .filter_map(|frame| {
                        frame
                            .get("response")
                            .unwrap_or(&frame)
                            .get("extensions")
                            .and_then(|extensions| extensions.get("localrouter.routing"))
                            .cloned()
                    })
                    .next_back()
                    .unwrap();
                assert_eq!(
                    final_trace["attempt_costs"]["reported_usd"], 0.02,
                    "{path}: {raw}"
                );
                assert_eq!(
                    final_trace["attempt_costs"]["unknown_calls"], 0,
                    "{path}: {raw}"
                );
            } else {
                let value: Value = serde_json::from_str(&raw).unwrap();
                let facts = &value["extensions"]["localrouter.routing"];
                assert_eq!(facts["attempts"][0]["outcome"], "success", "{path}: {raw}");
                assert_eq!(facts["explanation"]["no_route"], false);
                assert_eq!(
                    facts["explanation"]["config_fingerprint"]
                        .as_str()
                        .unwrap()
                        .len(),
                    64
                );
            }
        }
    }
    mock.verify().await;
}

#[tokio::test]
async fn permanent_auto_route_failures_retain_status_and_trace_without_fallback() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"id":"test"},{"id":"fallback"}]})),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"model":"test"})))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error":{"message":"forbidden","type":"permission_error"}})),
        )
        .expect(6)
        .mount(&mock)
        .await;
    let (base, token) = start_configured_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", mock.uri()), ("api_key", "k".into())],
        )],
        |config| {
            config.strategies[0]
                .auto_config
                .as_mut()
                .unwrap()
                .prioritized_models = vec![
                ("gw".into(), "test".into()),
                ("gw".into(), "fallback".into()),
            ];
        },
        true,
    )
    .await;
    for stream in [false, true] {
        for (route, input) in [
            (
                "/v1/chat/completions",
                json!({"messages":[{"role":"user","content":"private input"}]}),
            ),
            ("/v1/completions", json!({"prompt":"private input"})),
            (
                "/v1/responses",
                json!({"input":"private input","store":false}),
            ),
        ] {
            let mut body = input;
            body["model"] = json!("localrouter/auto");
            body["stream"] = json!(stream);
            let response = post(&base, route, &token, &body).await;
            assert_eq!(response.status(), 403, "{route}/{stream}");
            let error: Value = response.json().await.unwrap();
            let trace = &error["extensions"]["localrouter.routing"];
            assert_eq!(trace["attempts"].as_array().unwrap().len(), 1, "{error}");
            assert_eq!(trace["attempts"][0]["outcome"], "error");
            assert!(!trace.to_string().contains("private input"));
            assert!(trace["explanation"]["config_fingerprint"].is_string());
        }
    }
    assert!(mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
        .all(|request| request.body_json::<Value>().unwrap()["model"] == "test"));
    mock.verify().await;
}

#[tokio::test]
async fn bounded_http_attempts_keep_server_retry_after_and_do_not_repeat_an_account() {
    let limited = MockServer::start().await;
    let ready = MockServer::start().await;
    for server in [&limited, &ready] {
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "7")
                .set_body_json(json!({"error":{"message":"quota","type":"rate_limit_error"}})),
        )
        .expect(1)
        .mount(&limited)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"ready","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02}})))
        .expect(1).mount(&ready).await;
    let (base, token) = start_configured_server(
        vec![
            (
                "limited",
                "llmgateway",
                vec![
                    ("base_url", limited.uri()),
                    ("api_key", "limited-key".into()),
                ],
            ),
            (
                "ready",
                "llmgateway",
                vec![("base_url", ready.uri()), ("api_key", "ready-key".into())],
            ),
        ],
        |config| {
            let auto = config.strategies[0].auto_config.as_mut().unwrap();
            auto.prioritized_models = vec![
                ("limited".into(), "test".into()),
                ("ready".into(), "test".into()),
            ];
            auto.adaptive_routing.max_attempts = Some(1);
        },
        true,
    )
    .await;
    let body = json!({"model":"localrouter/auto","messages":[{"role":"user","content":"hello"}],"extensions":{"localrouter.gateway":{"allow_fallbacks":false}}});
    let first = post(&base, "/v1/chat/completions", &token, &body).await;
    assert_eq!(first.status(), 502);
    let error: Value = first.json().await.unwrap();
    assert_eq!(
        error["extensions"]["localrouter.routing"]["inference_dispatches"],
        1
    );
    let second = post(&base, "/v1/chat/completions", &token, &body).await;
    assert_eq!(second.status(), 200);
    let value: Value = second.json().await.unwrap();
    let trace = &value["extensions"]["localrouter.routing"];
    assert_eq!(trace["inference_dispatches"], 1);
    assert_eq!(trace["attempts"][0]["outcome"], "backoff");
    let seconds = trace["explanation"]["routes"][0]["candidates"][0]["retry_after_secs"]
        .as_u64()
        .unwrap();
    assert!((1..=7).contains(&seconds));
    assert_eq!(trace["attempt_costs"]["reported_usd"], 0.02);
    limited.verify().await;
    ready.verify().await;
}

#[tokio::test]
async fn bounded_http_attempt_timeout_can_fall_back_but_total_deadline_cannot() {
    let slow = MockServer::start().await;
    let ready = MockServer::start().await;
    for server in [&slow, &ready] {
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
            .mount(server)
            .await;
    }
    let reply = json!({"id":"ready","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.02}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(2))
                .set_body_json(reply.clone()),
        )
        .mount(&slow)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply))
        .expect(1)
        .mount(&ready)
        .await;
    for total in [false, true] {
        let (base, token) = start_configured_server(
            vec![
                (
                    "slow",
                    "llmgateway",
                    vec![("base_url", slow.uri()), ("api_key", "slow-key".into())],
                ),
                (
                    "ready",
                    "llmgateway",
                    vec![("base_url", ready.uri()), ("api_key", "ready-key".into())],
                ),
            ],
            move |config| {
                let auto = config.strategies[0].auto_config.as_mut().unwrap();
                auto.prioritized_models = vec![
                    ("slow".into(), "test".into()),
                    ("ready".into(), "test".into()),
                ];
                auto.adaptive_routing.max_attempts = Some(2);
                if total {
                    auto.adaptive_routing.total_timeout_ms = Some(500);
                } else {
                    auto.adaptive_routing.attempt_timeout_ms = Some(500);
                }
                config.pricing_overrides.insert(
                    "llmgateway".into(),
                    [(
                        "test".into(),
                        localrouter::config::ModelPricingOverride {
                            input_per_million: 0.0,
                            output_per_million: 0.0,
                        },
                    )]
                    .into_iter()
                    .collect(),
                );
            },
            true,
        )
        .await;
        let response=post(&base,"/v1/chat/completions",&token,&json!({"model":"localrouter/auto","messages":[{"role":"user","content":"hello"}],"extensions":{"localrouter.gateway":{"allow_fallbacks":false}}})).await;
        if total {
            assert_eq!(response.status(), 504);
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["error"]["code"], "deadline_exceeded");
            assert!(value["extensions"]["localrouter.routing"].is_object());
        } else {
            assert_eq!(response.status(), 200);
            let value: Value = response.json().await.unwrap();
            let trace = &value["extensions"]["localrouter.routing"];
            assert_eq!(trace["inference_dispatches"], 2);
            assert_eq!(trace["attempt_costs"]["unknown_calls"], 1);
            assert_eq!(trace["attempt_costs"]["reported_usd"], 0.02);
        }
    }
    ready.verify().await;
}

#[tokio::test]
async fn streaming_total_deadline_preserves_errors_on_all_surfaces_without_restarting() {
    use futures::StreamExt;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let fallback_calls = Arc::new(AtomicUsize::new(0));
    let primary = primary_calls.clone();
    let fallback = fallback_calls.clone();
    let app=axum::Router::new()
        .route("/models",axum::routing::get(||async {axum::Json(json!({"data":[{"id":"primary"},{"id":"fallback"}]}))}))
        .route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {
            let primary=primary.clone();let fallback=fallback.clone();
            async move {
                if body["model"]=="primary" {primary.fetch_add(1,Ordering::SeqCst);} else {fallback.fetch_add(1,Ordering::SeqCst);}
                let first=json!({"id":"partial","object":"chat.completion.chunk","created":1,"model":"primary","choices":[{"index":0,"delta":{"content":"partial answer"},"finish_reason":null}]});
                let chunks=futures::stream::once(async move {Ok::<_,std::io::Error>(axum::body::Bytes::from(format!("data: {first}\n\n")))})
                    .chain(futures::stream::once(async {tokio::time::sleep(std::time::Duration::from_secs(3)).await;Ok::<_,std::io::Error>(axum::body::Bytes::from("data: [DONE]\n\n"))}));
                axum::response::Response::builder().header("Content-Type","text/event-stream").body(axum::body::Body::from_stream(chunks)).unwrap()
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (base, token, state) = start_replay_test_server(
        vec![(
            "gw",
            "llmgateway",
            vec![("base_url", upstream), ("api_key", "k".into())],
        )],
        |config| {
            let auto = config.strategies[0].auto_config.as_mut().unwrap();
            auto.prioritized_models = vec![
                ("gw".into(), "primary".into()),
                ("gw".into(), "fallback".into()),
            ];
            auto.adaptive_routing.total_timeout_ms = Some(1000);
            config.pricing_overrides.insert(
                "llmgateway".into(),
                ["primary", "fallback"]
                    .into_iter()
                    .map(|model| {
                        (
                            model.into(),
                            localrouter::config::ModelPricingOverride {
                                input_per_million: 0.0,
                                output_per_million: 0.0,
                            },
                        )
                    })
                    .collect(),
            );
        },
        true,
    )
    .await;
    // This fixture targets expiry after the first stream event. Warm model
    // discovery separately so parallel suite startup cannot consume its budget
    // before headers; cold preflight expiry is covered by the bounded tests.
    state
        .provider_registry
        .list_provider_models_cached("gw")
        .await
        .unwrap();
    for (route, input) in [
        (
            "/v1/chat/completions",
            json!({"messages":[{"role":"user","content":"hello"}]}),
        ),
        ("/v1/completions", json!({"prompt":"hello"})),
        ("/v1/responses", json!({"input":"hello","store":false})),
    ] {
        let mut body = input;
        body["model"] = json!("localrouter/auto");
        body["stream"] = json!(true);
        let response = post(&base, route, &token, &body).await;
        assert_eq!(response.status(), 200, "{route}");
        let text = response.text().await.unwrap();
        assert!(text.contains("partial answer"), "{route}: {text}");
        assert!(text.contains("deadline_exceeded"), "{route}: {text}");
        assert!(text.contains("localrouter.routing"), "{route}: {text}");
    }
    assert_eq!(primary_calls.load(Ordering::SeqCst), 3);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn session_headers_pin_allowed_models_and_keep_cache_keys_private() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    for server in [&first, &second] {
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"test"}]})))
            .mount(server)
            .await;
    }
    let reply = json!({"id":"reply","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"prompt_tokens_details":{"cached_tokens":1,"cache_creation_tokens":1},"cost":0.02}});
    let calls = Arc::new(AtomicUsize::new(0));
    let upstream_calls = calls.clone();
    let first_reply = reply.clone();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_: &Request| {
            if upstream_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200).set_body_json(first_reply.clone())
            }
        })
        .expect(3)
        .mount(&first)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply))
        .expect(2)
        .mount(&second)
        .await;
    let (base, token) = start_configured_server(
        vec![
            (
                "first",
                "llmgateway",
                vec![("base_url", first.uri()), ("api_key", "key".into())],
            ),
            (
                "second",
                "llmgateway",
                vec![("base_url", second.uri()), ("api_key", "key".into())],
            ),
        ],
        |config| {
            let auto = config.strategies[0].auto_config.as_mut().unwrap();
            auto.prioritized_models = vec![
                ("first".into(), "test".into()),
                ("second".into(), "test".into()),
            ];
            auto.session_routing.enabled = true;
            auto.session_routing.forward_cache_key = true;
        },
        true,
    )
    .await;
    let body = json!({"model":"localrouter/auto","messages":[{"role":"user","content":"hello"}]});
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&token)
        .header("x-session-id", "session-a")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(
        value["extensions"]["localrouter.routing"]["attempts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["provider"],
        "second"
    );
    assert_eq!(
        value["extensions"]["localrouter.prompt_cache"]["read_tokens"],
        1
    );
    assert_eq!(
        value["extensions"]["localrouter.prompt_cache"]["write_tokens"],
        1
    );
    let mut same = body.clone();
    same["extensions"] = json!({"localrouter.session":{"id":"session-a"}});
    let value: Value = post(&base, "/v1/chat/completions", &token, &same)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        value["extensions"]["localrouter.routing"]["attempts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["provider"],
        "second"
    );
    assert_eq!(
        value["extensions"]["localrouter.routing"]["session"]["status"],
        "hit"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut independent = same.clone();
    independent["extensions"]["localrouter.session"]["id"] = json!("session-b");
    let value: Value = post(&base, "/v1/chat/completions", &token, &independent)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        value["extensions"]["localrouter.routing"]["attempts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["provider"],
        "first"
    );
    same["extensions"]["localrouter.session"]["reset"] = json!(true);
    let value: Value = post(&base, "/v1/chat/completions", &token, &same)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        value["extensions"]["localrouter.routing"]["attempts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["provider"],
        "first"
    );
    let mut bound = body;
    bound["extensions"] =
        json!({"localrouter.session":{"id":"session-a","require_continuity":true}});
    assert_eq!(
        post(&base, "/v1/chat/completions", &token, &bound)
            .await
            .status(),
        400
    );
    let mut keys = Vec::new();
    for server in [&first, &second] {
        for request in server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST")
        {
            let wire: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(!wire.to_string().contains("session-a"));
            assert!(!wire.to_string().contains("session-b"));
            let key = wire["prompt_cache_key"].as_str().unwrap();
            assert_eq!(key.len(), 64);
            keys.push(key.to_string());
        }
    }
    assert!(keys.iter().any(|key| key != &keys[0]));
    first.verify().await;
    second.verify().await;
}

async fn await_replay(manager: &lr_replay::ReplayManager, id: &str) -> lr_replay::ReplayRun {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = manager.get(id).unwrap();
            if run.status != lr_replay::RunStatus::Running {
                return run;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
fn replay_request() -> lr_replay::ReplayRunRequest {
    use lr_replay::*;
    ReplayRunRequest {
        client_id: "test-api-key".into(),
        dataset_id: None,
        max_output_tokens: 32,
        targets: vec![ReplayTarget::Model {
            model: "upstream/replay-fixture".into(),
        }],
        cases: vec![ReplayCase {
            id: "one".into(),
            name: "One".into(),
            input: ReplayInput {
                endpoint: ReplayEndpoint::ChatCompletions,
                body: json!({"messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"do_not_execute","parameters":{"type":"object"}}}],"tool_choice":"required"}),
                source_event_id: None,
            },
            checks: vec![],
        }],
    }
}
#[tokio::test]
async fn replay_uses_normal_client_auth_and_disables_injected_mcp_on_all_chat_surfaces() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"reply","object":"chat.completion","created":1,"model":"replay-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))).expect(3).mount(&upstream).await;
    let (base, _, state) = start_replay_test_server(
        vec![(
            "upstream",
            "openai_compatible",
            vec![("base_url", upstream.uri()), ("api_key", "fixture".into())],
        )],
        |config| {
            config.clients[0].mcp_mode = localrouter::config::McpMode::ViaLlm;
            config.strategies[0].model_permissions = localrouter::config::ModelPermissions {
                global: localrouter::config::PermissionState::Allow,
                ..Default::default()
            };
            config.clients[0].model_permissions = config.strategies[0].model_permissions.clone();
        },
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(lr_replay::ReplayManager::new(dir.path().join("replay")));
    let context = lr_replay::RunContext::new(state, format!("{base}/")).unwrap();
    let mut request = replay_request();
    request.cases.push(lr_replay::ReplayCase {
        id: "two".into(),
        name: "Legacy".into(),
        input: lr_replay::ReplayInput {
            endpoint: lr_replay::ReplayEndpoint::Completions,
            body: json!({"prompt":"hello"}),
            source_event_id: None,
        },
        checks: vec![],
    });
    request.cases.push(lr_replay::ReplayCase {
        id: "three".into(),
        name: "Responses".into(),
        input: lr_replay::ReplayInput {
            endpoint: lr_replay::ReplayEndpoint::Responses,
            body: json!({"input":"hello","tools":[{"type":"web_search"}]}),
            source_event_id: None,
        },
        checks: vec![],
    });
    let id = manager.start(context, request).unwrap();
    let run = await_replay(&manager, &id).await;
    assert!(
        run.results
            .iter()
            .all(|r| r.status == lr_replay::ResultStatus::Success),
        "{:?}",
        run.results
    );
    assert!(!dir.path().join("replay").exists());
    let requests = upstream.received_requests().await.unwrap();
    for req in requests.iter().filter(|r| r.method == "POST") {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert!(body.get("tools").is_none_or(Value::is_null), "{body}");
        assert!(body.get("tool_choice").is_none_or(Value::is_null));
        assert!(body
            .get("extensions")
            .is_none_or(|e| e.get("localrouter.replay").is_none()));
    }
}
#[tokio::test]
async fn replay_cancel_records_unknown_spend_without_retry_and_restart_does_not_resume() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(3))
                .set_body_json(json!({})),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let (base, _, state) = start_replay_test_server(
        vec![(
            "upstream",
            "openai_compatible",
            vec![("base_url", upstream.uri()), ("api_key", "fixture".into())],
        )],
        |config| {
            config.strategies[0].model_permissions = localrouter::config::ModelPermissions {
                global: localrouter::config::PermissionState::Allow,
                ..Default::default()
            };
            config.clients[0].model_permissions = config.strategies[0].model_permissions.clone();
        },
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("replay");
    let manager = Arc::new(lr_replay::ReplayManager::new(root.clone()));
    let id = manager
        .start(
            lr_replay::RunContext::new(state, format!("{base}/")).unwrap(),
            replay_request(),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while upstream
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "POST")
        {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    manager.save_run(&id).unwrap();
    manager.cancel(&id).unwrap();
    let run = await_replay(&manager, &id).await;
    assert_eq!(run.status, lr_replay::RunStatus::Cancelled);
    assert!(run.results[0].cost.unknown_calls > 0);
    let restarted = lr_replay::ReplayManager::new(root);
    assert!(restarted.runs().is_empty());
    let saved = restarted
        .store
        .read(lr_replay::ArtifactKind::Run, &id)
        .unwrap();
    let saved: lr_replay::ReplayRun = serde_json::from_value(saved.value).unwrap();
    assert_eq!(saved.status, lr_replay::RunStatus::Interrupted);
    assert!(saved.results[0].cost.unknown_calls > 0);
    sleep(Duration::from_millis(100)).await;
}
#[tokio::test]
async fn replay_does_not_bypass_denied_model_or_revoked_client() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&upstream)
        .await;
    let (base, _, state) = start_replay_test_server(
        vec![(
            "upstream",
            "openai_compatible",
            vec![("base_url", upstream.uri()), ("api_key", "fixture".into())],
        )],
        |config| {
            config.clients[0].mcp_mode = localrouter::config::McpMode::ViaLlm;
            config.strategies[0].model_permissions = localrouter::config::ModelPermissions {
                global: localrouter::config::PermissionState::Off,
                ..Default::default()
            }
        },
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(lr_replay::ReplayManager::new(dir.path().join("replay")));
    let context = lr_replay::RunContext::new(state.clone(), format!("{base}/")).unwrap();
    let id = manager.start(context.clone(), replay_request()).unwrap();
    let run = await_replay(&manager, &id).await;
    assert_eq!(run.results[0].status, lr_replay::ResultStatus::Error);
    assert_eq!(run.results[0].http_status, Some(403));
    state.client_manager.disable_client("test-api-key").unwrap();
    assert!(manager.start(context, replay_request()).is_err());
}

fn allow_replay_fixture(config: &mut AppConfig) {
    config.strategies[0].model_permissions.global = localrouter::config::PermissionState::Allow;
    config.clients[0].model_permissions = config.strategies[0].model_permissions.clone();
}
#[tokio::test]
async fn replay_policy_uses_shared_rubric_and_reports_classifier_separately() {
    let upstream = laya_mock().await;
    Mock::given(method("POST")).and(path("/v1/systemone")).and(body_partial_json(json!({"questions":{"route":{"type":"choice","criteria":{"fast":"Simple work","deep":"Difficult work"}}}}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"english","answers":{"route":{"type":"choice","choice":"fast","confidence":0.9,"probabilities":{"fast":0.9,"deep":0.1}}},"usage":{"input_tokens":20,"output_tokens":1}}))).expect(1).mount(&upstream).await;
    let (base, _, state) = start_replay_test_server(
        vec![(
            "native",
            "systemone_compatible",
            vec![("base_url", upstream.uri())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(lr_replay::ReplayManager::new(dir.path().join("replay")));
    let mut request = replay_request();
    request.targets = vec![lr_replay::ReplayTarget::Policy {
        name: "Draft policy".into(),
        policy: Box::new(localrouter::config::RoutingPolicy {
            enabled: true,
            decision_model: Some(("native".into(), "english".into())),
            default_route: "fast".into(),
            options: vec![
                localrouter::config::RoutingOption {
                    id: "fast".into(),
                    description: "Simple work".into(),
                    models: vec![],
                },
                localrouter::config::RoutingOption {
                    id: "deep".into(),
                    description: "Difficult work".into(),
                    models: vec![],
                },
            ],
            ..Default::default()
        }),
    }];
    request.cases[0].checks = vec![lr_replay::TaskCheck::ExpectedRoute {
        route: "fast".into(),
    }];
    let context = lr_replay::RunContext::new(state, format!("{base}/")).unwrap();
    manager.preview(context.clone(), request.clone()).unwrap();
    assert!(upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method != "POST"));
    let id = manager.start(context, request).unwrap();
    let run = await_replay(&manager, &id).await;
    let result = &run.results[0];
    assert_eq!(
        result.status,
        lr_replay::ResultStatus::Success,
        "{result:?}"
    );
    assert!(result.checks[0].passed);
    assert_eq!(result.cost.reported_usd, None);
    assert_eq!(result.cost.estimated_usd, None);
    assert_eq!(result.cost.unknown_calls, 0);
    assert_eq!(result.input_tokens, Some(20));
    assert!(result.policy_fingerprint.is_some());
}
#[tokio::test]
async fn replay_judge_is_a_separate_explicit_run_and_never_automatic() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"reply","object":"chat.completion","created":1,"model":"replay-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"{\"score\":0.8,\"reasoning\":\"Accurate\"}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))).expect(2).mount(&upstream).await;
    let (base, _, state) = start_replay_test_server(
        vec![(
            "upstream",
            "openai_compatible",
            vec![("base_url", upstream.uri()), ("api_key", "fixture".into())],
        )],
        allow_replay_fixture,
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(lr_replay::ReplayManager::new(dir.path().join("replay")));
    let context = lr_replay::RunContext::new(state, format!("{base}/")).unwrap();
    let id = manager.start(context.clone(), replay_request()).unwrap();
    let parent = await_replay(&manager, &id).await;
    assert_eq!(parent.results[0].status, lr_replay::ResultStatus::Success);
    assert!(parent.results[0].judge.is_none());
    assert_eq!(
        upstream
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    let child = manager
        .judge(
            context,
            &id,
            "upstream/replay-fixture".into(),
            "Accuracy".into(),
            vec![0],
        )
        .unwrap();
    let judged = await_replay(&manager, &child).await;
    assert_eq!(judged.judge_of.as_deref(), Some(id.as_str()));
    assert_eq!(judged.results[0].judge.as_ref().unwrap().score, 0.8);
    assert!(manager.get(&id).unwrap().results[0].judge.is_none());
}

async fn exact_cache_fixture(
    expected: u64,
    enabled: bool,
) -> (MockServer, String, String, server::state::AppState) {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"cache-fixture"}]})),
        )
        .mount(&upstream)
        .await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"stream":false}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"original-paid","object":"chat.completion","created":1,"model":"cache-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"Cached answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3,"cost":0.02},"provider":"fixture-inference"}))).expect(expected).mount(&upstream).await;
    let (base, secret, state) = start_replay_test_server(
        vec![(
            "cached",
            "llmgateway",
            vec![("base_url", upstream.uri()), ("api_key", "fixture".into())],
        )],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].response_cache.enabled = enabled;
            let mut strategy = config.strategies[0].clone();
            strategy.id = "other-strategy".into();
            strategy.parent = Some("other-client".into());
            let mut client = create_test_client("other-client", "other-strategy");
            client.model_permissions = strategy.model_permissions.clone();
            config.clients.push(client);
            config.strategies.push(strategy);
        },
        true,
    )
    .await;
    state
        .provider_registry
        .list_provider_models_cached("cached")
        .await
        .unwrap();
    (upstream, base, secret, state)
}
fn exact_cache_chat() -> Value {
    json!({"model":"cached/cache-fixture","messages":[{"role":"user","content":"hello"}],"max_tokens":32})
}
#[tokio::test]
async fn exact_cache_hits_have_fresh_ids_zero_spend_and_endpoint_isolation() {
    let (_upstream, base, secret, state) = exact_cache_fixture(5, true).await;
    use localrouter::router::rate_limit::RateLimitType;
    state.rate_limiter.add_api_key_limiters(
        "test-api-key".into(),
        vec![
            localrouter::router::rate_limit::RateLimiter::new(RateLimitType::Cost, 100.0, 60),
            localrouter::router::rate_limit::RateLimiter::new(RateLimitType::Requests, 100.0, 60),
            localrouter::router::rate_limit::RateLimiter::new(
                RateLimitType::TotalTokens,
                10000.0,
                60,
            ),
        ],
    );
    for (route, body) in [
        ("/v1/chat/completions", exact_cache_chat()),
        (
            "/v1/completions",
            json!({"model":"cached/cache-fixture","prompt":"hello","max_tokens":32}),
        ),
        (
            "/v1/responses",
            json!({"model":"cached/cache-fixture","input":"hello","max_output_tokens":32}),
        ),
    ] {
        let first = post(&base, route, &secret, &body).await;
        assert_eq!(first.status(), 200);
        let first: Value = first.json().await.unwrap();
        let second = post(&base, route, &secret, &body).await;
        assert_eq!(second.status(), 200);
        let second: Value = second.json().await.unwrap();
        let receipt = &second["extensions"]["localrouter.response_cache"];
        assert_eq!(receipt["hit"], true, "{route}: {second}");
        assert_eq!(receipt["upstream_spend_usd"], 0.0);
        assert_eq!(receipt["billed_input_tokens"], 0);
        assert_ne!(first["id"], second["id"]);
        assert!(!second
            .to_string()
            .contains("localrouter.response_cache_scope"));
    }
    let used = state
        .rate_limiter
        .get_api_key_usage("test-api-key", RateLimitType::Cost)
        .await
        .unwrap()
        .0;
    assert!((used - 0.06).abs() < 1e-10, "{used}");
    assert_eq!(
        state
            .rate_limiter
            .get_api_key_usage("test-api-key", RateLimitType::TotalTokens)
            .await
            .unwrap()
            .0,
        9.0
    );
    assert_eq!(
        state
            .rate_limiter
            .get_api_key_usage("test-api-key", RateLimitType::Requests)
            .await
            .unwrap()
            .0,
        6.0
    );
    let mut bypass = exact_cache_chat();
    bypass["extensions"] = json!({"localrouter.response_cache":{"mode":"bypass"}});
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &bypass)
            .await
            .status(),
        200
    );
    bypass["extensions"]["localrouter.response_cache"]["mode"] = json!("refresh");
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &bypass)
            .await
            .status(),
        200
    );
    let stats = state.router.response_cache_stats(Some("test-api-key"));
    assert_eq!(stats.hits, 3);
    assert_eq!(stats.entries, 3);
    assert!(stats.avoided_cost_estimate_usd.is_some());
}
#[tokio::test]
async fn exact_cache_partitioning_survives_rotation_and_rechecks_revocation() {
    let (upstream, base, secret, state) = exact_cache_fixture(4, true).await;
    let other = state.config_manager.get().clients[1].clone();
    let other_secret = state.client_manager.add_client_with_secret(other).unwrap();
    let body = exact_cache_chat();
    for key in [&secret, &other_secret] {
        for _ in 0..2 {
            assert_eq!(
                post(&base, "/v1/chat/completions", key, &body)
                    .await
                    .status(),
                200
            );
        }
    }
    assert_eq!(
        upstream
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        2
    );
    let rotated = state.client_manager.rotate_secret("other-client").unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &other_secret, &body)
            .await
            .status(),
        401
    );
    for _ in 0..2 {
        assert_eq!(
            post(&base, "/v1/chat/completions", &rotated, &body)
                .await
                .status(),
            200
        );
    }
    state
        .config_manager
        .update(|c| {
            c.strategies[1].model_permissions.global = localrouter::config::PermissionState::Off
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &rotated, &body)
            .await
            .status(),
        403
    );
    state
        .config_manager
        .update(|c| {
            c.strategies[1].model_permissions.global = localrouter::config::PermissionState::Allow;
            c.strategies[1].response_cache.enabled = false;
        })
        .unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &rotated, &body)
            .await
            .status(),
        200
    );
    state.provider_registry.remove_provider("cached").unwrap();
    assert_ne!(
        post(&base, "/v1/chat/completions", &secret, &body)
            .await
            .status(),
        200
    );
    state.client_manager.disable_client("test-api-key").unwrap();
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &body)
            .await
            .status(),
        401
    );
}
#[tokio::test]
async fn exact_cache_is_off_by_default_and_replay_or_session_requests_always_bypass() {
    let (_upstream, base, secret, state) = exact_cache_fixture(6, true).await;
    for controls in [
        json!({"localrouter.replay":{"tools":"disabled"}}),
        json!({"localrouter.session":{"id":"private-session"}}),
    ] {
        let mut body = exact_cache_chat();
        body["extensions"] = controls;
        for _ in 0..2 {
            let response = post(&base, "/v1/chat/completions", &secret, &body).await;
            assert_eq!(response.status(), 200);
            let response: Value = response.json().await.unwrap();
            assert!(response["extensions"]["localrouter.response_cache"].is_null());
        }
    }
    state
        .config_manager
        .update(|config| config.strategies[0].response_cache.enabled = false)
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            post(&base, "/v1/chat/completions", &secret, &exact_cache_chat())
                .await
                .status(),
            200
        );
    }
    assert_eq!(state.router.response_cache_stats(None).entries, 0);
    let mut spoof = exact_cache_chat();
    spoof["extensions"] = json!({"localrouter.response_cache_scope":{"credential":"forged","endpoint":"/v1/chat/completions"}});
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &spoof)
            .await
            .status(),
        400
    );
}
#[tokio::test]
async fn exact_cache_native_decisions_keep_served_usage_but_do_not_reuse_upstream_request_ids() {
    let upstream = laya_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "original-request")
                .set_body_json(native_answer()),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let (base, secret, state) = start_replay_test_server(
        vec![(
            "native",
            "systemone_compatible",
            vec![("base_url", upstream.uri())],
        )],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].response_cache.enabled = true;
        },
        true,
    )
    .await;
    let body = decision_body("native/english");
    let first = post(&base, "/v1/systemone", &secret, &body).await;
    assert_eq!(first.status(), 200);
    assert_eq!(first.headers()["x-typesafe-request-id"], "original-request");
    let second = post(&base, "/v1/systemone", &secret, &body).await;
    assert_eq!(second.status(), 200);
    assert!(second.headers().get("x-typesafe-request-id").is_none());
    let body: Value = second.json().await.unwrap();
    assert_eq!(
        body["extensions"]["localrouter.response_cache"]["hit"], true,
        "{body}"
    );
    assert_eq!(body["usage"]["input_tokens"], 120);
    assert_eq!(
        body["extensions"]["localrouter.response_cache"]["billed_input_tokens"],
        0
    );
    assert_eq!(state.router.response_cache_stats(None).hits, 1);
}

#[tokio::test]
async fn exact_cache_does_not_store_tool_requests_or_streams() {
    let (upstream, base, secret, state) = exact_cache_fixture(2, true).await;
    let chunk = json!({"id":"stream","object":"chat.completion.chunk","created":1,"model":"cache-fixture","choices":[{"index":0,"delta":{"content":"Text"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3,"cost":0.02}});
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"stream":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n")),
        )
        .expect(2)
        .mount(&upstream)
        .await;
    let mut tools = exact_cache_chat();
    tools["tools"] = json!([{"type":"function","function":{"name":"external_action","parameters":{"type":"object"}}}]);
    for _ in 0..2 {
        let response = post(&base, "/v1/chat/completions", &secret, &tools).await;
        assert_eq!(response.status(), 200);
    }
    let mut stream = exact_cache_chat();
    stream["stream"] = json!(true);
    for _ in 0..2 {
        let response = post(&base, "/v1/chat/completions", &secret, &stream).await;
        assert_eq!(response.status(), 200);
        let body = response.text().await.unwrap();
        assert!(body.contains("[DONE]"));
        assert!(!body.contains("localrouter.response_cache_pending"));
    }
    assert_eq!(state.router.response_cache_stats(None).entries, 0);
}
#[tokio::test]
async fn exact_cache_rechecks_current_request_quota_before_serving_a_hit() {
    let (_upstream, base, secret, state) = exact_cache_fixture(1, true).await;
    state.rate_limiter.add_api_key_limiters(
        "test-api-key".into(),
        vec![localrouter::router::rate_limit::RateLimiter::new(
            localrouter::router::rate_limit::RateLimitType::Requests,
            1.0,
            60,
        )],
    );
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &exact_cache_chat())
            .await
            .status(),
        200
    );
    let response = post(&base, "/v1/chat/completions", &secret, &exact_cache_chat()).await;
    assert_eq!(response.status(), 429);
    assert_eq!(state.router.response_cache_stats(None).hits, 0);
}

#[tokio::test]
async fn exact_cache_cannot_revive_a_model_disabled_in_the_provider_registry() {
    let (_upstream, base, secret, state) = exact_cache_fixture(1, true).await;
    let body = exact_cache_chat();
    for _ in 0..2 {
        assert_eq!(
            post(&base, "/v1/chat/completions", &secret, &body)
                .await
                .status(),
            200
        );
    }
    assert_eq!(state.router.response_cache_stats(None).hits, 1);
    state
        .provider_registry
        .set_model_settings(
            "cached",
            localrouter::config::ProviderModelSettings {
                entries: vec![localrouter::config::ModelEntry {
                    id: "cache-fixture".into(),
                    enabled: Some(false),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    assert_ne!(
        post(&base, "/v1/chat/completions", &secret, &body)
            .await
            .status(),
        200
    );
    assert_eq!(state.router.response_cache_stats(None).hits, 1);
}
#[tokio::test]
async fn exact_cache_requires_fresh_approval_when_a_temporary_grant_expires() {
    let (_upstream, base, secret, state) = exact_cache_fixture(1, true).await;
    state
        .config_manager
        .update(|config| {
            config.strategies[0].model_permissions.global =
                localrouter::config::PermissionState::Ask
        })
        .unwrap();
    state.model_approval_tracker.add_approval(
        "test-api-key",
        "cached",
        "cache-fixture",
        Duration::from_secs(60),
    );
    assert_eq!(
        post(&base, "/v1/chat/completions", &secret, &exact_cache_chat())
            .await
            .status(),
        200
    );
    assert_eq!(state.router.response_cache_stats(None).entries, 1);
    state.model_approval_tracker.add_approval(
        "test-api-key",
        "cached",
        "cache-fixture",
        Duration::ZERO,
    );
    let request = {
        let base = base.clone();
        let secret = secret.clone();
        tokio::spawn(async move {
            post(&base, "/v1/chat/completions", &secret, &exact_cache_chat()).await
        })
    };
    let pending = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(pending) = state
                .mcp_gateway
                .firewall_manager
                .list_pending()
                .into_iter()
                .next()
            {
                break pending;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(state.router.response_cache_stats(None).hits, 0);
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &pending.request_id,
            localrouter::mcp::gateway::firewall::FirewallApprovalAction::Deny,
            None,
        )
        .unwrap();
    assert_eq!(request.await.unwrap().status(), 403);
    let events = state.monitor_store.list(0, 20, None).events;
    let decision = events
        .iter()
        .find(|e| e.event_type == lr_monitor::MonitorEventType::FirewallDecision)
        .expect("model denial is captured");
    assert!(decision.session_id.is_some());
    let call = events
        .iter()
        .find(|e| {
            e.event_type == lr_monitor::MonitorEventType::LlmCall
                && e.session_id == decision.session_id
        })
        .expect("decision belongs to the denied request");
    assert_eq!(call.status, lr_monitor::EventStatus::Error);
    assert_eq!(
        call.conversation.as_ref().unwrap().id,
        decision.conversation.as_ref().unwrap().id
    );

    state.model_approval_tracker.add_approval(
        "test-api-key",
        "cached",
        "cache-fixture",
        Duration::from_secs(60),
    );
    let response = post(&base, "/v1/chat/completions", &secret, &exact_cache_chat()).await;
    assert_eq!(response.status(), 200);
    let response: Value = response.json().await.unwrap();
    assert_eq!(
        response["extensions"]["localrouter.response_cache"]["hit"],
        true
    );
    assert_eq!(state.router.response_cache_stats(None).hits, 1);
}

#[tokio::test]
async fn exact_cache_zero_model_spend_does_not_hide_fresh_classifier_charges() {
    let (_upstream, base, secret, state) = exact_cache_fixture(1, true).await;
    let classifier = laya_mock().await;
    Mock::given(method("POST")).and(path("/v1/systemone")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"english","answers":{"route":{"type":"choice","choice":"fast","confidence":0.9,"probabilities":{"fast":0.9,"deep":0.1}}},"usage":{"input_tokens":3,"output_tokens":1,"cost":0.01}}))).expect(2).mount(&classifier).await;
    state
        .provider_registry
        .create_provider(
            "classifier".into(),
            "systemone_compatible".into(),
            [("base_url".into(), classifier.uri())]
                .into_iter()
                .collect(),
        )
        .await
        .unwrap();
    state
        .provider_registry
        .list_provider_models_cached("classifier")
        .await
        .unwrap();
    state
        .config_manager
        .update(|config| {
            let auto = config.strategies[0].auto_config.as_mut().unwrap();
            auto.prioritized_models = vec![("cached".into(), "cache-fixture".into())];
            auto.routing_policy = Some(localrouter::config::RoutingPolicy {
                enabled: true,
                decision_model: Some(("classifier".into(), "english".into())),
                default_route: "fast".into(),
                options: vec![
                    localrouter::config::RoutingOption {
                        id: "fast".into(),
                        description: "Simple".into(),
                        models: vec![],
                    },
                    localrouter::config::RoutingOption {
                        id: "deep".into(),
                        description: "Complex".into(),
                        models: vec![],
                    },
                ],
                ..Default::default()
            });
        })
        .unwrap();
    state.rate_limiter.add_api_key_limiters(
        "test-api-key".into(),
        vec![localrouter::router::rate_limit::RateLimiter::new(
            localrouter::router::rate_limit::RateLimitType::Cost,
            100.0,
            60,
        )],
    );
    let mut body = exact_cache_chat();
    body["model"] = json!("localrouter/auto");
    let first = post(&base, "/v1/chat/completions", &secret, &body).await;
    assert_eq!(first.status(), 200);
    let second = post(&base, "/v1/chat/completions", &secret, &body).await;
    assert_eq!(second.status(), 200);
    let response: Value = second.json().await.unwrap();
    assert_eq!(
        response["extensions"]["localrouter.response_cache"]["scope"], "resolved_model_call",
        "{response}"
    );
    assert_eq!(
        response["extensions"]["localrouter.response_cache"]["upstream_spend_usd"],
        0.0
    );
    assert_eq!(
        response["extensions"]["localrouter.routing"]["decision_routing"]["classifier_cost_usd"],
        0.01
    );
    let used = state
        .rate_limiter
        .get_api_key_usage(
            "test-api-key",
            localrouter::router::rate_limit::RateLimitType::Cost,
        )
        .await
        .unwrap()
        .0;
    assert!((used - 0.04).abs() < 1e-10, "{used}");
}

#[tokio::test]
async fn exact_cache_native_vendor_fields_are_preserved_in_keys_without_sidecar_collisions() {
    let upstream = laya_mock().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut answer = native_answer();
            if body["_original_decisions"] == "second" {
                answer["answers"]["dept"]["choice"] = json!("tech");
                answer["answers"]["dept"]["probabilities"] = json!({"billing":0.05,"tech":0.95});
            }
            ResponseTemplate::new(200).set_body_json(answer)
        })
        .expect(2)
        .mount(&upstream)
        .await;
    let (base, secret, _state) = start_replay_test_server(
        vec![(
            "native",
            "systemone_compatible",
            vec![("base_url", upstream.uri())],
        )],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].response_cache.enabled = true;
        },
        true,
    )
    .await;
    for (parameter, expected) in [("first", "billing"), ("second", "tech")] {
        let mut body = decision_body("native/english");
        body["_original_decisions"] = json!(parameter);
        for _ in 0..2 {
            let response = post(&base, "/v1/systemone", &secret, &body).await;
            assert_eq!(response.status(), 200);
            let answer: Value = response.json().await.unwrap();
            assert_eq!(answer["answers"]["dept"]["choice"], expected);
        }
    }
}
#[tokio::test]
async fn exact_cache_decisions_refresh_replaces_the_same_typed_input_entry() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let upstream = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"decider","architecture":{"output_modalities":["decision"],"input_modalities":["text"]}}]}))).mount(&upstream).await;
    Mock::given(method("POST")).and(path("/v1/decisions")).respond_with(move |_:&Request| {let value=observed.fetch_add(1,Ordering::SeqCst)==0;ResponseTemplate::new(200).set_body_json(json!({"model":"decider","answers":[{"type":"choice","name":"pick","choice":value,"confidence":0.9,"probabilities":[{"value":value,"probability":0.9},{"value":!value,"probability":0.1}]}],"usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}))}).expect(2).mount(&upstream).await;
    let (base, secret, _state) = start_replay_test_server(
        vec![(
            "native",
            "llmgateway",
            vec![
                ("base_url", format!("{}/v1", upstream.uri())),
                ("api_key", "fixture".into()),
            ],
        )],
        |config| {
            allow_replay_fixture(config);
            config.strategies[0].response_cache.enabled = true;
        },
        true,
    )
    .await;
    let mut body = json!({"model":"native/decider","input":"choose","questions":[{"type":"choice","name":"pick","instructions":"Pick one","choices":[{"value":true},{"value":false}]}]});
    for (mode, expected, hit) in [
        ("use", true, false),
        ("use", true, true),
        ("refresh", false, false),
        ("use", false, true),
    ] {
        body["extensions"] = json!({"localrouter.response_cache":{"mode":mode}});
        let response = post(&base, "/v1/decisions", &secret, &body).await;
        assert_eq!(response.status(), 200);
        let answer: Value = response.json().await.unwrap();
        assert_eq!(answer["answers"][0]["choice"], expected, "{answer}");
        assert_eq!(
            answer["extensions"]["localrouter.response_cache"]["hit"]
                .as_bool()
                .unwrap_or(false),
            hit,
            "{answer}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[path = "fixtures/messages_endpoint_cases.rs"]
mod messages_endpoint_cases;

#[path = "fixtures/files_endpoint_cases.rs"]
mod files_endpoint_cases;

#[path = "fixtures/batches_endpoint_cases.rs"]
mod batches_endpoint_cases;

#[path = "fixtures/credential_security_cases.rs"]
mod credential_security_cases;

#[path = "fixtures/named_routes_endpoint_cases.rs"]
mod named_routes_endpoint_cases;

#[path = "fixtures/upstream_tools_endpoint_cases.rs"]
mod upstream_tools_endpoint_cases;

#[path = "fixtures/ranked_search_endpoint_cases.rs"]
mod ranked_search_endpoint_cases;

#[path = "fixtures/documents_endpoint_cases.rs"]
mod documents_endpoint_cases;

#[path = "fixtures/ocr_endpoint_cases.rs"]
mod ocr_endpoint_cases;

#[path = "fixtures/rerank_endpoint_cases.rs"]
mod rerank_endpoint_cases;

#[path = "fixtures/videos_endpoint_cases.rs"]
mod videos_endpoint_cases;

#[path = "fixtures/realtime_endpoint_cases.rs"]
mod realtime_endpoint_cases;

#[path = "fixtures/routing_profiles_endpoint_cases.rs"]
mod routing_profiles_endpoint_cases;
