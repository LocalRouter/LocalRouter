//! HTTP-transport tests for the MCP 2026-07-28 stateless revision.
//!
//! Exercises the pieces that only exist at the HTTP layer (headers, status
//! codes, MRTR retry validation) against the real server, complementing the
//! gateway-level integration tests in mcp_gateway_mock_integration_tests.rs.

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

fn create_test_client(id: &str, strategy_id: &str) -> Client {
    let mut client = Client::new_with_strategy("Test Client".to_string(), strategy_id.to_string());
    client.id = id.to_string();
    client.enabled = true;
    client
}

/// Start the real HTTP server (no MCP backends configured — the gateway also
/// serves stateless lifecycle methods without any backend running).
async fn start_test_server() -> (String, String) {
    let (base_url, secret, _state) = start_test_server_with_state().await;
    (base_url, secret)
}

async fn start_test_server_with_state() -> (String, String, server::state::AppState) {
    let test_client = create_test_client("test-api-key", "default");
    let strategy = Strategy::new("Default".to_string());

    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![strategy],
        ..Default::default()
    };

    let config_path =
        std::env::temp_dir().join(format!("test_stateless_http_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let mcp_server_manager = Arc::new(McpServerManager::new());
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_stateless_http_{}.db", uuid::Uuid::new_v4()));
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

    let test_port = 43000 + (std::process::id() % 10000) as u16;
    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: test_port,
        enable_cors: true,
    };

    let (state, _handle, actual_port, _shutdown) = server::start_server(
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

    (base_url, secret, state)
}

fn stateless_meta() -> serde_json::Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "http-test", "version": "1.0"}
    })
}

#[tokio::test]
async fn test_server_discover_over_http_with_version_echo() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "server/discover")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": { "_meta": stateless_meta() }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    // The negotiated protocol version is echoed to stateless peers
    assert_eq!(
        response
            .headers()
            .get("mcp-protocol-version")
            .and_then(|v| v.to_str().ok()),
        Some("2026-07-28")
    );

    let body: serde_json::Value = response.json().await.unwrap();
    let result = &body["result"];
    assert!(result["protocolVersions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "2026-07-28"));
    assert_eq!(result["serverInfo"]["name"], "LocalRouter MCP Gateway");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "LocalRouter MCP Gateway"
    );
    assert_eq!(result["resultType"], "complete");
}

#[tokio::test]
async fn test_mcp_method_header_mismatch_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list") // disagrees with the body
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": { "_meta": stateless_meta() }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32020); // HeaderMismatchError
}

#[tokio::test]
async fn test_future_protocol_version_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2031-01-01")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32022); // UnsupportedProtocolVersion
    assert!(body["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "2026-07-28"));
}

#[tokio::test]
async fn test_mrtr_unknown_request_state_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // A stateless retry carrying a requestState we never issued (or that
    // expired) must be rejected, not silently re-executed.
    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "whatever__tool",
                "requestState": "no-such-state",
                "inputResponses": [{"id": "x", "response": {"action": "accept", "content": {}}}],
                "_meta": stateless_meta()
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200); // JSON-RPC error in body
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32602);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requestState"));
}

#[tokio::test]
async fn test_legacy_mcp_post_unchanged() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // A legacy client: no version header, no _meta. initialize must work and
    // the response must carry no stateless fields or version echo header.
    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "legacy", "version": "1.0"}
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(response.headers().get("mcp-protocol-version").is_none());

    let body: serde_json::Value = response.json().await.unwrap();
    let result = &body["result"];
    assert!(result["protocolVersion"].is_string());
    assert!(result.get("resultType").is_none());
}

/// Strict Streamable HTTP clients (Codex's rmcp) abandon the server when a
/// notification or a client response is answered with a JSON body; the
/// transport requires `202 Accepted` with no body.
#[tokio::test]
async fn test_notifications_and_client_responses_get_empty_202() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let init = client
        .post(&base_url)
        .bearer_auth(&secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "codex-mcp-client", "version": "1.0"}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), 200);

    for body in [
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}}),
        // A response to a server-initiated request nobody is waiting for
        json!({"jsonrpc": "2.0", "id": "srv-1", "result": {}}),
    ] {
        let response = client
            .post(&base_url)
            .bearer_auth(&secret)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 202, "status for {body}");
        assert_eq!(response.text().await.unwrap(), "", "body for {body}");
    }

    // A stateless (2026-07-28) peer's notification gets the same answer
    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": 9, "_meta": stateless_meta()}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.text().await.unwrap(), "");
}

