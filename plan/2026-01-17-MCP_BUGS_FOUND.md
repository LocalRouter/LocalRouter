# MCP Implementation Bugs Found Through Testing

This document details critical bugs discovered in the MCP (Model Context Protocol) implementation through comprehensive integration testing.

## Summary

**Total Bugs Found: 3**
- **Severity: HIGH** - 2 bugs
- **Severity: MEDIUM** - 1 bug

All bugs were discovered by the integration tests created in `src-tauri/tests/mcp_tests/`.

---

## Bug #1: Duplicate Request ID Race Condition (HIGH SEVERITY)

### Location
- `src-tauri/src/mcp/transport/stdio.rs:230-242`
- `src-tauri/src/mcp/transport/websocket.rs:204-216`

### Description
The STDIO and WebSocket transports allow duplicate request IDs, causing a race condition where:
1. Multiple concurrent requests can share the same ID
2. The pending requests HashMap overwrites previous entries
3. Only the last request with a given ID receives a response
4. Earlier requests fail with "Response channel closed for ID: X"

### Root Cause
```rust
// Line 230-242 in stdio.rs (similar in websocket.rs)
let request_id = if request.id.is_none() {
    let id = self.next_request_id();
    request.id = Some(Value::Number(id.into()));
    id.to_string()
} else {
    request.id.as_ref().unwrap().to_string()  // ❌ No duplicate check!
};

self.pending.write().insert(request_id.clone(), tx);  // ❌ Overwrites existing!
```

When a request already has an ID, the code uses it without checking if it's already pending. The `HashMap::insert()` call overwrites any existing pending request with the same ID, dropping its `oneshot::Sender` and causing the original request's receiver to fail.

### Impact
- **Data Loss**: Responses are routed to the wrong request
- **Silent Failures**: Requests fail with cryptic "channel closed" errors
- **Race Conditions**: Non-deterministic behavior with concurrent requests
- **Security**: Could be exploited to intercept responses intended for other requests

### Reproduction
```rust
// From test: test_stdio_concurrent_requests
let mock = StdioMockBuilder::new()
    .mock_method("method1", json!({"result": 1}))
    .build();

let transport = Arc::new(StdioTransport::spawn(...).await.unwrap());

// All three requests have ID=1 (from standard_jsonrpc_request)
let (resp1, resp2, resp3) = tokio::join!(
    transport.send_request(standard_jsonrpc_request("method1")),
    transport.send_request(standard_jsonrpc_request("method1")),
    transport.send_request(standard_jsonrpc_request("method1")),
);

// ❌ Only resp3 succeeds, resp1 and resp2 fail with "Response channel closed for ID: 1"
```

### Discovered By
- Test: `test_stdio_concurrent_requests` in `stdio_transport_tests.rs`
- Test: `test_stdio_rapid_fire_requests` in `stdio_transport_tests.rs`
- Test: `test_websocket_concurrent_requests` in `websocket_transport_tests.rs`
- Test: `test_websocket_rapid_requests` in `websocket_transport_tests.rs`

### Recommended Fix
Option 1 (Most Secure): Always generate unique IDs
```rust
let request_id = {
    let id = self.next_request_id();
    request.id = Some(Value::Number(id.into()));
    id.to_string()
};
```

Option 2: Check for duplicates and return error
```rust
let request_id = if request.id.is_none() {
    let id = self.next_request_id();
    request.id = Some(Value::Number(id.into()));
    id.to_string()
} else {
    let id_str = request.id.as_ref().unwrap().to_string();
    if self.pending.read().contains_key(&id_str) {
        return Err(AppError::Mcp(format!("Request ID {} already in use", id_str)));
    }
    id_str
};
```

---

## Bug #2: SSE Transport Doesn't Validate Connection (MEDIUM SEVERITY)

### Location
- `src-tauri/src/mcp/transport/sse.rs:53-74`

### Description
The `SseTransport::connect()` method accepts any URL without validation. It only creates an HTTP client and stores the URL, never attempting a connection to verify the URL is valid or the server is reachable. Invalid URLs are silently accepted and only fail later when `send_request()` is called.

### Root Cause
```rust
pub async fn connect(url: String, headers: HashMap<String, String>) -> AppResult<Self> {
    tracing::info!("Connecting to MCP SSE server: {}", url);

    // Build HTTP client with timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Mcp(format!("Failed to create HTTP client: {}", e)))?;

    let transport = Self {
        url,
        client,
        headers,
        // ...
    };

    tracing::info!("MCP SSE transport connected successfully");  // ❌ No actual connection!

    Ok(transport)  // ❌ Returns success without validating URL
}
```

### Impact
- **Delayed Error Detection**: Errors surface only on first request, not at connection time
- **API Inconsistency**: WebSocket transport validates connections, SSE doesn't
- **Poor UX**: Users get false success from connect(), then unexpected failures later
- **Resource Waste**: Invalid transports remain in memory until first use

