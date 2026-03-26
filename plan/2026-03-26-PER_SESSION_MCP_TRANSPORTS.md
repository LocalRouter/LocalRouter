# Per-Session MCP Transport Architecture

## Context

MCP server transports (stdio processes, SSE connections, WebSocket connections) are currently **singletons** shared across all client sessions via `McpServerManager`'s global `DashMap<String, Arc<Transport>>` maps. This is fundamentally broken:

1. **MCP protocol violation**: Multiple clients send `initialize` to the same stdio process — the spec requires one `initialize` per connection
2. **No session isolation**: One client's crash affects all others using the same server
3. **Request callback race**: `set_request_callback` on the shared transport means only the last client to initialize gets sampling/elicitation callbacks
4. **No cleanup on disconnect**: When a session ends, server processes keep running forever
5. **Health checks stale**: Dead transports stay in global maps, blocking fresh readiness checks

The fix: each gateway session owns its own set of MCP server transports. When the session ends, its transports are torn down.

## Design Decisions

- **Preview/instructions builder**: Uses ephemeral transports (spawn, get data, tear down)
- **Health checks**: Always do fresh readiness checks (no global "running" state)
- **Notifications**: Per-session only — each session registers callbacks on its own transports
- **`McpServerManager`**: Becomes a config store + transport factory. Global transport maps removed.

---

## Implementation Plan

### Phase 1: Create `SessionTransportSet`

**New file**: `crates/lr-mcp/src/transport/session_transport_set.rs`

A container that owns a set of transports for one gateway session:

```rust
pub struct SessionTransportSet {
    transports: DashMap<String, Arc<dyn Transport>>,
}

impl SessionTransportSet {
    pub fn new() -> Self { ... }
    pub fn insert(&self, server_id: String, transport: Arc<dyn Transport>) { ... }
    pub fn send_request(&self, server_id: &str, request: JsonRpcRequest) -> AppResult<JsonRpcResponse> { ... }
    pub fn stream_request(&self, server_id: &str, request: JsonRpcRequest) -> AppResult<...> { ... }
    pub fn is_running(&self, server_id: &str) -> bool { ... }
    pub fn running_server_ids(&self) -> Vec<String> { ... }
    pub async fn close_all(&self) { ... }  // Kill all transports
    pub async fn close_server(&self, server_id: &str) { ... }
}

impl Drop for SessionTransportSet {
    fn drop(&mut self) {
        // Spawn blocking cleanup for any remaining transports
    }
}
```

**Why `Arc<dyn Transport>`**: The `Transport` trait is already object-safe. Using trait objects avoids needing separate maps per transport type.

**Files changed**:
- `crates/lr-mcp/src/transport/session_transport_set.rs` (new)
- `crates/lr-mcp/src/transport/mod.rs` (add module export)

### Phase 2: Add factory methods to `McpServerManager`

Add methods that **create and return** transports without storing them globally:

```rust
impl McpServerManager {
    /// Create a transport for a server config (does NOT store it)
    pub async fn create_transport(&self, server_id: &str) -> AppResult<Arc<dyn Transport>> { ... }

    /// Create transports for multiple servers in parallel
    pub async fn create_transports(
        &self,
        server_ids: &[String],
        timeout: Duration,
    ) -> (SessionTransportSet, Vec<ServerFailure>) { ... }
}
```

The factory methods reuse the existing logic from `start_stdio_server`, `start_sse_server`, `start_websocket_server` but return the transport instead of inserting into global maps. Notification/request callback setup happens at the call site (the gateway), not in the factory.

