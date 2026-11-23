//! The STDIO bridge against the real HTTP server: the GET stream the bridge
//! holds after `initialize` registers its gateway session for
//! server→client traffic, delivers notifications and server requests
//! (sampling passthrough from an upstream MCP server, including the
//! upstream cancelling), survives a reconnect without duplicating delivery,
//! and is cleaned up when the client goes away. A legacy SSE connection is
//! cleaned up (connection and gateway session) when its client disconnects.
//!
//! Everything runs on loopback: a local axum upstream, the LocalRouter
//! server and the bridge on in-memory stdio.

use axum::{body::Bytes, extract::State, response::IntoResponse, routing::post, Json};
use chrono::Utc;
use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::types::session_notification_key;
use localrouter::mcp::protocol::JsonRpcNotification;
use localrouter::mcp::{McpServerManager, StdioBridge};
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_config::PermissionState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CLIENT: &str = "bridge-stream";

// ---------------------------------------------------------------------------
// Upstream: an MCP server whose tools ask the client to sample
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Sampler {
    /// Answers the gateway posted for this server's requests, by id.
    answers: Arc<parking_lot::Mutex<HashMap<String, Value>>>,
}

fn frame(message: Value) -> Result<Bytes, std::convert::Infallible> {
    Ok(Bytes::from(format!("data: {message}\n\n")))
}

impl Sampler {
    async fn answer(&self, id: &str) -> Option<Value> {
        for _ in 0..500 {
            if let Some(answer) = self.answers.lock().remove(id) {
                return Some(answer);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        None
    }

    /// `ask` samples and returns the sampled text (or the error message);
    /// `ask_cancel` samples, then cancels its request.
    fn call(&self, id: Value, tool: &str) -> axum::response::Response {
        let request_id = format!("s-{}", uuid::Uuid::new_v4());
        let cancel = tool == "ask_cancel";
        let up = self.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Value>(8);
        tokio::spawn(async move {
            let _ = tx
                .send(json!({"jsonrpc": "2.0", "id": request_id, "method": "sampling/createMessage",
                    "params": {"messages": [{"role": "user", "content": {"type": "text", "text": "hi"}}],
                        "maxTokens": 10}}))
                .await;
            let text = if cancel {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = tx
                    .send(
                        json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                        "params": {"requestId": request_id, "reason": "changed my mind"}}),
                    )
                    .await;
                "cancelled".to_string()
            } else {
                match up.answer(&request_id).await {
                    Some(answer) => answer["result"]["content"]["text"]
                        .as_str()
                        .or_else(|| answer["error"]["message"].as_str())
                        .unwrap_or("?")
                        .to_string(),
                    None => "no answer".to_string(),
                }
            };
            let _ = tx
                .send(json!({"jsonrpc": "2.0", "id": id,
                    "result": {"content": [{"type": "text", "text": text}]}}))
                .await;
        });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|message| (frame(message), rx))
        });
        (
            [("content-type", "text/event-stream")],
            axum::body::Body::from_stream(stream),
        )
            .into_response()
    }
}

async fn upstream_handler(State(up): State<Sampler>, body: Bytes) -> axum::response::Response {
    let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(method) = message["method"].as_str() else {
        if let Some(id) = message["id"].as_str() {
            up.answers.lock().insert(id.to_string(), message.clone());
        }
        return axum::http::StatusCode::ACCEPTED.into_response();
    };
    if method.starts_with("notifications/") {
        return axum::http::StatusCode::ACCEPTED.into_response();
    }
    let id = message["id"].clone();
    let result = match method {
        "initialize" => json!({"protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}}, "serverInfo": {"name": "sampler", "version": "1"}}),
        "tools/list" => json!({"tools": [
            {"name": "ask", "inputSchema": {"type": "object"}},
            {"name": "ask_cancel", "inputSchema": {"type": "object"}}]}),
        "tools/call" => {
            let tool = message["params"]["name"].as_str().unwrap_or_default();
            return up.call(id, tool);
        }
        _ => {
            return Json(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "Method not found"}}))
            .into_response()
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn start_upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route(
            "/mcp",
            post(upstream_handler).get(|| async { axum::http::StatusCode::METHOD_NOT_ALLOWED }),
        )
        .with_state(Sampler::default());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    url
}

// ---------------------------------------------------------------------------
// LocalRouter server
// ---------------------------------------------------------------------------

struct Server {
    url: String,
    token: String,
    state: localrouter::server::state::AppState,
}

async fn start(client: lr_config::Client) -> Server {
    let upstream = start_upstream().await;
    let server = McpServerConfig {
        id: "sampler-id".to_string(),
        name: "Sampler".to_string(),
        transport: McpTransportType::HttpSse,
        transport_config: McpTransportConfig::HttpSse {
            url: upstream,
            headers: HashMap::new(),
        },
        auth_config: None,
        discovered_oauth: None,
        oauth_config: None,
        enabled: true,
        created_at: Utc::now(),
        capability_snapshot: None,
    };
    let config = AppConfig {
        clients: vec![client.clone()],
        strategies: vec![lr_config::Strategy::new("Default".to_string())],
        mcp_servers: vec![server.clone()],
        ..Default::default()
    };
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::env::temp_dir().join(format!("bridge_stream_{}.yaml", uuid::Uuid::new_v4())),
    ));
    let mcp_server_manager = Arc::new(McpServerManager::new());
    mcp_server_manager.add_config(server);
    let metrics_collector = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(
            std::env::temp_dir().join(format!("bridge_stream_{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap(),
    )));
    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let token_store = Arc::new(TokenStore::new());
    let (token, _) = token_store.generate_token(client.id.clone()).unwrap();
    let (state, _handle, port, _shutdown) = localrouter::server::start_server(
        localrouter::server::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            enable_cors: false,
        },
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        Arc::new(ClientManager::new(vec![client])),
        token_store,
        metrics_collector,
        None,
    )
    .await
    .expect("server");
    Server {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token,
        state,
    }
}

