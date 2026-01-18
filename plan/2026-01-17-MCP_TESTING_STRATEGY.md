# MCP Testing Strategy - Comprehensive Integration & Unit Tests

## Overview

Create a thorough testing suite for the MCP (Model Context Protocol) implementation, mirroring the existing provider testing patterns. The tests will cover all three transport types (STDIO, SSE, WebSocket), OAuth authentication (both client and server), proxy routing, lifecycle management, and edge cases.

**Testing Philosophy**: Follow the established patterns from `provider_tests/` with mock servers, reusable builders, comprehensive scenario coverage, and isolation.

## Testing Architecture

### Test Organization

```
src-tauri/tests/
├── mcp_integration_tests.rs          # Main entry point (re-exports all MCP tests)
└── mcp_tests/
    ├── mod.rs                         # Module documentation
    ├── common.rs                      # Mock builders & shared utilities
    ├── request_validation.rs          # JSON-RPC request assertion helpers
    ├── stdio_transport_tests.rs       # STDIO process management tests
    ├── sse_transport_tests.rs         # SSE transport tests
    ├── websocket_transport_tests.rs   # WebSocket transport tests
    ├── oauth_client_tests.rs          # OAuth client authentication tests
    ├── oauth_server_tests.rs          # MCP server OAuth discovery/tokens tests
    ├── proxy_integration_tests.rs     # End-to-end proxy flow tests
    ├── manager_lifecycle_tests.rs     # McpServerManager lifecycle tests
    ├── concurrent_requests_tests.rs   # Concurrent request handling
    ├── error_scenarios_tests.rs       # HTTP errors, timeouts, failures
    └── health_check_tests.rs          # Health monitoring tests
```

## Phase 1: Mock Server Infrastructure

### 1.1 Mock MCP Server Builders (in `common.rs`)

Create reusable mock server builders following the pattern from `provider_tests/common.rs`:

#### **StdioMockBuilder**
- **Purpose**: Mock MCP server process that communicates via stdin/stdout
- **Implementation**:
  - NOT a wiremock HTTP server - instead a real subprocess that reads stdin/writes stdout
  - Use a simple Python/Node.js script bundled in test resources
  - Script reads JSON-RPC requests from stdin, writes JSON-RPC responses to stdout
  - Builder configures canned responses for specific methods
- **Methods**:
  ```rust
  pub struct StdioMockBuilder {
      script_path: PathBuf,
      responses: HashMap<String, Value>,
  }

  impl StdioMockBuilder {
      pub fn new() -> Self { ... }
      pub fn mock_method(self, method: &str, result: Value) -> Self { ... }
      pub fn mock_error(self, method: &str, error_code: i32, message: &str) -> Self { ... }
      pub fn build(self) -> StdioMockConfig { ... }
  }
  ```

#### **SseMockBuilder**
- **Purpose**: HTTP server that accepts JSON-RPC requests and returns responses
- **Implementation**: Use `wiremock::MockServer` with SSE-style responses
- **Methods**:
  ```rust
  pub struct SseMockBuilder {
      server: MockServer,
  }

  impl SseMockBuilder {
      pub async fn new() -> Self { ... }
      pub fn base_url(&self) -> String { ... }
      pub async fn mock_method(self, method: &str, result: Value) -> Self { ... }
      pub async fn mock_streaming_response(self, method: &str, chunks: Vec<Value>) -> Self { ... }
      pub async fn mock_error(self, method: &str, error_code: i32) -> Self { ... }
  }
  ```

#### **WebSocketMockBuilder**
- **Purpose**: WebSocket server for bidirectional JSON-RPC
- **Implementation**:
  - Use `tokio-tungstenite` to create a test WebSocket server
  - Accept connections on localhost:random_port
  - Echo JSON-RPC requests with canned responses
- **Methods**:
  ```rust
  pub struct WebSocketMockBuilder {
      server_addr: String,
      handle: JoinHandle<()>,
  }

  impl WebSocketMockBuilder {
      pub async fn new() -> Self { ... }
      pub fn server_url(&self) -> String { ... }
      pub async fn mock_method(self, method: &str, result: Value) -> Self { ... }
      pub async fn shutdown(self) { ... }
  }
  ```

