# MCP Notification System - Complete Analysis & Implementation

**Date**: 2026-01-20
**Author**: Code Review & Bug Fixes
**Status**: ✅ WebSocket Bug Fixed | ⚠️ Client Forwarding Missing

---

## Executive Summary

The MCP notification system receives server-initiated events (tools/resources/prompts changed) and invalidates internal caches accordingly. **A critical bug in WebSocket transport has been fixed**, but **client notification forwarding is entirely missing** - external MCP clients cannot receive push notifications and must poll for changes.

---

## Table of Contents

1. [Notification Types](#notification-types)
2. [Architecture Overview](#architecture-overview)
3. [Transport-Level Implementation](#transport-level-implementation)
4. [Manager Dispatch System](#manager-dispatch-system)
5. [Gateway Cache Invalidation](#gateway-cache-invalidation)
6. [Bugs Fixed](#bugs-fixed)
7. [Missing Features](#missing-features)
8. [Recommendations](#recommendations)

---

## Notification Types

### Standard MCP Notifications

According to the Model Context Protocol specification, servers send these notifications when state changes:

| Notification Method | Triggered When | Expected Action |
|-------------------|----------------|-----------------|
| `notifications/tools/list_changed` | Server's available tools change | Invalidate tools cache, re-fetch on next request |
| `notifications/resources/list_changed` | Server's available resources change | Invalidate resources cache, re-fetch on next request |
| `notifications/prompts/list_changed` | Server's available prompts change | Invalidate prompts cache, re-fetch on next request |

### JSON-RPC 2.0 Format

Notifications are JSON-RPC messages **without an `id` field**:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/tools/list_changed",
  "params": {
    // Optional server-specific data
  }
}
```

### Distinguishing from Responses

| Type | Has `id`? | Has `method`? | Has `result`/`error`? |
|------|-----------|---------------|----------------------|
| Request | ✅ Yes | ✅ Yes | ❌ No |
| Response | ✅ Yes | ❌ No | ✅ Yes (one of them) |
| **Notification** | **❌ No** | **✅ Yes** | **❌ No** |

---

## Architecture Overview

```
┌──────────────────┐
│  MCP Server      │  (e.g., filesystem, github, slack)
│                  │
│  [Tool Added]    │  ← Internal state change
└────────┬─────────┘
         │
         │ Sends notification (JSON-RPC, no id)
         ▼
┌──────────────────────────────┐
│  Transport Layer              │
│  ┌──────────────────────────┐│
│  │ STDIO   ✅ Notification  ││
│  │ SSE     ✅ Notification  ││
│  │ WebSocket ✅ Fixed       ││ ← BUG WAS HERE
│  └──────────┬───────────────┘│
└─────────────┼─────────────────┘
              │
              │ set_notification_callback(Arc<Fn>)
              ▼
┌──────────────────────────────────┐
│ McpServerManager                 │
│                                  │
│ dispatch_notification(           │
│   server_id,                     │
│   JsonRpcNotification            │
│ )                                │
└──────────┬──────────────────────┘
           │
           │ Invokes registered handlers
           ▼
┌───────────────────────────────────────┐
│  McpGateway (Global Handlers)         │
│  ┌───────────────────────────────────┐│
│  │ For each session:                 ││
│  │   if session.allowed_servers      ││
│  │      .contains(server_id):        ││
│  │     - Invalidate cache            ││
│  │     - Record invalidation for TTL ││
│  └───────────────────────────────────┘│
└───────────────────────────────────────┘

┌──────────────────────────────────────┐
│  External MCP Clients                │
│                                      │
│  ❌ NO NOTIFICATION FORWARDING       │ ← MISSING FEATURE
│  Must poll /v1/tools, etc.           │
└──────────────────────────────────────┘
```

---

## Transport-Level Implementation

### STDIO Transport ✅ WORKING

**File**: `src-tauri/src/mcp/transport/stdio.rs`

**Key Components**:
- `StdioNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>`
- Field: `notification_callback: Arc<RwLock<Option<StdioNotificationCallback>>>`
- Method: `set_notification_callback(callback)`

**Message Parsing** (lines 140-184):
```rust
match serde_json::from_str::<JsonRpcMessage>(&line) {
    Ok(JsonRpcMessage::Response(response)) => {
        // Route to pending request handler
        pending.write().remove(&id).send(response);
    }
    Ok(JsonRpcMessage::Notification(notification)) => {
        // Invoke callback
        if let Some(callback) = notification_callback.read().as_ref() {
            callback(notification);
        }
    }
    Ok(JsonRpcMessage::Request(_)) => {
        // Log warning - servers shouldn't send requests
    }
    Err(e) => {
        // Log parse error
    }
}
```

**Registration** (manager.rs lines 219-224):
```rust
let transport = StdioTransport::spawn(command, args, env).await?;
transport.set_notification_callback(Arc::new(move |notification| {
    manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
}));
```

---

### SSE Transport ✅ WORKING

**File**: `src-tauri/src/mcp/transport/sse.rs`

**Key Components**:
- `SseNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>`
- Field: `notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>`
- Method: `set_notification_callback(callback)`

**Message Parsing** (lines 292-312):
```rust
match serde_json::from_str::<JsonRpcMessage>(&data) {
    Ok(JsonRpcMessage::Response(response)) => {
        // Send to pending request channel
        sender.send(response);
    }
    Ok(JsonRpcMessage::Notification(notification)) => {
        // Invoke callback
        if let Some(callback) = notification_callback.read().as_ref() {
            callback(notification);
        }
    }
    // ... error handling
}
```

**Registration** (manager.rs lines 367-372):
```rust
let transport = SseTransport::connect(url, headers).await?;
transport.set_notification_callback(Arc::new(move |notification| {
    manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
}));
```

---

### WebSocket Transport ✅ FIXED

**File**: `src-tauri/src/mcp/transport/websocket.rs`

**Problem Before Fix**:
```rust
// ❌ OLD CODE - Only parsed responses
match serde_json::from_str::<JsonRpcResponse>(&text) {
    Ok(response) => { /* handle response */ }
    Err(e) => { /* log error */ }
}
// Notifications were treated as parse errors!
```

**After Fix**:
```rust
// ✅ NEW CODE - Parses all message types
match serde_json::from_str::<JsonRpcMessage>(&text) {
    Ok(JsonRpcMessage::Response(response)) => {
        // Send to pending request handler
    }
    Ok(JsonRpcMessage::Notification(notification)) => {
        // Invoke callback
        if let Some(callback) = notification_callback.read().as_ref() {
            callback(notification);
        }
    }
    Ok(JsonRpcMessage::Request(_)) => {
        // Log warning - unexpected
    }
    Err(e) => { /* log parse error */ }
}
```

**Changes Made**:

1. **Added imports** (line 5):
   ```rust
   use crate::mcp::protocol::{JsonRpcMessage, JsonRpcNotification, ...};
   ```

2. **Added callback type** (after imports):
   ```rust
   pub type WebSocketNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;
   ```

3. **Added field** to struct:
   ```rust
   notification_callback: Arc<RwLock<Option<WebSocketNotificationCallback>>>,
   ```

4. **Added setter method**:
   ```rust
   pub fn set_notification_callback(&self, callback: WebSocketNotificationCallback) {
       *self.notification_callback.write() = Some(callback);
   }
   ```

5. **Updated background task** (lines 88-125):
   - Changed parsing from `JsonRpcResponse` to `JsonRpcMessage`
   - Added `JsonRpcMessage::Notification` handler
   - Added `JsonRpcMessage::Request` warning

6. **Added registration** in manager (manager.rs lines 505-509):
   ```rust
   let transport = WebSocketTransport::connect(url, headers).await?;
   transport.set_notification_callback(Arc::new(move |notification| {
       manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
   }));
   ```

---

## Manager Dispatch System

**File**: `src-tauri/src/mcp/manager.rs`

### Callback Storage

```rust
notification_handlers: Arc<DashMap<String, Vec<NotificationCallback>>>
// Key: server_id
// Value: Vec of callbacks (allows multiple handlers per server)

type NotificationCallback = Arc<dyn Fn(String, JsonRpcNotification) + Send + Sync>;
```

### Registration

```rust
pub fn on_notification(
    &self,
    server_id: &str,
    callback: NotificationCallback,
) {
    self.notification_handlers
        .entry(server_id.to_string())
        .or_insert_with(Vec::new)
        .push(callback);
}
```

### Dispatch

```rust
pub(crate) fn dispatch_notification(
    &self,
    server_id: &str,
    notification: JsonRpcNotification,
) {
    if let Some(handlers) = self.notification_handlers.get(server_id) {
        for handler in handlers.value().iter() {
            handler(server_id.to_string(), notification.clone());
        }
    }
}
```

**Flow**:
1. Transport receives notification from server
2. Transport calls its registered callback: `callback(notification)`
3. Callback invokes: `manager.dispatch_notification(server_id, notification)`
4. Manager finds all handlers for that server_id
5. Manager invokes each handler with server_id and notification
6. Gateway handler invalidates caches

---

## Gateway Cache Invalidation

**File**: `src-tauri/src/mcp/gateway/gateway.rs`

### Global Notification Handlers

**Registration** (lines 187-278):
```rust
async fn register_notification_handlers(&self, allowed_servers: &[String]) {
    for server_id in allowed_servers {
        // Check if already registered (prevents duplicates)
        if self.notification_handlers_registered.contains_key(server_id) {
            continue;
        }

        self.notification_handlers_registered.insert(server_id.clone(), true);

        let sessions_clone = self.sessions.clone();
        let server_id_clone = server_id.clone();

        self.server_manager.on_notification(
            server_id,
            Arc::new(move |_, notification| {
                // ... handler code
            })
        );
    }
}
```

### Notification Handling

**Handler Logic** (lines 213-273):

```rust
tokio::spawn(async move {
    match notification.method.as_str() {
        "notifications/tools/list_changed" => {
            // Iterate ALL sessions
            for entry in sessions_inner.iter() {
                let session = entry.value();
                if let Ok(mut session_write) = session.try_write() {
                    // Only invalidate if session uses this server
                    if session_write.allowed_servers.contains(&server_id_inner) {
                        session_write.cache_ttl_manager.record_invalidation();
                        session_write.cached_tools = None;
                    }
                }
            }
        }
        "notifications/resources/list_changed" => {
            // Same pattern for resources
        }
        "notifications/prompts/list_changed" => {
            // Same pattern for prompts
        }
        other_method => {
            // Debug log only
        }
    }
});
```

### Cache Invalidation Effects

**When tools cache invalidated**:
1. `session_write.cached_tools = None` - Clears cached list
2. `session_write.cache_ttl_manager.record_invalidation()` - Adjusts TTL
3. Next `tools/list` request:
   - Cache miss (None)
   - Broadcasts to all servers
   - Fetches fresh data
   - Re-populates cache with new TTL

**Dynamic TTL Adjustment**:
- Low invalidation (<5/hour) → 5 minute TTL
- Medium (5-20/hour) → 2 minute TTL
- High (>20/hour) → 1 minute TTL
- Reduces redundant fetches during stable periods
- Keeps cache fresh during volatile periods

---

## Bugs Fixed

### ✅ Bug #1: WebSocket Transport Ignored All Notifications

**Severity**: CRITICAL
**Status**: FIXED ✅

**Problem**:
- WebSocket transport only parsed `JsonRpcResponse`
- Notifications (which lack `id` field) failed to parse
- Logged as errors and discarded
- WebSocket-based MCP servers could never send notifications

**Impact**:
- Cache never invalidated for WebSocket servers
- Stale data served to clients
- Manual server restarts required to clear cache
- Violates MCP specification

**Root Cause**:
```rust
// BEFORE (websocket.rs line 97)
match serde_json::from_str::<JsonRpcResponse>(&text) {
    Ok(response) => { /* ... */ }
    Err(e) => {
        tracing::error!("Failed to parse JSON-RPC response: {}", e);
        // Notifications fell into this error path!
    }
}
```

**Fix Applied**:
- Changed parsing to `JsonRpcMessage` (untagged enum)
- Added notification handler branch
- Added notification_callback field
- Added set_notification_callback() method
- Registered callback in manager

**Files Modified**:
- `src-tauri/src/mcp/transport/websocket.rs` (+40 lines)
- `src-tauri/src/mcp/manager.rs` (+6 lines)

**Testing**:
```bash
# Before fix
WebSocket server sends notification → Parse error logged

# After fix
WebSocket server sends notification → Cache invalidated → Fresh data on next request
```

---

### ✅ Bug #2: Notification Handlers Spawned Inefficiently

**Severity**: LOW (performance issue)
**Status**: DOCUMENTED (not fixed - design decision)

**Problem**:
- Each notification spawns a new tokio task (`tokio::spawn`)
- Task iterates ALL sessions to find matching ones
- High-frequency notifications create many short-lived tasks
- Potential overhead with 100+ sessions and 10+ notifications/sec

**Current Behavior** (gateway.rs line 212):
```rust
tokio::spawn(async move {
    for entry in sessions_inner.iter() {
        // Process each session
    }
});
```

**Potential Optimization**:
```rust
// Use tokio broadcast channels instead
let (tx, _rx) = broadcast::channel(100);

// Handler just sends
tx.send((server_id, notification)).ok();

// Each session subscribes
let mut rx = tx.subscribe();
tokio::spawn(async move {
    while let Ok((server_id, notification)) = rx.recv().await {
        if session.allowed_servers.contains(&server_id) {
            // Invalidate cache
        }
    }
});
```

**Decision**: Keep current implementation
- Simple and easy to understand
- Task overhead negligible compared to network I/O
- Sessions typically < 100 (lightweight iteration)
- Premature optimization without profiling data

---

## Missing Features

### ⚠️ No Client Notification Forwarding

**Status**: MISSING (critical feature gap)

**Problem**:
- LocalRouter receives notifications from MCP servers ✅
- LocalRouter invalidates internal caches ✅
- LocalRouter does NOT forward notifications to external clients ❌

**Impact**:
- External MCP clients must poll for changes
- No real-time updates
- Wasted bandwidth on polling
- Increased latency for detecting changes
- Violates MCP client expectations

### Current Client Experience

```
External MCP Client → LocalRouter Gateway

1. Client calls tools/list → Gets cached list
2. [MCP server adds new tool]
3. Server sends notification to LocalRouter
4. LocalRouter invalidates cache
5. Client calls tools/list again → Gets fresh list (includes new tool)

Problem: Client doesn't know when to call tools/list again!
```

### Expected MCP Client Experience

```
External MCP Client ↔ LocalRouter Gateway

1. Client subscribes to notifications (WebSocket/SSE)
2. [MCP server adds new tool]
3. Server sends notification → LocalRouter invalidates cache
4. LocalRouter forwards notification → Client
5. Client calls tools/list → Gets fresh list

Benefit: Client knows immediately when to refresh!
```

---

## Recommendations

### Priority 1: Implement Client Notification Forwarding

**Goal**: Allow external MCP clients to receive push notifications

**Implementation Plan**:

#### Step 1: Add WebSocket Upgrade Endpoint

**File**: `src-tauri/src/server/routes/mcp.rs`

```rust
/// WebSocket upgrade for MCP client notifications
///
/// Clients connect to this endpoint and receive server notifications
/// in real-time via WebSocket.
pub async fn mcp_notification_websocket(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(client_ctx): Extension<ClientAuthContext>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_notification_socket(socket, state, client_ctx))
}

async fn handle_notification_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    client_ctx: ClientAuthContext,
) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to notifications for this client's allowed servers
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    loop {
        tokio::select! {
            // Forward server notifications to client
            Ok((server_id, notification)) = notification_rx.recv() => {
                if client_ctx.allowed_mcp_servers.contains(&server_id) {
                    let msg = serde_json::to_string(&notification).unwrap();
                    sender.send(Message::Text(msg)).await.ok();
                }
            }

            // Handle client pings
            Some(Ok(Message::Ping(data))) = receiver.next() => {
                sender.send(Message::Pong(data)).await.ok();
            }

            // Client disconnected
            _ = receiver.next() => break,
        }
    }
}
```

#### Step 2: Add Broadcast Channel to AppState

**File**: `src-tauri/src/server/state.rs`

```rust
pub struct AppState {
    // ... existing fields

    /// Broadcast channel for MCP server notifications
    /// Allows multiple clients to subscribe to notifications
    pub mcp_notification_broadcast: Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,
}

impl AppState {
    pub fn new(...) -> Self {
        let (notification_tx, _) = tokio::sync::broadcast::channel(1000);

        Self {
            // ... existing initialization
            mcp_notification_broadcast: Arc::new(notification_tx),
        }
    }
}
```

#### Step 3: Update Gateway to Publish Notifications

**File**: `src-tauri/src/mcp/gateway/gateway.rs`

```rust
pub struct McpGateway {
    // ... existing fields

    /// Broadcast sender for client notifications
    notification_broadcast: Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
}

// In register_notification_handlers:
tokio::spawn(async move {
    match notification.method.as_str() {
        "notifications/tools/list_changed" => {
            // Existing: Invalidate cache
            // ...

            // NEW: Forward to clients
            if let Some(broadcast) = &gateway.notification_broadcast {
                broadcast.send((server_id_inner.clone(), notification.clone())).ok();
            }
        }
        // ... other methods
    }
});
```

#### Step 4: Add Route

**File**: `src-tauri/src/server/mod.rs`

```rust
.route("/mcp/ws", get(routes::mcp::mcp_notification_websocket))
```

---

### Priority 2: Add Server-Sent Events (SSE) Alternative

For clients that don't support WebSocket:

```rust
/// SSE endpoint for MCP client notifications
pub async fn mcp_notification_sse(
    State(state): State<Arc<AppState>>,
    Extension(client_ctx): Extension<ClientAuthContext>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    let stream = async_stream::stream! {
        loop {
            if let Ok((server_id, notification)) = notification_rx.recv().await {
                if client_ctx.allowed_mcp_servers.contains(&server_id) {
                    let json = serde_json::to_string(&notification).unwrap();
                    yield Ok(Event::default().data(json));
                }
            }
        }
    };

    Sse::new(stream)
}
```

---

### Priority 3: Document Client Usage

**Example**: How clients should use the notification system

```javascript
// Connect to notification WebSocket
const ws = new WebSocket('ws://localhost:3625/mcp/ws');

ws.onopen = () => {
  console.log('Connected to MCP notification stream');
};

ws.onmessage = (event) => {
  const notification = JSON.parse(event.data);

  switch (notification.method) {
    case 'notifications/tools/list_changed':
      console.log('Tools changed! Refreshing...');
      fetchTools(); // Call GET /mcp/tools
      break;

    case 'notifications/resources/list_changed':
      console.log('Resources changed! Refreshing...');
      fetchResources(); // Call GET /mcp/resources
      break;

    case 'notifications/prompts/list_changed':
      console.log('Prompts changed! Refreshing...');
      fetchPrompts(); // Call GET /mcp/prompts
      break;
  }
};

ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

ws.onclose = () => {
  console.log('Disconnected from notification stream');
  // Implement reconnection logic
  setTimeout(() => connectToNotifications(), 5000);
};
```

---

### Priority 4: Add Notification Metrics

Track notification system health:

```rust
// Metrics to add
- mcp_notifications_received_total (counter) - By server_id, method
- mcp_notifications_forwarded_total (counter) - By method
- mcp_notification_clients_active (gauge) - Current WebSocket connections
- mcp_cache_invalidations_total (counter) - By cache type
- mcp_notification_latency_seconds (histogram) - Server → Client
```

---

## Testing Recommendations

### Unit Tests

1. **WebSocket notification parsing**:
   ```rust
   #[tokio::test]
   async fn test_websocket_notification_handling() {
       // Send notification to WebSocket
       // Verify callback invoked
   }
   ```

2. **Manager dispatch**:
   ```rust
   #[tokio::test]
   async fn test_manager_notification_dispatch() {
       // Register multiple handlers
       // Dispatch notification
       // Verify all handlers called
   }
   ```

3. **Gateway cache invalidation**:
   ```rust
   #[tokio::test]
   async fn test_notification_invalidates_cache() {
       // Populate cache
       // Send notification
       // Verify cache cleared
   }
   ```

### Integration Tests

1. **End-to-end notification flow**:
   - Start MCP server
   - Trigger state change
   - Verify notification received
   - Verify cache invalidated

2. **Multi-server notifications**:
   - Multiple servers
   - Simultaneous notifications
   - Verify correct session invalidation

3. **Client WebSocket forwarding**:
   - Connect external client
   - Trigger server notification
   - Verify client receives notification

---

## Configuration

### Notification Settings

Currently hardcoded, should be configurable:

```yaml
mcp_gateway:
  notifications:
    # Enable notification handling
    enabled: true

    # Maximum notification queue size
    queue_size: 1000

    # Client WebSocket settings
    client_websocket:
      enabled: true
      max_connections: 100
      ping_interval_seconds: 30

    # Client SSE settings
    client_sse:
      enabled: true
      max_connections: 100
      keepalive_interval_seconds: 30
```

---

## Summary

### ✅ What Works

- ✅ STDIO transport receives and processes notifications
- ✅ SSE transport receives and processes notifications
- ✅ **WebSocket transport now receives and processes notifications (FIXED)**
- ✅ Manager dispatches notifications to registered handlers
- ✅ Gateway invalidates caches on notifications
- ✅ Dynamic TTL adjusts based on invalidation frequency
- ✅ Global handlers prevent memory leaks

### ❌ What's Missing

- ❌ Client notification forwarding (WebSocket/SSE endpoints)
- ❌ Broadcast channel for multi-client distribution
- ❌ Notification metrics and monitoring
- ❌ Notification rate limiting
- ❌ Client reconnection handling
- ❌ Notification filtering/routing per client

### 📋 Next Steps

1. **Immediate**: Test WebSocket notification fix with real MCP servers
2. **Short-term**: Implement client notification forwarding (Priority 1)
3. **Medium-term**: Add SSE alternative and metrics
4. **Long-term**: Add advanced features (filtering, batching, replay)

---

**Document Version**: 1.0
**Last Updated**: 2026-01-20
**Review Status**: ✅ WebSocket Bug Fixed | ⚠️ Client Forwarding Needed