**Files changed**:
- `crates/lr-mcp/src/manager.rs` (add factory methods alongside existing ones — don't remove old ones yet)

### Phase 3: Add transports to `GatewaySession`

```rust
pub struct GatewaySession {
    // ... existing fields ...

    /// Per-session MCP server transports (owned by this session)
    pub transports: Option<Arc<SessionTransportSet>>,
}
```

Using `Option<Arc<SessionTransportSet>>`:
- `Option` because transports are created during `handle_initialize`, not at session creation
- `Arc` because we need to extract it with a brief read lock, then use it without holding the session lock (avoids lock contention during requests)

**Files changed**:
- `crates/lr-mcp/src/gateway/session.rs`

### Phase 4: Wire `handle_initialize` to use `SessionTransportSet`

In `gateway.rs` `handle_initialize()` (lines 1786-1821):

**Before**: Calls `self.server_manager.start_server()` which stores transports globally
**After**: Calls `self.server_manager.create_transports()` which returns a `SessionTransportSet`

Then:
1. Register notification callbacks on each transport in the set (per-session, not global)
2. Register request callbacks (sampling/elicitation) on each transport in the set
3. Store the `SessionTransportSet` on the session via `session.write().transports = Some(Arc::new(transport_set))`

Notification handler registration changes:
- Remove `register_notification_handlers` (global) — replace with per-session registration during initialize
- Each transport's notification callback invalidates only THIS session's caches and forwards to THIS session's SSE connection
- Remove `notification_handlers_registered: DashMap` from `McpGateway`

Request callback registration changes:
- `set_request_callback` is called on individual transports in the set, not on the global manager
- Each session gets its own callback — no more "last one wins" race

**Files changed**:
- `crates/lr-mcp/src/gateway/gateway.rs` (handle_initialize, register_notification_handlers, register_request_handlers)

### Phase 5: Route all requests through session transports

Change every call site that currently does `self.server_manager.send_request(&server_id, request)` to instead get the `SessionTransportSet` from the session and route through it.

**Pattern**: Extract `Arc<SessionTransportSet>` from session with brief read lock, then use it:
```rust
let transports = session.read().await.transports.clone()
    .ok_or_else(|| AppError::Mcp("Session not initialized".into()))?;
transports.send_request(&server_id, request).await
```

**Call sites to change**:

| File | What | Line(s) |
|------|------|---------|
| `gateway/router.rs` | `broadcast_request()` — change signature to accept `&SessionTransportSet` instead of `&McpServerManager` | 15-18 |
| `gateway/gateway.rs` | `handle_initialize` broadcast calls | ~1995, ~2513 |
| `gateway/gateway.rs` | `broadcast_and_return_first` for ping/logging | various |
| `gateway/gateway.rs` | `notifications/cancelled` forwarding | various |
| `gateway/gateway_tools.rs` | `handle_tools_list` pagination + `handle_tools_call` tool execution | 133, 189 |
| `gateway/gateway_resources.rs` | `handle_resources_list` pagination + `handle_resources_read` + subscribe/unsubscribe | 175, 669 |
| `gateway/gateway_prompts.rs` | `handle_prompts_list` pagination + `handle_prompts_get` | 154 |

**Files changed**:
- `crates/lr-mcp/src/gateway/router.rs`
- `crates/lr-mcp/src/gateway/gateway.rs`
- `crates/lr-mcp/src/gateway/gateway_tools.rs`
- `crates/lr-mcp/src/gateway/gateway_resources.rs`
- `crates/lr-mcp/src/gateway/gateway_prompts.rs`

### Phase 6: Session cleanup closes transports

**`terminate_session`** (gateway.rs ~2798):
```rust
pub async fn terminate_session(&self, session_key: &str) -> Result<(), String> {
    if let Some((_, session)) = self.sessions.remove(session_key) {
        if let Ok(session_read) = session.try_read() {
            if let Some(transports) = &session_read.transports {
                transports.close_all().await;
            }
        }
        Ok(())
    } else {
        Err(...)
    }
}
```

**`terminate_sessions_for_client`**: Same pattern — close transports before removing.

**`cleanup_expired_sessions`**: Change to `async fn`. Close transports for expired sessions. Update caller in `lr-server/src/lib.rs` to `.await`.

**SSE disconnect** (mcp.rs ~356): Call `gateway.terminate_session(&session_id_cleanup)` to close transports when SSE stream ends.

**Re-initialize path**: If a session already has transports and reinitializes, close old transports first.

**`SessionTransportSet::Drop`**: Spawn a tokio task to close remaining transports as a safety net.

**Files changed**:
- `crates/lr-mcp/src/gateway/gateway.rs` (terminate_session, terminate_sessions_for_client, cleanup_expired_sessions)
- `crates/lr-server/src/routes/mcp.rs` (SSE disconnect cleanup)
- `crates/lr-server/src/lib.rs` (async cleanup caller)

### Phase 7: Preview uses ephemeral transports

`build_instructions_context` (gateway.rs ~3062): Change to use `create_transports()` factory, get instructions, then `close_all()` the temporary transport set.

**Files changed**:
- `crates/lr-mcp/src/gateway/gateway.rs` (build_instructions_context)

### Phase 8: Remove global transports from `McpServerManager`

Remove from `McpServerManager`:
- `stdio_transports`, `sse_transports`, `websocket_transports` DashMap fields
- `start_server`, `stop_server`, `send_request`, `stream_request` methods (replaced by factory + session)
- `is_running`, `supports_streaming`, `get_transport_type` methods
- `on_notification`, `remove_notification_handler`, `clear_notification_handlers`, `dispatch_notification` methods
- `set_request_callback` method
- `shutdown_all` method

Keep on `McpServerManager`:
- `configs` DashMap (config management)
- `oauth_manager` (OAuth for servers)
- `get_config`, `add_config`, `remove_config`, `list_configs`
- `get_server_health` — simplified to always do readiness check
- Factory methods from Phase 2

Remove from `McpGateway`:
- `notification_handlers_registered` DashMap

**Files changed**:
- `crates/lr-mcp/src/manager.rs` (major cleanup)
- `crates/lr-mcp/src/gateway/gateway.rs` (remove notification_handlers_registered)

### Phase 9: Simplify health checks

`get_server_health` no longer checks global transports (they don't exist). It always calls `check_server_readiness()`:
- For stdio: try to spawn command, verify it can start, tear down → Ready/Unhealthy
- For SSE/WebSocket: HTTP HEAD check → Ready/Unhealthy
- No more "Healthy"/"Process not running" for global state — just Ready or Unhealthy

Also incorporate the earlier fix: stdout EOF without output is treated as success (process was spawnable).

**Files changed**:
- `crates/lr-mcp/src/manager.rs` (simplify get_server_health)

---

## Files Summary

| File | Change |
|------|--------|
| `crates/lr-mcp/src/transport/session_transport_set.rs` | **NEW** — Per-session transport container |
| `crates/lr-mcp/src/transport/mod.rs` | Add module export |
| `crates/lr-mcp/src/manager.rs` | Add factory methods, remove global transport state, simplify health |
| `crates/lr-mcp/src/gateway/session.rs` | Add `transports: Option<Arc<SessionTransportSet>>` field |
| `crates/lr-mcp/src/gateway/gateway.rs` | Rewire initialize, routing, cleanup, notifications; remove global tracking |
| `crates/lr-mcp/src/gateway/router.rs` | Change `broadcast_request` to use `SessionTransportSet` |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Route through session transports |
| `crates/lr-mcp/src/gateway/gateway_resources.rs` | Route through session transports |
| `crates/lr-mcp/src/gateway/gateway_prompts.rs` | Route through session transports |
| `crates/lr-server/src/routes/mcp.rs` | SSE disconnect → terminate session |
| `crates/lr-server/src/lib.rs` | Async cleanup caller |

---

## Verification

1. **Compile**: `cargo check` — ensure all global transport references are removed
2. **Tests**: `cargo test --package lr-mcp` — existing tests + new tests for `SessionTransportSet`
3. **Manual test — session isolation**:
   - Connect two clients via SSE to the gateway simultaneously
   - Both should get their own MCP server processes (verify via process list)
   - Disconnect one — its processes should die, other client unaffected
4. **Manual test — reconnect**:
   - Connect, disconnect, reconnect — fresh processes each time
   - No stale "Process not running" health status
5. **Manual test — health check**:
   - Health check for a configured stdio server should report Ready (spawns, verifies, tears down)
   - Refreshing should always do a fresh check
6. **Manual test — MCP via LLM**:
   - MCP via LLM sessions should get their own transports
   - Session expiry should clean up transports
7. **Manual test — preview**:
   - Instructions preview should work (ephemeral transports)
   - Should not interfere with active client sessions

---

## Mandatory Final Steps

1. **Plan Review**: Review this plan against implementation — identify missed changes
2. **Test Coverage Review**: Add tests for SessionTransportSet, factory methods, session cleanup
3. **Bug Hunt**: Re-read implementation looking for races, missing cleanup paths, lock contention
4. **Commit**: Stage only modified files, commit with conventional commit format