#### **OAuthServerMockBuilder**
- **Purpose**: Mock OAuth server for discovery and token endpoints
- **Implementation**: Use `wiremock::MockServer`
- **Methods**:
  ```rust
  pub struct OAuthServerMockBuilder {
      server: MockServer,
  }

  impl OAuthServerMockBuilder {
      pub async fn new() -> Self { ... }
      pub fn base_url(&self) -> String { ... }
      pub async fn mock_discovery(self, auth_url: &str, token_url: &str) -> Self { ... }
      pub async fn mock_token_endpoint(self, access_token: &str, expires_in: i64) -> Self { ... }
      pub async fn mock_token_refresh(self, new_token: &str) -> Self { ... }
      pub async fn mock_discovery_404(self) -> Self { ... }
      pub async fn mock_token_failure(self) -> Self { ... }
  }
  ```

### 1.2 Standard Request Builders

Create standard JSON-RPC request helpers:

```rust
pub fn standard_jsonrpc_request(method: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params: Some(json!({})),
    }
}

pub fn notification_request(method: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: None,  // Notifications have no ID
        method: method.to_string(),
        params: Some(json!({})),
    }
}

pub fn request_with_params(method: &str, params: Value) -> JsonRpcRequest { ... }
```

### 1.3 Assertion Helpers

```rust
pub fn assert_valid_jsonrpc_response(response: &JsonRpcResponse) {
    assert_eq!(response.jsonrpc, "2.0");
    assert!(response.result.is_some() || response.error.is_some());
}

pub fn assert_jsonrpc_result(response: &JsonRpcResponse, expected: &Value) {
    assert!(response.error.is_none(), "Expected success, got error: {:?}", response.error);
    assert_eq!(response.result.as_ref().unwrap(), expected);
}

pub fn assert_jsonrpc_error(response: &JsonRpcResponse, expected_code: i32) {
    assert!(response.result.is_none());
    let error = response.error.as_ref().unwrap();
    assert_eq!(error.code, expected_code);
}
```

## Phase 2: Transport Layer Tests

### 2.1 STDIO Transport Tests (`stdio_transport_tests.rs`)

**Test Coverage**:

1. **Basic Request/Response**
   ```rust
   #[tokio::test]
   async fn test_stdio_single_request() {
       let mock = StdioMockBuilder::new()
           .mock_method("tools/list", json!({"tools": []}))
           .build();

       let transport = StdioTransport::spawn(
           mock.command,
           mock.args,
           HashMap::new()
       ).await.unwrap();

       let request = standard_jsonrpc_request("tools/list");
       let response = transport.send_request(request).await.unwrap();

       assert_valid_jsonrpc_response(&response);
       assert_jsonrpc_result(&response, &json!({"tools": []}));
   }
   ```

2. **Concurrent Requests**
   ```rust
   #[tokio::test]
   async fn test_stdio_concurrent_requests() {
       // Send 10 concurrent requests with different IDs
       // Verify all responses received with correct correlation
   }
   ```

3. **Request Timeout**
   ```rust
   #[tokio::test]
   async fn test_stdio_request_timeout() {
       // Mock server that never responds
       // Verify 30-second timeout triggers
   }
   ```

4. **Process Crash Handling**
   ```rust
   #[tokio::test]
   async fn test_stdio_process_crash() {
       // Start transport
       // Kill process externally
       // Verify pending requests fail gracefully
       // Verify is_alive() returns false
   }
   ```

5. **Invalid JSON Response**
   ```rust
   #[tokio::test]
   async fn test_stdio_invalid_json() {
       // Mock returns malformed JSON
       // Verify error propagation
   }
   ```

6. **Process Cleanup**
   ```rust
   #[tokio::test]
   async fn test_stdio_cleanup() {
       // Start transport, drop it
       // Verify process terminated
   }
   ```

### 2.2 SSE Transport Tests (`sse_transport_tests.rs`)

