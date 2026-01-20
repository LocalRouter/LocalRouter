//! Common utilities and test helpers for MCP testing
//!
//! This module provides reusable components for testing MCP functionality:
//! - Mock server builders for different transport types (STDIO, SSE, WebSocket)
//! - Mock OAuth server builders
//! - Standard test request builders

// Re-export MockKeychain from the main crate
pub use localrouter_ai::api_keys::keychain_trait::MockKeychain;

use futures_util::{SinkExt, StreamExt};
use localrouter_ai::mcp::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use localrouter_ai::utils::errors::AppResult;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use wiremock::{
    matchers::{header, method as http_method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

// ==================== STDIO MOCK ====================

/// Configuration for STDIO mock server
pub struct StdioMockConfig {
    /// Python script path
    pub script_path: PathBuf,

    /// Command to run (e.g., "python3")
    pub command: String,

    /// Arguments
    pub args: Vec<String>,

    /// Environment variables
    pub env: HashMap<String, String>,
}

impl StdioMockConfig {
    /// Get a clone of the command
    pub fn get_command(&self) -> String {
        self.command.clone()
    }

    /// Get a clone of the args
    pub fn get_args(&self) -> Vec<String> {
        self.args.clone()
    }

    /// Get a clone of the environment variables
    pub fn get_env(&self) -> HashMap<String, String> {
        self.env.clone()
    }
}

/// Builder for STDIO mock server
///
/// Uses a standalone Python script configured via environment variables.
pub struct StdioMockBuilder {
    /// Canned responses for methods
    responses: HashMap<String, Value>,

    /// Canned errors for methods
    errors: HashMap<String, (i32, String)>,

    /// Whether to delay responses (for timeout testing)
    delay_seconds: Option<u64>,

    /// Whether to hang forever (for timeout testing)
    hang_forever: bool,
}

impl StdioMockBuilder {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            errors: HashMap::new(),
            delay_seconds: None,
            hang_forever: false,
        }
    }

    /// Mock a successful method response
    pub fn mock_method(mut self, method: &str, result: Value) -> Self {
        self.responses.insert(method.to_string(), result);
        self
    }

    /// Mock an error response for a method
    pub fn mock_error(mut self, method: &str, error_code: i32, message: &str) -> Self {
        self.errors
            .insert(method.to_string(), (error_code, message.to_string()));
        self
    }

    /// Add a delay to all responses (for timeout testing)
    pub fn with_delay(mut self, seconds: u64) -> Self {
        self.delay_seconds = Some(seconds);
        self
    }

    /// Make the server hang forever (for timeout testing)
    pub fn hang_forever(mut self) -> Self {
        self.hang_forever = true;
        self
    }

    /// Build the mock configuration
    pub fn build(self) -> StdioMockConfig {
        // Use the standalone Python script
        let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("resources")
            .join("mcp_mock_server.py");

        // Build environment variables for configuration
        let mut env = HashMap::new();

        // Convert responses to JSON
        let responses_json = serde_json::to_string(&self.responses).unwrap();
        env.insert("MCP_MOCK_RESPONSES".to_string(), responses_json);

        // Convert errors to JSON (as map of method -> [code, message])
        let errors_map: HashMap<String, Vec<Value>> = self
            .errors
            .into_iter()
            .map(|(method, (code, message))| (method, vec![json!(code), json!(message)]))
            .collect();
        let errors_json = serde_json::to_string(&errors_map).unwrap();
        env.insert("MCP_MOCK_ERRORS".to_string(), errors_json);

        // Set delay
        if let Some(seconds) = self.delay_seconds {
            env.insert("MCP_MOCK_DELAY".to_string(), seconds.to_string());
        }

        // Set hang forever
        if self.hang_forever {
            env.insert("MCP_MOCK_HANG".to_string(), "1".to_string());
        }

        StdioMockConfig {
            script_path: script_path.clone(),
            command: "python3".to_string(),
            args: vec![script_path.to_string_lossy().to_string()],
            env,
        }
    }
}

// ==================== SSE MOCK SERVER ====================

/// SSE mock server builder
///
/// Creates a wiremock HTTP server that handles SSE-style MCP requests.
pub struct SseMockBuilder {
    server: MockServer,
}

impl SseMockBuilder {
    pub async fn new() -> Self {
        let server = MockServer::start().await;

        // Mock HEAD request for connection validation
        // SSE transport uses HEAD to verify the server is reachable
        Mock::given(http_method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        Self { server }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock a successful method response
    pub async fn mock_method(self, _method: &str, result: Value) -> Self {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: Some(result),
            error: None,
        };

        Mock::given(http_method("POST"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock a streaming response (multiple SSE events)
    pub async fn mock_streaming_response(self, _method: &str, chunks: Vec<Value>) -> Self {
        // Build SSE response
        let mut sse_body = String::new();
        for chunk in chunks {
            sse_body.push_str(&format!(
                "data: {}\n\n",
                serde_json::to_string(&chunk).unwrap()
            ));
        }

        Mock::given(http_method("POST"))
            .and(header("content-type", "application/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&self.server)
            .await;

        self
    }

    /// Mock an error response
    pub async fn mock_error(self, _method: &str, error_code: i32, message: &str) -> Self {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: None,
            error: Some(JsonRpcError {
                code: error_code,
                message: message.to_string(),
                data: None,
            }),
        };

        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock a 404 response
    pub async fn mock_404(self) -> Self {
        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock a timeout (never responds)
    pub async fn mock_timeout(self) -> Self {
        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(60)))
            .mount(&self.server)
            .await;

        self
    }
}

// ==================== WEBSOCKET MOCK SERVER ====================

/// WebSocket mock server
pub struct WebSocketMockServer {
    server_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    responses: Arc<RwLock<HashMap<String, Value>>>,
    errors: Arc<RwLock<HashMap<String, (i32, String)>>>,
}

impl WebSocketMockServer {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_url = format!("ws://{}", addr);

        let responses = Arc::new(RwLock::new(HashMap::new()));
        let errors = Arc::new(RwLock::new(HashMap::new()));

        let responses_clone = responses.clone();
        let errors_clone = errors.clone();

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        if let Ok((stream, _)) = accept_result {
                            let responses = responses_clone.clone();
                            let errors = errors_clone.clone();

                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_connection(stream, responses, errors).await {
                                    tracing::error!("WebSocket connection error: {}", e);
                                }
                            });
                        }
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }
        });