### Reproduction
```rust
// From test: test_sse_connection_to_invalid_url
let result = SseTransport::connect("http://localhost:99999".to_string(), HashMap::new()).await;

// ❌ Expected: Err(...)
// ✅ Actual: Ok(transport) - no validation!
assert!(result.is_err(), "Should fail to connect to invalid URL");  // FAILS!
```

### Discovered By
- Test: `test_sse_connection_to_invalid_url` in `sse_transport_tests.rs`

### Recommended Fix
Option 1: Perform a test request during connect()
```rust
pub async fn connect(url: String, headers: HashMap<String, String>) -> AppResult<Self> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Mcp(format!("Failed to create HTTP client: {}", e)))?;

    // Validate connection with a test request (e.g., OPTIONS or GET)
    let test_response = client.get(&url).send().await
        .map_err(|e| AppError::Mcp(format!("Failed to connect to SSE server: {}", e)))?;

    if !test_response.status().is_success() {
        return Err(AppError::Mcp(format!(
            "Server returned error status on connect: {}",
            test_response.status()
        )));
    }

    Ok(Self { url, client, headers, ... })
}
```

Option 2: Document lazy connection behavior
If validation is intentionally skipped for performance, add clear documentation that connection errors are deferred.

---

## Bug #3: JsonRpcResponse Cannot Represent Null Results (HIGH SEVERITY)

### Location
- `src-tauri/src/mcp/protocol.rs:44`

### Description
The `JsonRpcResponse` struct uses `Option<Value>` for the result field with `#[serde(skip_serializing_if = "Option::is_none")]`. This causes serde to deserialize JSON `"result": null` as `result: None` instead of `result: Some(Value::Null)`, making it impossible to distinguish between:
1. A missing result field (malformed response)
2. A null result value (valid per JSON-RPC 2.0 spec)

This violates the JSON-RPC 2.0 specification which explicitly allows null as a valid result value.

### Root Cause
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,  // ❌ Cannot distinguish null from absent

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}
```

When deserializing `{"jsonrpc": "2.0", "id": 1, "result": null}`:
- **Expected**: `result: Some(Value::Null)`
- **Actual**: `result: None` (serde treats JSON null as Rust None)

### Impact
- **Spec Violation**: Cannot represent valid JSON-RPC 2.0 responses
- **Data Loss**: Null results are indistinguishable from missing results
- **MCP Compatibility**: MCP methods that return null cannot be used correctly
- **Validation Failures**: Validation logic incorrectly rejects null results

### Reproduction
```rust
// From test: test_stdio_empty_result
let mock = StdioMockBuilder::new()
    .mock_method("empty", json!(null))  // Method returns null result
    .build();

let transport = StdioTransport::spawn(...).await.unwrap();
let request = standard_jsonrpc_request("empty");
let response = transport.send_request(request).await.unwrap();

// Python mock sends: {"jsonrpc": "2.0", "id": 1, "result": null}
// Deserialized as: JsonRpcResponse { result: None, error: None }

assert_jsonrpc_result(&response, &json!(null));
// ❌ FAILS: assert_valid_jsonrpc_response checks result.is_some() || error.is_some()
//    Both are None, so validation fails!
```

### Discovered By
- Test: `test_stdio_empty_result` in `stdio_transport_tests.rs`
- Debug test: `test_null_result_deserialization` in `debug_null_deserialize.rs`

### JSON-RPC 2.0 Spec Reference
From [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification):
> **result**: This member is REQUIRED on success. The value of this member is determined by the method invoked on the Server.

The spec explicitly allows null as a valid result value for successful responses.

### Recommended Fix
Option 1: Use custom deserializer to preserve null distinction
```rust
use serde::{Deserialize, Deserializer};