**Test Coverage**:

1. **Basic Request/Response**
   ```rust
   #[tokio::test]
   async fn test_sse_single_request() {
       let mock = SseMockBuilder::new()
           .await
           .mock_method("tools/list", json!({"tools": []}))
           .await;

       let transport = SseTransport::connect(
           mock.base_url(),
           HashMap::new()
       ).await.unwrap();

       let request = standard_jsonrpc_request("tools/list");
       let response = transport.send_request(request).await.unwrap();

       assert_jsonrpc_result(&response, &json!({"tools": []}));
   }
   ```

2. **Custom Headers**
   ```rust
   #[tokio::test]
   async fn test_sse_custom_headers() {
       // Verify headers passed correctly
   }
   ```

3. **HTTP Error Responses**
   ```rust
   #[tokio::test]
   async fn test_sse_404_error() {
       // Mock returns 404
       // Verify error handling
   }
   ```

4. **Connection Timeout**
   ```rust
   #[tokio::test]
   async fn test_sse_connection_timeout() {
       // Mock server that doesn't respond
       // Verify 30-second timeout
   }
   ```

### 2.3 WebSocket Transport Tests (`websocket_transport_tests.rs`)

**Test Coverage**:

1. **Connection & Basic Request**
   ```rust
   #[tokio::test]
   async fn test_websocket_connection_and_request() {
       let mock = WebSocketMockBuilder::new().await;

       let transport = WebSocketTransport::connect(
           mock.server_url(),
           HashMap::new()
       ).await.unwrap();

       let request = standard_jsonrpc_request("tools/list");
       let response = transport.send_request(request).await.unwrap();

       assert_valid_jsonrpc_response(&response);
   }
   ```

2. **Concurrent Requests**
   ```rust
   #[tokio::test]
   async fn test_websocket_concurrent_requests() {
       // Send multiple requests concurrently
       // Verify correct response correlation by ID
   }
   ```

3. **Connection Failure**
   ```rust
   #[tokio::test]
   async fn test_websocket_connection_refused() {
       // Try connecting to invalid address
       // Verify error
   }
   ```

4. **Server Disconnect Mid-Request**
   ```rust
   #[tokio::test]
   async fn test_websocket_disconnect_during_request() {
       // Send request
       // Close server while waiting
       // Verify pending requests fail gracefully
   }
   ```

5. **Ping/Pong Handling**
   ```rust
   #[tokio::test]
   async fn test_websocket_ping_pong() {
       // Verify transport responds to pings
   }
   ```

## Phase 3: OAuth Authentication Tests

### 3.1 OAuth Client Tests (`oauth_client_tests.rs`)

**Test Coverage**:

1. **Client Creation & Keychain Storage**
   ```rust
   #[tokio::test]
   async fn test_oauth_client_creation() {
       let mock_keychain = Arc::new(MockKeychain::new());
       let manager = OAuthClientManager::with_keychain(vec![], mock_keychain.clone());

       let (client_id, client_secret, config) = manager.create_client(Some("test-client".to_string())).await.unwrap();

       // Verify client_id format (lr-...)
       assert!(client_id.starts_with("lr-"));

       // Verify secret in keychain
       let stored_secret = mock_keychain.get("LocalRouter-OAuthClients", &config.id).unwrap();
       assert_eq!(stored_secret, Some(client_secret));
   }
   ```

2. **Client Verification**
   ```rust
   #[tokio::test]
   async fn test_oauth_client_verify_credentials() {
       // Create client
       // Verify credentials with correct client_id + client_secret
       // Verify rejection with wrong credentials
   }
   ```

3. **Server Linking**
   ```rust
   #[tokio::test]
   async fn test_oauth_client_server_linking() {
       // Create client
       // Link to server
       // Verify linked_server_ids updated
       // Unlink
       // Verify removed
   }
   ```

4. **Access Control Check**
   ```rust
   #[tokio::test]
   async fn test_oauth_client_can_access_server() {
       // Create client
       // Link to server A
       // Verify can_access_server(A) == true
       // Verify can_access_server(B) == false
   }
   ```

