//! Streaming usage through `/v1/chat/completions`: the upstream's own usage
//! (sent after the finish reason, the OpenAI way) is what gets recorded and
//! what a client asking for `stream_options.include_usage` receives, and the
//! provider that served the request is attributed even when the model has no
//! `provider/` prefix.

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration, Instant};

const PROMPT_TOKENS: u64 = 1234;
const COMPLETION_TOKENS: u64 = 56;

/// A tool-call-only turn: no content at all, then the finish reason, then a
/// usage-only chunk, then `[DONE]`.
fn upstream_sse() -> String {
    let chunks = [
        r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"record","arguments":"{\"word\":\"hello\"}"}}]},"finish_reason":null}]}"#.to_string(),
        r#"{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#.to_string(),
        format!(
            r#"{{"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[],"usage":{{"prompt_tokens":{PROMPT_TOKENS},"completion_tokens":{COMPLETION_TOKENS},"total_tokens":{}}}}}"#,
            PROMPT_TOKENS + COMPLETION_TOKENS
        ),
    ];
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

async fn spawn_mock_upstream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let mut req = String::new();
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            req.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if req.contains("\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let (content_type, body) =
                    if req.starts_with("POST") && req.contains("chat/completions") {
                        ("text/event-stream", upstream_sse())
                    } else {
                        (
                            "application/json",
                            r#"{"object":"list","data":[{"id":"test-model","object":"model"}]}"#
                                .to_string(),
                        )
                    };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    format!("http://{}", addr)
}

async fn start_test_server(upstream_url: String) -> (String, String, Arc<MetricsCollector>) {
    let mut test_client =
        Client::new_with_strategy("Test Client".to_string(), "default".to_string());
    test_client.id = "test-api-key".to_string();
    test_client.enabled = true;
    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![Strategy::new("Default".to_string())],
        ..Default::default()
    };
    let config_path =
        std::env::temp_dir().join(format!("test_stream_usage_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    provider_registry.register_factory(Arc::new(
        localrouter::providers::factory::OpenAICompatibleProviderFactory,
    ));
    let mut provider_config = HashMap::new();
    provider_config.insert("base_url".to_string(), upstream_url);
    provider_config.insert("api_key".to_string(), "test-key".to_string());
    provider_registry
        .create_provider(
            "mockai".to_string(),
            "openai_compatible".to_string(),
            provider_config,
        )
        .await
        .expect("create mock provider");

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_stream_usage_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));
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
            port: 44000 + (std::process::id() % 10000) as u16,
            enable_cors: true,
        },
        router,
        Arc::new(McpServerManager::new()),
        rate_limiter,
        provider_registry,
        config_manager,
        Arc::new(ClientManager::new(vec![test_client])),
        Arc::new(TokenStore::new()),
        metrics_collector.clone(),
        None,
    )
    .await
    .expect("Failed to start test server");
    sleep(Duration::from_millis(200)).await;
    (
        format!("http://127.0.0.1:{}", port),
        state.get_internal_test_secret(),
        metrics_collector,
    )
}

/// The `data:` payloads of an SSE body.
async fn stream_events(base_url: &str, secret: &str, body: serde_json::Value) -> Vec<String> {
    let text = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", base_url))
        .bearer_auth(secret)
        .json(&body)
        .send()
        .await
        .expect("request send")
        .text()
        .await
        .expect("stream body");
    text.lines()
        .filter_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:")))
        .map(|l| l.trim().to_string())
        .collect()
}

fn request(include_usage: Option<bool>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": "mockai/test-model",
        "stream": true,
        "messages": [{"role": "user", "content": "say hello with the tool"}],
    });
    if let Some(include_usage) = include_usage {
        body["stream_options"] = serde_json::json!({ "include_usage": include_usage });
    }
    body
}

#[tokio::test]
async fn include_usage_gets_the_upstream_usage_before_done() {
    let upstream = spawn_mock_upstream().await;
    let (base_url, secret, metrics) = start_test_server(upstream).await;

    let events = stream_events(&base_url, &secret, request(Some(true))).await;
    assert_eq!(
        events.last().map(String::as_str),
        Some("[DONE]"),
        "{events:?}"
    );
    let usage_chunk: serde_json::Value =
        serde_json::from_str(&events[events.len() - 2]).expect("usage chunk is JSON");
    assert_eq!(usage_chunk["choices"], serde_json::json!([]));
    assert_eq!(usage_chunk["usage"]["prompt_tokens"], PROMPT_TOKENS);
    assert_eq!(usage_chunk["usage"]["completion_tokens"], COMPLETION_TOKENS);
    assert_eq!(
        usage_chunk["usage"]["total_tokens"],
        PROMPT_TOKENS + COMPLETION_TOKENS
    );
    // Only our own usage chunk: the upstream's is not forwarded as well
    let with_usage = events.iter().filter(|e| e.contains("\"usage\"")).count();
    assert_eq!(with_usage, 1, "{events:?}");

    // Recorded with the upstream's numbers and the serving provider
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let totals = metrics.get_usage_for_type("llm_provider:mockai", 3600);
        if totals.requests > 0 {
            assert_eq!(totals.tokens, PROMPT_TOKENS + COMPLETION_TOKENS);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no usage recorded for provider mockai; providers: {:?}",
            metrics.get_provider_names()
        );
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn without_include_usage_no_usage_chunk_is_sent() {
    let upstream = spawn_mock_upstream().await;
    let (base_url, secret, _metrics) = start_test_server(upstream).await;

    for include_usage in [None, Some(false)] {
        let events = stream_events(&base_url, &secret, request(include_usage)).await;
        assert_eq!(
            events.last().map(String::as_str),
            Some("[DONE]"),
            "{events:?}"
        );
        assert!(
            events.iter().all(|e| !e.contains("\"usage\"")),
            "{events:?}"
        );
        assert!(
            events.iter().any(|e| e.contains("\"tool_calls\"")),
            "{events:?}"
        );
    }
}
