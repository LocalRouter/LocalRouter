//! Regression tests: stopping the server (or a client disconnecting) must
//! cancel in-flight upstream provider requests, not just the downstream
//! response.
//!
//! Local providers like Ollama keep generating for as long as the HTTP
//! connection to them stays open. The bug: the streaming worker only noticed
//! a dead downstream on its next chunk send — during a provider's silent
//! prefill phase (no chunks yet) the upstream connection stayed open
//! indefinitely, so Ollama kept processing after Stop. The mock upstream
//! below streams one chunk and then goes silent, reproducing exactly that
//! phase, and records when the gateway actually closes the connection.

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout, Duration, Instant};

/// Mock OpenAI-compatible upstream:
/// - answers GET /models (and friends) with a one-model list
/// - answers POST /chat/completions with an SSE stream that emits ONE chunk
///   and then goes silent (simulating a model busy in prefill / slow
///   generation), while watching for the client to close the connection
///
/// `disconnected` flips to true the moment the gateway drops the connection.
async fn spawn_mock_upstream(disconnected: Arc<AtomicBool>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let disconnected = disconnected.clone();

            tokio::spawn(async move {
                // Read the request head
                let mut buf = vec![0u8; 8192];
                let mut req = String::new();
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) => return,
                        Ok(n) => {
                            req.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if req.contains("\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }

                if req.starts_with("POST") && req.contains("chat/completions") {
                    // SSE response: one chunk, then silence. Connection: close
                    // makes body framing "read until close" which reqwest
                    // accepts for streams.
                    let chunk = "data: {\"id\":\"cmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\n";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{}",
                        chunk
                    );
                    if socket.write_all(head.as_bytes()).await.is_err() {
                        disconnected.store(true, Ordering::SeqCst);
                        return;
                    }
                    let _ = socket.flush().await;

                    // Silent "prefill": no more chunks. Detect the client
                    // closing the connection (read returns 0 / error).
                    let mut probe = [0u8; 64];
                    loop {
                        match timeout(Duration::from_secs(30), socket.read(&mut probe)).await {
                            Ok(Ok(0)) | Ok(Err(_)) => {
                                disconnected.store(true, Ordering::SeqCst);
                                return;
                            }
                            Ok(Ok(_)) => continue, // stray bytes; keep watching
                            Err(_) => return,      // gave up waiting — test will fail
                        }
                    }
                } else {
                    // Model listing / anything else: small JSON body
                    let body = "{\"object\":\"list\",\"data\":[{\"id\":\"test-model\",\"object\":\"model\"}]}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    format!("http://{}", addr)
}

fn create_test_client(id: &str, strategy_id: &str) -> Client {
    let mut client = Client::new_with_strategy("Test Client".to_string(), strategy_id.to_string());
    client.id = id.to_string();
    client.enabled = true;
    client
}

/// Boot the real HTTP server with the mock upstream registered as an
/// openai-compatible provider. Returns (base_url, bearer secret, shutdown
/// token) — cancelling the token is exactly what the Stop button does.
async fn start_test_server(
    upstream_url: String,
) -> (String, String, tokio_util::sync::CancellationToken) {
    let test_client = create_test_client("test-api-key", "default");
    let strategy = Strategy::new("Default".to_string());

    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![strategy],
        ..Default::default()
    };

    let config_path =
        std::env::temp_dir().join(format!("test_stop_cancel_{}.yaml", uuid::Uuid::new_v4()));
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

    let mcp_server_manager = Arc::new(McpServerManager::new());
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_stop_cancel_{}.db", uuid::Uuid::new_v4()));
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
    let client_manager = Arc::new(ClientManager::new(vec![test_client]));
    let token_store = Arc::new(TokenStore::new());

    let test_port = 42000 + (std::process::id() % 10000) as u16;
    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: test_port,
        enable_cors: true,
    };

    let (state, _handle, actual_port, shutdown) = server::start_server(
        server_config,
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
        None,
    )
    .await
    .expect("Failed to start test server");

    let base_url = format!("http://127.0.0.1:{}", actual_port);
    let secret = state.get_internal_test_secret();
    sleep(Duration::from_millis(200)).await;

    (base_url, secret, shutdown)
}

/// Open a streaming completion through the gateway, read the first chunk to
/// prove the upstream stream is established, and return the pending response.
async fn open_streaming_completion(base_url: &str, secret: &str) -> reqwest::Response {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/v1/chat/completions", base_url))
        .bearer_auth(secret)
        .json(&serde_json::json!({
            "model": "mockai/test-model",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .expect("request send");
    assert_eq!(
        response.status(),
        200,
        "streaming request should be accepted"
    );
    response
}

#[tokio::test]
async fn test_server_stop_cancels_upstream_stream_during_silence() {
    let disconnected = Arc::new(AtomicBool::new(false));
    let upstream_url = spawn_mock_upstream(disconnected.clone()).await;
    let (base_url, secret, shutdown) = start_test_server(upstream_url).await;

    let mut response = open_streaming_completion(&base_url, &secret).await;

    // Read the first SSE chunk: the pipeline is live end-to-end
    let first = timeout(Duration::from_secs(10), response.chunk())
        .await
        .expect("first chunk within 10s")
        .expect("chunk read")
        .expect("chunk present");
    assert!(
        String::from_utf8_lossy(&first).contains("hello"),
        "first chunk flows through"
    );
    assert!(
        !disconnected.load(Ordering::SeqCst),
        "upstream still connected mid-stream"
    );

    // Stop the server — exactly what the Stop button / stop_server does.
    // The upstream is now SILENT (prefill simulation): cancellation must not
    // depend on another chunk arriving.
    let stop_at = Instant::now();
    shutdown.cancel();

    // The upstream connection must close promptly
    let deadline = Instant::now() + Duration::from_secs(3);
    while !disconnected.load(Ordering::SeqCst) && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        disconnected.load(Ordering::SeqCst),
        "upstream provider connection was not closed within 3s of server stop \
         — a local model (e.g. Ollama) would keep generating"
    );
    println!(
        "upstream cancelled {}ms after stop",
        stop_at.elapsed().as_millis()
    );
}

#[tokio::test]
async fn test_client_disconnect_cancels_upstream_stream_during_silence() {
    let disconnected = Arc::new(AtomicBool::new(false));
    let upstream_url = spawn_mock_upstream(disconnected.clone()).await;
    let (base_url, secret, _shutdown) = start_test_server(upstream_url).await;

    let mut response = open_streaming_completion(&base_url, &secret).await;
    let _ = timeout(Duration::from_secs(10), response.chunk())
        .await
        .expect("first chunk within 10s");

    // Client walks away mid-stream while the upstream is silent
    drop(response);

    let deadline = Instant::now() + Duration::from_secs(3);
    while !disconnected.load(Ordering::SeqCst) && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        disconnected.load(Ordering::SeqCst),
        "upstream provider connection was not closed within 3s of client disconnect"
    );
}