### 3.2 MCP Server OAuth Tests (`oauth_server_tests.rs`)

**Test Coverage**:

1. **OAuth Discovery Success**
   ```rust
   #[tokio::test]
   async fn test_oauth_discovery_success() {
       let oauth_mock = OAuthServerMockBuilder::new()
           .await
           .mock_discovery(
               "http://localhost:9999/auth",
               "http://localhost:9999/token"
           )
           .await;

       let manager = McpOAuthManager::new();
       let discovery = manager.discover_oauth(&oauth_mock.base_url()).await.unwrap();

       assert!(discovery.is_some());
       let disco = discovery.unwrap();
       assert_eq!(disco.token_endpoint, "http://localhost:9999/token");
   }
   ```

2. **OAuth Discovery Not Found**
   ```rust
   #[tokio::test]
   async fn test_oauth_discovery_not_found() {
       let oauth_mock = OAuthServerMockBuilder::new()
           .await
           .mock_discovery_404()
           .await;

       let manager = McpOAuthManager::new();
       let discovery = manager.discover_oauth(&oauth_mock.base_url()).await.unwrap();

       assert!(discovery.is_none());
   }
   ```

3. **Token Acquisition**
   ```rust
   #[tokio::test]
   async fn test_oauth_acquire_token() {
       let oauth_mock = OAuthServerMockBuilder::new()
           .await
           .mock_token_endpoint("test-token-abc", 3600)
           .await;

       let mock_keychain = Arc::new(MockKeychain::new());
       // Store client_secret in keychain
       mock_keychain.store("LocalRouter-McpServerTokens", "server1_client_secret", "secret123").unwrap();

       let manager = McpOAuthManager::with_keychain(mock_keychain.clone());

       let oauth_config = McpOAuthConfig {
           auth_url: "".to_string(),
           token_url: oauth_mock.base_url() + "/token",
           scopes: vec!["read".to_string()],
           client_id: "client1".to_string(),
           client_secret_ref: "".to_string(),
       };

       let token = manager.acquire_token("server1", &oauth_config).await.unwrap();

       assert_eq!(token, "test-token-abc");

       // Verify cached in memory
       let cached = manager.get_cached_token("server1").await;
       assert_eq!(cached, Some("test-token-abc".to_string()));
   }
   ```

4. **Token Caching**
   ```rust
   #[tokio::test]
   async fn test_oauth_token_caching() {
       // Acquire token
       // Call acquire_token again without mock response
       // Verify cached token returned (no second HTTP call)
   }
   ```

5. **Token Expiration & Refresh**
   ```rust
   #[tokio::test]
   async fn test_oauth_token_refresh() {
       // Acquire token with short expiration
       // Mock refresh endpoint
       // Wait for expiration
       // Request token again
       // Verify refresh_token flow used
   }
   ```

6. **Token Refresh Failure**
   ```rust
   #[tokio::test]
   async fn test_oauth_token_refresh_failure() {
       // Mock refresh endpoint to fail
       // Verify fallback to re-authentication
   }
   ```

## Phase 4: Proxy Integration Tests

### 4.1 End-to-End Proxy Tests (`proxy_integration_tests.rs`)

**Test Coverage**:

1. **Full STDIO Proxy Flow**
   ```rust
   #[tokio::test]
   async fn test_stdio_proxy_end_to_end() {
       // 1. Create OAuth client
       // 2. Create STDIO MCP server with mock
       // 3. Link client to server
       // 4. Start MCP server
       // 5. Make HTTP request to /mcp/{client_id}/{server_id}
       //    with Authorization: Basic header
       // 6. Verify JSON-RPC request proxied to STDIO
       // 7. Verify JSON-RPC response returned
   }
   ```

2. **Full SSE Proxy Flow**
   ```rust
   #[tokio::test]
   async fn test_sse_proxy_end_to_end() {
       // Same as STDIO but with SSE transport
   }
   ```

3. **Full WebSocket Proxy Flow**
   ```rust
   #[tokio::test]
   async fn test_websocket_proxy_end_to_end() {
       // Same as STDIO but with WebSocket transport
   }
   ```