fn client(mode: lr_config::LlmMode) -> lr_config::Client {
    let mut client = lr_config::Client::new_with_strategy("Bridge".to_string(), String::new());
    client.id = CLIENT.to_string();
    client.enabled = true;
    client.mcp_permissions.global = PermissionState::Allow;
    client.llm_mode = mode;
    client
}

// ---------------------------------------------------------------------------
// The bridge on in-memory stdio
// ---------------------------------------------------------------------------

struct Bridge {
    stdin: Option<tokio::io::DuplexStream>,
    stdout: tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    task: tokio::task::JoinHandle<()>,
}

impl Bridge {
    fn start(server: &Server) -> Self {
        let (stdin, bridge_in) = tokio::io::duplex(1 << 20);
        let (bridge_out, stdout) = tokio::io::duplex(1 << 20);
        let bridge =
            StdioBridge::with_endpoint(CLIENT.into(), server.token.clone(), server.url.clone())
                .unwrap();
        let task = tokio::spawn(async move {
            let _ = bridge.run_with_io(bridge_in, bridge_out).await;
        });
        Self {
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            task,
        }
    }

    async fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        stdin.flush().await.unwrap();
    }

    /// The next stdout message, or `None` after `timeout`.
    async fn next(&mut self, timeout: Duration) -> Option<Value> {
        let line = tokio::time::timeout(timeout, self.stdout.next_line())
            .await
            .ok()?
            .ok()??;
        Some(serde_json::from_str(&line).expect("stdout carries JSON-RPC only"))
    }

    /// Skip messages until one satisfies `pred`.
    async fn until(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        loop {
            let message = self
                .next(Duration::from_secs(20))
                .await
                .unwrap_or_else(|| panic!("no {what} on stdout"));
            if pred(&message) {
                return message;
            }
        }
    }

    async fn response(&mut self, id: i64) -> Value {
        self.until(&format!("response {id}"), |m| {
            m["id"] == id && m.get("method").is_none()
        })
        .await
    }

    async fn initialize(&mut self, capabilities: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": capabilities,
                "clientInfo": {"name": "bridge-test", "version": "1"}}}))
            .await;
        let init = self.response(1).await;
        assert!(init.get("result").is_some(), "{init}");
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
    }
}

async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..1500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The gateway session of the bridge's client, once there is one.
async fn session_key(server: &Server) -> String {
    let mut key = None;
    wait_until("gateway session", || {
        key = server
            .state
            .mcp_gateway
            .get_sessions_for_client(CLIENT)
            .into_iter()
            .next()
            .map(|(key, _)| key);
        key.is_some()
    })
    .await;
    key.unwrap()
}

fn notify(server: &Server, key: &str, n: u64) {
    let _ = server.state.mcp_notification_broadcast.send((
        session_notification_key(key),
        JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/message".into(),
            params: Some(json!({"level": "info", "data": {"n": n}})),
        },
    ));
}

fn tool_text(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool text in {response}"))
        .to_string()
}