/// The path a tool activation takes to a Streamable HTTP client: the
/// client opens its GET notification stream only after `notifications/initialized`
/// is answered with 202, and a `tools/list_changed` for its session (keyed by
/// client id, as POSTs carry no sessionId) must arrive on that stream.
#[tokio::test]
async fn test_list_changed_reaches_streamable_http_client() {
    use futures::StreamExt;

    let (base_url, secret, state) = start_test_server_with_state().await;
    let client = reqwest::Client::new();

    let init = client
        .post(&base_url)
        .bearer_auth(&secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "claude-code", "version": "2.1"}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), 200);
    let session_id = init
        .headers()
        .get("mcp-session-id")
        .expect("initialize issues a session id")
        .to_str()
        .unwrap()
        .to_string();
    let initialized = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("Mcp-Session-Id", &session_id)
        .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(initialized.status(), 202);

    let stream_response = client
        .get(&base_url)
        .bearer_auth(&secret)
        .header("Accept", "text/event-stream")
        .header("Mcp-Session-Id", &session_id)
        .send()
        .await
        .unwrap();
    assert_eq!(stream_response.status(), 200);
    let mut stream = stream_response.bytes_stream();

    let mut seen = String::new();
    async fn read_until<B: AsRef<[u8]>>(
        stream: &mut (impl futures::Stream<Item = reqwest::Result<B>> + Unpin),
        seen: &mut String,
        needle: &str,
    ) {
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            while !seen.contains(needle) {
                let chunk = stream.next().await.expect("stream ended").unwrap();
                seen.push_str(&String::from_utf8_lossy(chunk.as_ref()));
            }
        })
        .await;
        assert!(found.is_ok(), "no {needle:?} on the stream; got: {seen}");
    }

    // The stream is registered once its first event arrives
    read_until(&mut stream, &mut seen, "event: endpoint").await;

    // What the gateway broadcasts when IndexSearch activates a deferred
    // tool: keyed by the gateway session, which is this client's session id.
    // Another session's notification must not arrive here.
    use localrouter::mcp::gateway::types::{session_notification_key, streamable_session_key};
    let notify = |session_key: &str, which: &str| {
        state
            .mcp_notification_broadcast
            .send((
                session_notification_key(session_key),
                localrouter::mcp::protocol::JsonRpcNotification::new(
                    "notifications/tools/list_changed".to_string(),
                    Some(json!({"which": which})),
                ),
            ))
            .unwrap();
    };
    notify(
        &streamable_session_key("internal-test", "other-instance"),
        "other",
    );
    notify(
        &streamable_session_key("internal-test", &session_id),
        "mine",
    );
    read_until(&mut stream, &mut seen, "\"mine\"").await;
    assert!(seen.contains("notifications/tools/list_changed"));
    assert!(!seen.contains("\"other\""), "{seen}");
}

async fn initialize_session(client: &reqwest::Client, base_url: &str, secret: &str) -> String {
    let response = client
        .post(base_url)
        .bearer_auth(secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "claude-code", "version": "2.1"}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response
        .headers()
        .get("mcp-session-id")
        .expect("initialize issues a session id")
        .to_str()
        .unwrap()
        .to_string()
}

/// Two instances of a client sharing one token get separate gateway
/// sessions: one's initialize must not reset the other's (activated tools,
/// indexed responses). DELETE ends a session.
#[tokio::test]
async fn test_streamable_sessions_are_isolated_and_deletable() {
    use localrouter::mcp::gateway::types::streamable_session_key;

    let (base_url, secret, state) = start_test_server_with_state().await;
    let client = reqwest::Client::new();
    let gateway = &state.mcp_gateway;

    let a = initialize_session(&client, &base_url, &secret).await;
    let b = initialize_session(&client, &base_url, &secret).await;
    assert_ne!(a, b);
    let key_a = streamable_session_key("internal-test", &a);
    let key_b = streamable_session_key("internal-test", &b);
    assert!(
        gateway.get_session(&key_a).is_some(),
        "A survives B's initialize"
    );
    assert!(gateway.get_session(&key_b).is_some());

    // Requests carrying the id use that session
    let list = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("Mcp-Session-Id", &a)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    assert!(
        list.headers().get("mcp-session-id").is_none(),
        "only initialize issues ids"
    );

    let delete = |id: Option<&str>| {
        let mut request = client.delete(&base_url).bearer_auth(&secret);
        if let Some(id) = id {
            request = request.header("Mcp-Session-Id", id);
        }
        request.send()
    };
    assert_eq!(delete(Some(&a)).await.unwrap().status(), 204);
    assert!(gateway.get_session(&key_a).is_none());
    assert!(gateway.get_session(&key_b).is_some());
    assert_eq!(delete(Some(&a)).await.unwrap().status(), 404);
    assert_eq!(delete(None).await.unwrap().status(), 400);

    // An id that is not visible ASCII is rejected
    let bad = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("Mcp-Session-Id", "has space")
        .json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn test_subscription_acknowledges_explicit_filter_and_request_id() {
    let (base_url, secret) = start_test_server().await;
    let mut response = reqwest::Client::new().post(&base_url).bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "subscriptions/listen")
        .json(&json!({"jsonrpc":"2.0", "id":42, "method":"subscriptions/listen", "params":{
            "_meta":stateless_meta(), "notifications":{"toolsListChanged":true,"promptsListChanged":false}
        }})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(chunk.to_vec()).unwrap();
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let ack: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(ack["method"], "notifications/subscriptions/acknowledged");
    assert_eq!(
        ack["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        42
    );
    assert_eq!(
        ack["params"]["notifications"],
        json!({"toolsListChanged":true})
    );
    assert!(ack.get("result").is_none());
}