4. **Proxy with MCP Server OAuth**
   ```rust
   #[tokio::test]
   async fn test_proxy_with_mcp_oauth() {
       // 1. Create OAuth mock server
       // 2. Create MCP server that points to OAuth server for discovery
       // 3. Make proxy request
       // 4. Verify OAuth discovery happens automatically
       // 5. Verify token acquired
       // 6. Verify Authorization: Bearer header added to MCP request
   }
   ```

5. **Auto-Start Server**
   ```rust
   #[tokio::test]
   async fn test_proxy_auto_start_server() {
       // Create server but don't start it
       // Make proxy request
       // Verify server started automatically
       // Verify request succeeds
   }
   ```

6. **Unauthorized Access - Invalid Credentials**
   ```rust
   #[tokio::test]
   async fn test_proxy_unauthorized_invalid_credentials() {
       // Create OAuth client
       // Make request with wrong client_id or client_secret
       // Verify 401 Unauthorized
   }
   ```

7. **Forbidden Access - Client ID Mismatch**
   ```rust
   #[tokio::test]
   async fn test_proxy_forbidden_client_id_mismatch() {
       // Create client A
       // Make request to /mcp/{client_B_id}/... with client A credentials
       // Verify 403 Forbidden
   }
   ```

8. **Forbidden Access - Unlinked Server**
   ```rust
   #[tokio::test]
   async fn test_proxy_forbidden_unlinked_server() {
       // Create client
       // Create server but don't link
       // Make proxy request
       // Verify 403 Forbidden
   }
   ```

9. **Bad Gateway - Server Start Failure**
   ```rust
   #[tokio::test]
   async fn test_proxy_bad_gateway_start_failure() {
       // Create server with invalid config (bad command for STDIO)
       // Make proxy request
       // Verify 502 Bad Gateway
   }
   ```

10. **Bad Gateway - Transport Send Failure**
    ```rust
    #[tokio::test]
    async fn test_proxy_bad_gateway_transport_failure() {
        // Start server
        // Kill server process/connection
        // Make proxy request
        // Verify 502 Bad Gateway
    }
    ```

## Phase 5: Manager Lifecycle Tests

### 5.1 Manager Tests (`manager_lifecycle_tests.rs`)

**Test Coverage**:

1. **Load Configs**
   ```rust
   #[tokio::test]
   async fn test_manager_load_configs() {
       let manager = McpServerManager::new();
       let configs = vec![
           McpServerConfig::new(...),
           McpServerConfig::new(...),
       ];

       manager.load_configs(configs);

       assert_eq!(manager.list_configs().len(), 2);
   }
   ```

2. **Start Server - All Transports**
   ```rust
   #[tokio::test]
   async fn test_manager_start_stdio_server() {
       // Add STDIO config
       // Call start_server
       // Verify server running
       // Verify is_running() == true
   }

   // Repeat for SSE and WebSocket
   ```

3. **Stop Server - All Transports**
   ```rust
   #[tokio::test]
   async fn test_manager_stop_stdio_server() {
       // Start server
       // Call stop_server
       // Verify is_running() == false
       // Verify process terminated
   }
   ```

4. **Send Request Routing**
   ```rust
   #[tokio::test]
   async fn test_manager_send_request_routes_correctly() {
       // Start STDIO server
       // Start SSE server
       // Send request to each
       // Verify routed to correct transport
   }
   ```

5. **Health Check - All States**
   ```rust
   #[tokio::test]
   async fn test_manager_health_check_running() {
       // Start server
       // Get health
       // Verify status == Healthy
   }

   #[tokio::test]
   async fn test_manager_health_check_stopped() {
       // Don't start server
       // Get health
       // Verify status == Unhealthy, error == "Not started"
   }

   #[tokio::test]
   async fn test_manager_health_check_crashed() {
       // Start STDIO server
       // Kill process
       // Get health
       // Verify status == Unhealthy, error contains "Process not running"
   }
   ```

