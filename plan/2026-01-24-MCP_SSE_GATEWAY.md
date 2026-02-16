# MCP SSE Gateway Implementation Plan

## Problem Statement

The unified MCP gateway at `/` and `/mcp` needs to be a proper MCP SSE transport implementation that:
1. Follows the MCP SSE transport specification
2. Actually streams data from backend MCP servers to clients
3. Works correctly with MCP SDK clients (Claude Code, Cursor, etc.)

## Current Architecture Understanding

### How It Works (Correctly)

```
External Client              LocalRouter Gateway              Backend MCP Servers
     │                              │                                │
     │                              │ [McpServerManager manages       │
     │                              │  SseTransport connections]      │
     │                              │         │                       │
     │                              │         │── persistent SSE ────>│
     │                              │         │<── responses/notifs ──│
     │                              │                                │
     │── GET / (SSE) ──────────────>│                                │
     │<── endpoint event ───────────│                                │
     │                              │                                │
     │── POST / (request) ─────────>│                                │
     │                              │── route to backend ───────────>│
     │                              │<── response ───────────────────│
     │<── 202 Accepted ─────────────│                                │
     │<── SSE: response ────────────│                                │
     │                              │                                │
     │                              │<── notification from backend ──│
     │<── SSE: notification ────────│ [via mcp_notification_broadcast]│
```

**Key Components:**
- `SseTransport` - Actual backend connections (gateway as MCP client)
- `McpServerManager` - Manages all backend transports
- `SseConnectionManager` - Routes POST responses to client SSE streams
- `mcp_notification_broadcast` - Broadcasts backend notifications to all subscribers

### Issues Found

#### 1. Endpoint Event Format (Wrong)
**Current:**
```
event: endpoint
data: {"type":"endpoint","endpoint":"/"}
```

**MCP Spec:**
```
event: endpoint
data: /
```

#### 2. Message Event Format (Wrong)
**Current:** Wrapped in `SseMessage` enum
```
event: message
data: {"Response":{"jsonrpc":"2.0","id":1,"result":{...}}}
```

**MCP Spec:** Raw JSON-RPC
```
event: message
data: {"jsonrpc":"2.0","id":1,"result":{...}}
```

#### 3. Redundant Session-Based Endpoints
The `/gateway/stream/*` endpoints duplicate functionality of `/` and `/mcp`.

## Implementation Plan

### Step 1: Fix Endpoint Event Format

**File:** `src-tauri/src/server/routes/mcp.rs`

**For unified gateway (mcp_gateway_get_handler):**
```rust
// Change from:
let endpoint_event = serde_json::json!({
    "type": "endpoint",
    "endpoint": "/"
});
if let Ok(json) = serde_json::to_string(&endpoint_event) {
    yield Ok::<_, Infallible>(Event::default().event("endpoint").data(json));
}

// To:
yield Ok::<_, Infallible>(Event::default().event("endpoint").data("/"));
```

**For individual server (mcp_server_sse_handler):**
```rust
// Change from:
let endpoint_event = serde_json::json!({
    "type": "endpoint",
    "endpoint": format!("/mcp/{}", target_server_id)
});

// To:
yield Ok::<_, Infallible>(
    Event::default()
        .event("endpoint")
        .data(format!("/mcp/{}", target_server_id))
);
```

### Step 2: Fix Message Event Format

**File:** `src-tauri/src/server/routes/mcp.rs`

Change SSE message serialization to send raw JSON-RPC instead of wrapped `SseMessage`:

```rust
// In the SSE stream loop, change from:
match serde_json::to_string(&sse_msg) {
    Ok(json) => {
        let event_type = match &sse_msg {
            SseMessage::Response(_) => "message",
            SseMessage::Notification(_) => "message",
            SseMessage::Endpoint { .. } => "endpoint",
        };
        yield Ok::<_, Infallible>(Event::default().event(event_type).data(json));
    }
}

// To:
match sse_msg {
    SseMessage::Response(response) => {
        if let Ok(json) = serde_json::to_string(&response) {
            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
        }
    }
    SseMessage::Notification(notification) => {
        if let Ok(json) = serde_json::to_string(&notification) {
            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
        }
    }
    SseMessage::Endpoint { .. } => {
        // Handled separately at stream start
    }
}
```

### Step 3: Update SseConnectionManager (If Needed)

**File:** `src-tauri/src/server/state.rs`

The `SseMessage` enum may need to store raw types instead of wrappers:
```rust
pub enum SseMessage {
    Response(JsonRpcResponse),      // Raw response
    Notification(JsonRpcNotification), // Raw notification
    Endpoint { endpoint: String },   // Keep for internal routing
}
```

### Step 4: Remove Session-Based Endpoints

Delete the redundant session-based streaming endpoints:

1. Remove routes from `src-tauri/src/server/mod.rs`:
   - `/gateway/stream` (POST)
   - `/mcp/gateway/stream` (POST)
   - `/gateway/stream/:session_id` (GET, POST, DELETE)
   - `/mcp/gateway/stream/:session_id` (GET, POST, DELETE)

2. Delete `src-tauri/src/server/routes/mcp_streaming.rs`

3. Remove exports from `src-tauri/src/server/routes/mod.rs`

4. Remove `StreamingSessionManager` from `src-tauri/src/server/state.rs` (if not used elsewhere)

5. Consider removing `src-tauri/src/mcp/gateway/streaming.rs` if no longer needed

## Event Flow Summary

### What Flows Over SSE

| Event Type | When | Data Format |
|------------|------|-------------|
| `endpoint` | Connection established | `"/"` or `"/mcp/{server_id}"` |
| `message` | Response to POST request | `{"jsonrpc":"2.0","id":X,"result":{...}}` |
| `message` | Notification from backend | `{"jsonrpc":"2.0","method":"...", "params":{...}}` |

### Unified Gateway (`/`) vs Individual Server (`/mcp/{server_id}`)

| Aspect | Unified (`/`) | Individual (`/mcp/{id}`) |
|--------|---------------|--------------------------|
| Notifications | From ALL allowed servers | From ONE server only |
| Namespacing | `server_id::method` | No prefix (direct) |
| POST routing | By method namespace or broadcast | Direct to server |

## Files to Modify

1. `src-tauri/src/server/routes/mcp.rs` - Fix event formats (~4 locations)
2. `src-tauri/src/server/state.rs` - Simplify SseMessage enum, remove StreamingSessionManager
3. `src-tauri/src/server/mod.rs` - Remove session-based routes
4. `src-tauri/src/server/routes/mod.rs` - Remove streaming exports
5. `src-tauri/src/server/routes/mcp_streaming.rs` - DELETE FILE
6. `src-tauri/src/mcp/gateway/streaming.rs` - DELETE FILE (if unused)
7. `src-tauri/src/server/openapi/mod.rs` - Remove streaming endpoint docs

## Verification

### Manual Testing
1. Use `curl` to test SSE connection:
   ```bash
   curl -N -H "Accept: text/event-stream" -H "Authorization: Bearer <token>" http://localhost:3625/
   ```
   Expected: `event: endpoint\ndata: /\n\n`

2. In another terminal, POST a request:
   ```bash
   curl -X POST -H "Authorization: Bearer <token>" -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' http://localhost:3625/
   ```
   Expected: SSE stream receives `event: message\ndata: {"jsonrpc":"2.0","id":1,"result":{...}}`

### Integration Testing
1. Use MCP SDK's `SSEClientTransport` to connect
2. Verify `initialize`, `tools/list`, `resources/list` work
3. Trigger backend notification and verify it arrives

### Unit Tests
- Add tests in `src-tauri/src/server/routes/mcp.rs` for event format validation
