//! A chat response that JSON repair changes records a JsonRepair monitor
//! event next to its LLM call; a valid response records none.

use std::collections::HashMap;
use std::sync::Arc;

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, ConfigManager};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::factory::OpenAICompatibleProviderFactory;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server::{self, state::AppState};
use lr_monitor::{MonitorEventData, MonitorEventType};
use serde_json::{json, Value};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn start(upstream: &MockServer) -> (String, AppState) {
    let config_path =
        std::env::temp_dir().join(format!("test_json_repair_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(AppConfig::default(), config_path));
    let provider_registry = Arc::new(ProviderRegistry::new());
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    let cfg: HashMap<String, String> = [
        ("base_url".to_string(), upstream.uri()),
        ("api_key".to_string(), "k".to_string()),
    ]
    .into();
    provider_registry
        .create_provider("compat".into(), "openai_compatible".into(), cfg)
        .await
        .expect("create provider");
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_json_repair_{}.db", uuid::Uuid::new_v4()));
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
    let (state, _handle, port, _shutdown) = server::start_server(
        server::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            enable_cors: false,
        },
        router,
        Arc::new(McpServerManager::new()),
        rate_limiter,
        provider_registry,
        config_manager,
        Arc::new(ClientManager::new(vec![])),
        Arc::new(TokenStore::new()),
        metrics_collector,
        None,
    )
    .await
    .expect("server starts");
    (format!("http://127.0.0.1:{port}"), state)
}

async fn mock_models(upstream: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "gpt"}]})))
        .mount(upstream)
        .await;
}

async fn mock_completion(upstream: &MockServer, content: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c1", "object": "chat.completion", "created": 0, "model": "gpt",
            "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": content}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10}
        })))
        .mount(upstream)
        .await;
}

async fn chat(base_url: &str, state: &AppState, stream: bool) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base_url}/v1/chat/completions"))
        .bearer_auth(state.get_internal_test_secret())
        .json(&json!({
            "model": "compat/gpt",
            "stream": stream,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "user", "content": "Classify the ticket"}]
        }))
        .send()
        .await
        .expect("request sent")
}

/// JSON repair events in the store, with the session of their LLM call.
fn repair_events(state: &AppState) -> Vec<lr_monitor::MonitorEvent> {
    let listed = state.monitor_store.list(0, 100, None);
    listed
        .events
        .iter()
        .filter(|e| e.event_type == MonitorEventType::JsonRepair)
        .map(|e| state.monitor_store.get(&e.id).expect("stored"))
        .collect()
}

fn llm_session(state: &AppState) -> Option<String> {
    state
        .monitor_store
        .list(0, 100, None)
        .events
        .into_iter()
        .find(|e| e.event_type == MonitorEventType::LlmCall)
        .and_then(|e| e.session_id)
}

#[tokio::test]
async fn repaired_response_records_one_event() {
    let upstream = MockServer::start().await;
    mock_models(&upstream).await;
    mock_completion(&upstream, "```json\n{\"department\": \"billing\",}\n```").await;
    let (base_url, state) = start(&upstream).await;

    let resp = chat(&base_url, &state, false).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(content).unwrap(),
        json!({"department": "billing"})
    );

    let events = repair_events(&state);
    assert_eq!(events.len(), 1);
    assert!(events[0].session_id.is_some());
    assert_eq!(events[0].session_id, llm_session(&state));
    match &events[0].data {
        MonitorEventData::JsonRepair {
            streamed,
            repairs,
            original,
            repaired,
            ..
        } => {
            assert!(!streamed);
            assert!(repairs.contains(&"Stripped markdown code fences".to_string()));
            assert!(original.as_deref().unwrap().starts_with("```json"));
            assert_eq!(
                serde_json::from_str::<Value>(repaired.as_deref().unwrap()).unwrap(),
                json!({"department": "billing"})
            );
        }
        other => panic!("unexpected data {other:?}"),
    }
}

#[tokio::test]
async fn valid_response_records_no_event() {
    let upstream = MockServer::start().await;
    mock_models(&upstream).await;
    mock_completion(&upstream, "{\"department\": \"billing\"}").await;
    let (base_url, state) = start(&upstream).await;

    let resp = chat(&base_url, &state, false).await;
    assert_eq!(resp.status(), 200);
    resp.bytes().await.unwrap();
    assert!(repair_events(&state).is_empty());
}

#[tokio::test]
async fn repaired_stream_records_one_event() {
    let upstream = MockServer::start().await;
    mock_models(&upstream).await;
    let chunk = |content: &str, finish: Option<&str>| {
        format!(
            "data: {}\n\n",
            json!({
                "id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "gpt",
                "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": finish}]
            })
        )
    };
    let sse = [
        chunk("```json\n{\"department\": ", None),
        chunk("\"billing\"", None),
        chunk("", Some("stop")),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&upstream)
        .await;
    let (base_url, state) = start(&upstream).await;

    let resp = chat(&base_url, &state, true).await;
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("[DONE]"));

    let events = repair_events(&state);
    assert_eq!(events.len(), 1, "stream: {text}");
    assert_eq!(events[0].session_id, llm_session(&state));
    match &events[0].data {
        MonitorEventData::JsonRepair {
            streamed,
            repairs,
            original,
            ..
        } => {
            assert!(streamed);
            assert!(repairs.contains(&"Stripped markdown code fences".to_string()));
            assert!(repairs.contains(&"Fixed JSON syntax".to_string()));
            assert_eq!(
                repairs.iter().filter(|r| *r == "Fixed JSON syntax").count(),
                1
            );
            assert!(original.is_none());
        }
        other => panic!("unexpected data {other:?}"),
    }
}