6. **Shutdown All**
   ```rust
   #[tokio::test]
   async fn test_manager_shutdown_all() {
       // Start multiple servers
       // Call shutdown_all
       // Verify all stopped
   }
   ```

## Phase 6: Error & Edge Case Tests

### 6.1 Error Scenarios (`error_scenarios_tests.rs`)

**Test Coverage**:

1. **Timeout Scenarios**
   ```rust
   #[tokio::test]
   async fn test_stdio_request_timeout_30_seconds() {
       // Mock server that hangs
       // Verify timeout after 30 seconds
   }
   ```

2. **Malformed JSON-RPC**
   ```rust
   #[tokio::test]
   async fn test_malformed_jsonrpc_request() {
       // Send request with missing "jsonrpc" field
       // Verify error
   }

   #[tokio::test]
   async fn test_malformed_jsonrpc_response() {
       // Mock returns invalid JSON-RPC
       // Verify parsing error
   }
   ```

3. **JSON-RPC Error Codes**
   ```rust
   #[tokio::test]
   async fn test_jsonrpc_parse_error() {
       // Server returns -32700 (Parse error)
       // Verify error propagated
   }

   #[tokio::test]
   async fn test_jsonrpc_method_not_found() {
       // Server returns -32601 (Method not found)
       // Verify error propagated
   }
   ```

4. **Network Errors**
   ```rust
   #[tokio::test]
   async fn test_sse_connection_refused() {
       // Try connecting to non-existent SSE server
       // Verify connection error
   }
   ```

5. **Disabled Server**
   ```rust
   #[tokio::test]
   async fn test_start_disabled_server() {
       // Create server config with enabled = false
       // Try to start
       // Verify error
   }
   ```

### 6.2 Concurrent Request Tests (`concurrent_requests_tests.rs`)

**Test Coverage**:

1. **High Concurrency**
   ```rust
   #[tokio::test]
   async fn test_100_concurrent_requests_stdio() {
       // Start STDIO server
       // Send 100 concurrent requests
       // Verify all succeed with correct response correlation
   }
   ```

2. **Request ID Collision Avoidance**
   ```rust
   #[tokio::test]
   async fn test_request_id_uniqueness() {
       // Send rapid-fire requests
       // Verify no ID collisions
   }
   ```

3. **Race Condition Tests**
   ```rust
   #[tokio::test]
   async fn test_concurrent_start_stop() {
       // Spawn tasks to start/stop server concurrently
       // Verify no panics
       // Verify final state is consistent
   }
   ```

## Phase 7: Health Check Tests

### 7.1 Health Monitoring (`health_check_tests.rs`)

**Test Coverage**:

1. **Health Check - All Transports**
   ```rust
   #[tokio::test]
   async fn test_health_stdio_running() {
       // Verify is_alive() for running STDIO process
   }

   #[tokio::test]
   async fn test_health_sse_connected() {
       // Verify SSE transport reports healthy
   }

   #[tokio::test]
   async fn test_health_websocket_connected() {
       // Verify WebSocket transport reports healthy
   }
   ```

2. **Health Check - Failures**
   ```rust
   #[tokio::test]
   async fn test_health_stdio_process_died() {
       // Start server, kill process
       // Verify unhealthy
   }
   ```

3. **Health Check Aggregation**
   ```rust
   #[tokio::test]
   async fn test_get_all_health() {
       // Start multiple servers
       // Call get_all_health()
       // Verify all servers reported
   }
   ```

## Test Execution & Verification

### Running Tests

```bash
# Run all MCP integration tests
cargo test --test mcp_integration_tests

# Run specific test modules
cargo test --test mcp_integration_tests stdio_transport
cargo test --test mcp_integration_tests oauth_client
cargo test --test mcp_integration_tests proxy_integration

# Run with output
cargo test --test mcp_integration_tests -- --nocapture

# Run tests serially to avoid port conflicts
cargo test --test mcp_integration_tests -- --test-threads=1
```

### Test Coverage Metrics

**Goal**: Achieve >90% code coverage for MCP modules