fn deserialize_result<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    // Explicitly handle null JSON values as Some(Value::Null)
    Ok(Some(Value::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,

    #[serde(default, deserialize_with = "deserialize_result")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}
```

Option 2: Remove Option wrapper for result/error
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponseContent {
    Success { result: Value },
    Error { error: JsonRpcError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(flatten)]
    pub content: JsonRpcResponseContent,
}
```

Option 3: Use skip_deserializing to avoid the issue
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub result: Option<Value>,
```
Then manually validate that responses have exactly one of result or error.

---

## Test Suite Statistics

### Total Tests: 61
- **Passing**: 53 (87%)
- **Failing**: 6 (10%) - All failures due to the bugs above
- **Ignored**: 2 (3%) - Long-running timeout tests

### Test Files Created
1. `stdio_transport_tests.rs` - 20 tests (17 passing, 3 failing due to Bug #1)
2. `sse_transport_tests.rs` - 8 tests (7 passing, 1 failing due to Bug #2)
3. `websocket_transport_tests.rs` - 9 tests (7 passing, 2 failing due to Bug #1)
4. `oauth_client_tests.rs` - 14 tests (14 passing)
5. `request_validation.rs` - 7 tests (7 passing)
6. Additional placeholder modules for future tests

### Bug Detection Effectiveness
- **Bug #1**: Detected by 4 different test cases across 2 transport types
- **Bug #2**: Detected by 1 targeted connection validation test
- **Bug #3**: Detected by 1 edge case test + 1 debug test

---

## Conclusion

The comprehensive MCP test suite successfully identified **3 critical bugs** in the production code:

1. **Duplicate ID Race Condition** (High) - Causes data loss and response routing errors
2. **Missing Connection Validation** (Medium) - Inconsistent error handling
3. **Null Result Spec Violation** (High) - Cannot represent valid JSON-RPC responses

All bugs have clear reproduction steps, detailed analysis, and recommended fixes. The test suite provides a solid foundation for regression testing.

---

## Bugs Fixed ✅

**All 3 bugs have been successfully fixed!**

### Fix #1: Duplicate Request ID Race Condition (STDIO, WebSocket, SSE)

**Files Modified:**
- `src-tauri/src/mcp/transport/stdio.rs` (lines 229-235)
- `src-tauri/src/mcp/transport/websocket.rs` (lines 203-209)
- `src-tauri/src/mcp/transport/sse.rs` (lines 125-131)

**Solution:** Changed all three transports to always generate unique request IDs, preventing collisions:

```rust
// Before (vulnerable to race conditions):
let request_id = if request.id.is_none() {
    let id = self.next_request_id();
    request.id = Some(Value::Number(id.into()));
    id.to_string()
} else {
    request.id.as_ref().unwrap().to_string()  // ❌ Could duplicate!
};

// After (always unique):
let request_id = {
    let id = self.next_request_id();
    request.id = Some(Value::Number(id.into()));
    id.to_string()
};
```

**Additional Fix:** Discovered and fixed a concurrency bug in STDIO transport where concurrent requests could fail with "Stdin not available". Changed `stdin` field from `Arc<RwLock<Option<ChildStdin>>>` to `Arc<Mutex<Option<ChildStdin>>>` to safely hold the lock across async writes.

**Tests Now Passing:**
- ✅ `test_stdio_concurrent_requests`
- ✅ `test_stdio_rapid_fire_requests`
- ✅ `test_websocket_concurrent_requests`
- ✅ `test_websocket_rapid_requests`
- ✅ `test_stdio_request_id_uniqueness` (updated to verify uniqueness)

### Fix #2: SSE Connection Validation

**Files Modified:**
- `src-tauri/src/mcp/transport/sse.rs` (lines 66-81)
- `src-tauri/tests/mcp_tests/common.rs` (lines 169-174)

**Solution:** Added connection validation using HEAD request before considering connection successful:

```rust
// Added validation in connect():
let mut validation_req = client.head(&url);
for (key, value) in &headers {
    validation_req = validation_req.header(key, value);
}

let validation_response = validation_req.send().await
    .map_err(|e| AppError::Mcp(format!("Failed to connect to SSE server: {}", e)))?;

if !validation_response.status().is_success() {
    return Err(AppError::Mcp(format!(
        "Server returned error status on connect: {}",
        validation_response.status()
    )));
}
```

**Test Infrastructure Fix:** Updated `SseMockBuilder::new()` to mock HEAD requests for connection validation.

**Tests Now Passing:**
- ✅ `test_sse_connection_to_invalid_url`
- ✅ All SSE transport tests (6 tests)

### Fix #3: JsonRpcResponse Null Result Deserialization

**Files Modified:**
- `src-tauri/src/mcp/protocol.rs` (lines 9-20, 55-60)

**Solution:** Added custom deserializer to preserve distinction between `null` result and missing result:

```rust
// Added custom deserializer function:
fn deserialize_result<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    // Deserialize the value directly - this captures null as Value::Null
    Ok(Some(Value::deserialize(deserializer)?))
}

// Updated JsonRpcResponse struct:
#[serde(default, deserialize_with = "deserialize_result")]
#[serde(skip_serializing_if = "Option::is_none")]
pub result: Option<Value>,
```

Now `"result": null` deserializes as `Some(Value::Null)` instead of `None`, allowing proper distinction between absent and null results per JSON-RPC 2.0 spec.

**Tests Now Passing:**
- ✅ `test_stdio_empty_result`
- ✅ `test_null_result_deserialization` (debug test)

---

## Final Test Results

**Test Suite Status:** ✅ ALL TESTS PASSING

```
running 61 tests
test result: ok. 59 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**Tests by Category:**
- STDIO Transport: 20 tests - ✅ All passing
- SSE Transport: 8 tests - ✅ All passing
- WebSocket Transport: 9 tests - ✅ All passing
- OAuth Client: 14 tests - ✅ All passing
- Request Validation: 7 tests - ✅ All passing
- Debug Tests: 2 tests - ✅ All passing
- Ignored Tests: 2 (long-running timeout tests)

**Bug Detection Effectiveness:** 100% - All bugs found were successfully fixed and verified.

---

**Report Generated**: 2026-01-17
**Report Updated**: 2026-01-17 (All bugs fixed)
**Test Suite Version**: Initial comprehensive implementation
**Tests Written**: 61 tests across 8 test modules
**Bugs Found**: 3 (2 High, 1 Medium severity)
**Bugs Fixed**: 3 (100%)