#[tokio::test]
async fn bridge_get_stream_registers_delivers_reconnects_and_cleans_up() {
    let server = start(client(lr_config::LlmMode::Gateway)).await;
    let mut bridge = Bridge::start(&server);
    bridge.initialize(json!({"sampling": {}})).await;

    // Registration: the GET stream is registered under the gateway session
    let key = session_key(&server).await;
    let sse = server.state.sse_connection_manager.clone();
    wait_until("GET stream registered", || sse.has_connection(&key)).await;

    // Server → client notification
    notify(&server, &key, 1);
    let message = bridge
        .until("notification", |m| m["method"] == "notifications/message")
        .await;
    assert_eq!(message["params"]["data"]["n"], 1);

    // Server → client request: the upstream's sampling request reaches the
    // client under a gateway id; the client's answer goes back upstream
    bridge
        .send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "sampler__ask", "arguments": {}}}))
        .await;
    let request = bridge
        .until("sampling request", |m| {
            m["method"] == "sampling/createMessage"
        })
        .await;
    assert!(request["id"]
        .as_str()
        .is_some_and(|id| !id.starts_with("s-")));
    assert_eq!(
        request["params"]["messages"][0]["content"]["text"], "hi",
        "params pass through unchanged"
    );
    bridge
        .send(json!({"jsonrpc": "2.0", "id": request["id"], "result": {
            "role": "assistant", "model": "test-model", "stopReason": "endTurn",
            "content": {"type": "text", "text": "sampled!"}}}))
        .await;
    assert_eq!(tool_text(&bridge.response(2).await), "sampled!");
    wait_until("answered request forgotten", || {
        sse.pending_server_request_count() == 0
    })
    .await;

    // The upstream cancels its request: the client is told, under its id
    bridge
        .send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "sampler__ask_cancel", "arguments": {}}}))
        .await;
    let request = bridge
        .until("sampling request", |m| {
            m["method"] == "sampling/createMessage"
        })
        .await;
    // The cancellation (GET stream) and the tool result (POST body) may
    // arrive in either order
    let (mut cancelled, mut result) = (None, None);
    while cancelled.is_none() || result.is_none() {
        let message = bridge
            .next(Duration::from_secs(20))
            .await
            .expect("cancellation and tool result");
        if message["method"] == "notifications/cancelled" {
            cancelled = Some(message);
        } else if message["id"] == 3 && message.get("method").is_none() {
            result = Some(message);
        }
    }
    assert_eq!(cancelled.unwrap()["params"]["requestId"], request["id"]);
    assert_eq!(tool_text(&result.unwrap()), "cancelled");

    // Reconnection: replace the bridge's registration; its stream ends, the
    // bridge reconnects and takes the registration back
    let (mut stolen, _handle) = sse.register_tracked(&key);
    tokio::time::timeout(Duration::from_secs(20), stolen.recv())
        .await
        .expect("the bridge re-registered its stream");
    assert!(sse.has_connection(&key));
    // Delivery resumes, exactly once (the replaced stream is gone)
    notify(&server, &key, 2);
    let message = bridge
        .until("notification after reconnect", |m| {
            m["method"] == "notifications/message"
        })
        .await;
    assert_eq!(message["params"]["data"]["n"], 2);
    let mut duplicates = 0;
    while let Some(message) = bridge.next(Duration::from_millis(500)).await {
        if message["method"] == "notifications/message" {
            duplicates += 1;
        }
    }
    assert_eq!(duplicates, 0, "one delivery per notification");

    // Cleanup on disconnect: stdin EOF ends the bridge, which deletes the
    // session; its stream registration goes with the stream
    bridge.stdin.take();
    tokio::time::timeout(Duration::from_secs(10), &mut bridge.task)
        .await
        .expect("bridge stopped")
        .unwrap();
    wait_until("stream unregistered", || !sse.has_connection(&key)).await;
    wait_until("gateway session ended", || {
        server.state.mcp_gateway.get_session(&key).is_none()
    })
    .await;
}

/// A client that did not declare sampling is never sent a sampling request.
/// An MCP-only client (no LLM access to fall back on) gets a clear error
/// back at the upstream.
#[tokio::test]
async fn sampling_for_a_client_without_the_capability_is_refused_for_mcp_only() {
    let server = start(client(lr_config::LlmMode::Off)).await;
    let mut bridge = Bridge::start(&server);
    // No `sampling` capability declared
    bridge.initialize(json!({})).await;
    let _ = session_key(&server).await;
    bridge
        .send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "sampler__ask", "arguments": {}}}))
        .await;
    let response = bridge.response(2).await;
    let text = tool_text(&response);
    assert!(text.contains("does not support sampling"), "{text}");
    bridge.stdin.take();
    let _ = tokio::time::timeout(Duration::from_secs(10), &mut bridge.task).await;
}

/// A legacy HTTP+SSE connection owns its gateway session: when the client
/// goes away, the connection is unregistered and the session ends.
#[tokio::test]
async fn legacy_sse_disconnect_unregisters_and_ends_the_session() {
    use futures_util::StreamExt;
    let server = start(client(lr_config::LlmMode::Gateway)).await;
    let http = reqwest::Client::new();
    let response = http
        .get(server.url.trim_end_matches("/mcp").to_string() + "/")
        .bearer_auth(&server.token)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.bytes_stream();
    let mut text = String::new();
    while !text.contains("\n\n") {
        let chunk = tokio::time::timeout(Duration::from_secs(10), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    let session = text
        .split("sessionId=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("endpoint event with a session id")
        .to_string();
    let sse = server.state.sse_connection_manager.clone();
    assert!(sse.has_connection(&session));
    // Open the gateway session on this connection
    let init = http
        .post(format!(
            "{}/?sessionId={session}",
            server.url.trim_end_matches("/mcp")
        ))
        .bearer_auth(&server.token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "legacy", "version": "1"}}}))
        .send()
        .await
        .unwrap();
    assert!(init.status().is_success());
    wait_until("legacy gateway session", || {
        server.state.mcp_gateway.get_session(&session).is_some()
    })
    .await;

    drop(body);
    // The server learns of the disconnect when it next writes to the stream
    for _ in 0..1500 {
        if !sse.has_connection(&session) {
            break;
        }
        notify(&server, &session, 0);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!sse.has_connection(&session), "connection unregistered");
    wait_until("session ended", || {
        server.state.mcp_gateway.get_session(&session).is_none()
    })
    .await;
}