**Critical paths to cover**:
- ✅ All three transport types (STDIO, SSE, WebSocket)
- ✅ OAuth client authentication (create, verify, link)
- ✅ MCP server OAuth (discovery, token acquisition, caching, refresh)
- ✅ Proxy routing (all transports, auth checks, error handling)
- ✅ Manager lifecycle (start, stop, health, shutdown)
- ✅ Concurrent request handling
- ✅ Error scenarios (timeouts, failures, malformed data)

### Success Criteria

**For each test category**:
1. All tests pass consistently
2. No flaky tests (run 10 times to verify)
3. Tests execute in reasonable time (<5 minutes total)
4. Mock servers properly isolated (no port conflicts)
5. Proper cleanup (no lingering processes/connections)

## Critical Files to Modify/Create

### New Test Files (to create)

1. `src-tauri/tests/mcp_integration_tests.rs` - Main entry point
2. `src-tauri/tests/mcp_tests/mod.rs` - Module documentation
3. `src-tauri/tests/mcp_tests/common.rs` - Mock builders (~600 lines)
4. `src-tauri/tests/mcp_tests/request_validation.rs` - Assertion helpers (~200 lines)
5. `src-tauri/tests/mcp_tests/stdio_transport_tests.rs` - STDIO tests (~400 lines)
6. `src-tauri/tests/mcp_tests/sse_transport_tests.rs` - SSE tests (~300 lines)
7. `src-tauri/tests/mcp_tests/websocket_transport_tests.rs` - WebSocket tests (~350 lines)
8. `src-tauri/tests/mcp_tests/oauth_client_tests.rs` - OAuth client tests (~400 lines)
9. `src-tauri/tests/mcp_tests/oauth_server_tests.rs` - MCP OAuth tests (~450 lines)
10. `src-tauri/tests/mcp_tests/proxy_integration_tests.rs` - Proxy tests (~600 lines)
11. `src-tauri/tests/mcp_tests/manager_lifecycle_tests.rs` - Manager tests (~350 lines)
12. `src-tauri/tests/mcp_tests/concurrent_requests_tests.rs` - Concurrency tests (~250 lines)
13. `src-tauri/tests/mcp_tests/error_scenarios_tests.rs` - Error tests (~400 lines)
14. `src-tauri/tests/mcp_tests/health_check_tests.rs` - Health tests (~200 lines)

### Test Resources (to create)

15. `src-tauri/tests/resources/mock_mcp_server.py` - Python STDIO mock server
16. `src-tauri/tests/resources/mock_mcp_server.js` - Node.js STDIO mock server (alternative)

### Dependencies to Add

```toml
[dev-dependencies]
# Already present:
# tempfile = "3.8"
# wiremock = "0.6"

# May need to add:
tokio-tungstenite = "0.21"  # For WebSocket mock server
```

## Implementation Order

### Week 1: Mock Infrastructure
- Day 1-2: Create `common.rs` with mock builders
- Day 3: Create `request_validation.rs` helpers
- Day 4-5: Create mock STDIO server scripts (Python/Node.js)

### Week 2: Transport Tests
- Day 1-2: STDIO transport tests
- Day 3: SSE transport tests
- Day 4-5: WebSocket transport tests

### Week 3: OAuth & Proxy Tests
- Day 1-2: OAuth client tests
- Day 3: MCP server OAuth tests
- Day 4-5: Proxy integration tests (all transports)

### Week 4: Manager, Error, & Edge Cases
- Day 1-2: Manager lifecycle tests
- Day 3: Error scenario tests
- Day 4: Concurrent request tests
- Day 5: Health check tests, polish, documentation

## Notes

- **Isolation**: Use `TempDir` for config files, mock keychains for secrets, random ports for servers
- **Serial execution**: Some tests may need `#[serial]` attribute to avoid port conflicts
- **Cleanup**: Ensure all mock servers shut down, all STDIO processes terminated
- **Deterministic**: No sleeps, use proper synchronization (channels, barriers)
- **Comprehensive**: Test happy path + all error branches
- **Documentation**: Each test should have a doc comment explaining what it validates