        Self {
            server_url,
            shutdown_tx: Some(shutdown_tx),
            responses,
            errors,
        }
    }

    async fn handle_connection(
        stream: TcpStream,
        responses: Arc<RwLock<HashMap<String, Value>>>,
        errors: Arc<RwLock<HashMap<String, (i32, String)>>>,
    ) -> AppResult<()> {
        let ws_stream = accept_async(stream).await.map_err(|e| {
            localrouter_ai::utils::errors::AppError::Mcp(format!("WebSocket accept failed: {}", e))
        })?;

        let (mut write, mut read) = ws_stream.split();

        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| {
                localrouter_ai::utils::errors::AppError::Mcp(format!("WebSocket read error: {}", e))
            })?;

            if let Message::Text(text) = msg {
                // Parse JSON-RPC request
                if let Ok(request) = serde_json::from_str::<JsonRpcRequest>(&text) {
                    let method = &request.method;
                    let req_id = request.id.clone().unwrap_or(json!(null));

                    // Check for error response
                    let response =
                        if let Some((error_code, error_message)) = errors.read().get(method) {
                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: req_id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: *error_code,
                                    message: error_message.clone(),
                                    data: None,
                                }),
                            }
                        } else if let Some(result) = responses.read().get(method) {
                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: req_id,
                                result: Some(result.clone()),
                                error: None,
                            }
                        } else {
                            // Method not found
                            JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: req_id,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32601,
                                    message: format!("Method not found: {}", method),
                                    data: None,
                                }),
                            }
                        };

                    // Send response
                    let response_text = serde_json::to_string(&response).unwrap();
                    write
                        .send(Message::Text(response_text))
                        .await
                        .map_err(|e| {
                            localrouter_ai::utils::errors::AppError::Mcp(format!(
                                "WebSocket write error: {}",
                                e
                            ))
                        })?;
                }
            } else if let Message::Ping(data) = msg {
                // Respond to ping
                write.send(Message::Pong(data)).await.map_err(|e| {
                    localrouter_ai::utils::errors::AppError::Mcp(format!(
                        "WebSocket pong error: {}",
                        e
                    ))
                })?;
            } else if let Message::Close(_) = msg {
                break;
            }
        }

        Ok(())
    }

    pub fn server_url(&self) -> String {
        self.server_url.clone()
    }

    /// Mock a successful method response
    pub fn mock_method(&self, method: &str, result: Value) {
        self.responses.write().insert(method.to_string(), result);
    }

    /// Mock an error response
    pub fn mock_error(&self, method: &str, error_code: i32, message: &str) {
        self.errors
            .write()
            .insert(method.to_string(), (error_code, message.to_string()));
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// ==================== OAUTH MOCK SERVER ====================

/// OAuth server mock builder
pub struct OAuthServerMockBuilder {
    server: MockServer,
}

impl OAuthServerMockBuilder {
    pub async fn new() -> Self {
        let server = MockServer::start().await;
        Self { server }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock OAuth discovery endpoint
    pub async fn mock_discovery(self, auth_url: &str, token_url: &str) -> Self {
        let discovery_response = json!({
            "authorization_endpoint": auth_url,
            "token_endpoint": token_url,
            "scopes_supported": ["read", "write"],
            "grant_types_supported": ["client_credentials", "refresh_token"]
        });

        Mock::given(http_method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery_response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock discovery returning 404
    pub async fn mock_discovery_404(self) -> Self {
        Mock::given(http_method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock token endpoint
    pub async fn mock_token_endpoint(self, access_token: &str, expires_in: i64) -> Self {
        let token_response = json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": expires_in,
        });

        Mock::given(http_method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock token refresh endpoint
    pub async fn mock_token_refresh(self, new_token: &str, expires_in: i64) -> Self {
        let token_response = json!({
            "access_token": new_token,
            "token_type": "Bearer",
            "expires_in": expires_in,
            "refresh_token": "new_refresh_token"
        });

        Mock::given(http_method("POST"))
            .and(path("/token"))
            .and(query_param("grant_type", "refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock token endpoint failure
    pub async fn mock_token_failure(self) -> Self {
        Mock::given(http_method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "invalid_client",
                "error_description": "Client authentication failed"
            })))
            .mount(&self.server)
            .await;

        self
    }
}

// ==================== STANDARD REQUEST BUILDERS ====================

/// Create a standard JSON-RPC request
pub fn standard_jsonrpc_request(method: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params: Some(json!({})),
    }
}

/// Create a JSON-RPC notification (no id)
pub fn notification_request(method: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: None,
        method: method.to_string(),
        params: Some(json!({})),
    }
}

/// Create a request with custom parameters
pub fn request_with_params(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params: Some(params),
    }
}

/// Create a request with a string ID
pub fn request_with_string_id(id: &str, method: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(id)),
        method: method.to_string(),
        params: Some(json!({})),
    }
}
