# Fix MCP Client Capabilities Forwarding

## Context

MCP servers that conditionally expose tools based on client capabilities (sampling, elicitation) don't see those capabilities through LocalRouter. Four distinct issues prevent the MCP handshake from properly forwarding client capabilities to backend servers.

## Files to Modify

1. `crates/lr-mcp/src/protocol.rs` — Add `MCP_PROTOCOL_VERSION` constant
2. `crates/lr-mcp/src/gateway/gateway.rs` — Send `notifications/initialized` after init broadcast; handle client notifications; update hardcoded protocol versions
3. `crates/lr-mcp/src/transport/sse.rs` — Remove premature initialize from `connect()`
4. `src-tauri/src/ui/commands_mcp.rs` — Add init handshake in `get_mcp_server_capabilities`
5. `src/lib/mcp-client.ts` — Read actual capabilities & protocol version
6. `crates/lr-mcp/src/gateway/merger.rs` — Update test protocol versions
7. `crates/lr-mcp/src/gateway/tests.rs` — Update test protocol versions

## Implementation Steps

### Step 1: Add `MCP_PROTOCOL_VERSION` constant

**File:** `crates/lr-mcp/src/protocol.rs`

Add after the error code constants (line ~160):
```rust
/// Latest MCP protocol version supported by LocalRouter.
/// Elicitation requires >= 2025-03-26; sampling improvements >= 2025-06-18.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
```

### Step 2: Fix SSE transport — remove premature initialize

**File:** `crates/lr-mcp/src/transport/sse.rs`

The `connect()` method (lines 164-296) currently sends a hardcoded `initialize` with `capabilities: {}` and `protocolVersion: "2024-11-05"`. This pre-initializes SSE servers with empty capabilities. The gateway's later broadcast of the real init is rejected ("already initialized").

**Changes:**
- Remove the init request construction and POST validation (lines 170-228)
- Go directly to setting up the SSE stream (pending, closed, stream_ready structs at line 230)
- The SSE GET stream (sse_stream_task) already validates connectivity — if it connects, the server is reachable
- Change `next_id` from `2` to `1` (line 287) since we no longer use ID 1 for the init
- Remove the comment about starting at 2

### Step 3: Gateway — send `notifications/initialized` after init broadcast

**File:** `crates/lr-mcp/src/gateway/gateway.rs`

**3a: After init broadcast succeeds (line ~1264, after `init_results` is built):**

Send `notifications/initialized` to each successfully initialized server. Use `JsonRpcRequest::new(None, "notifications/initialized".to_string(), Some(json!({})))` — the stdio transport already handles notifications with `id: None` as fire-and-forget (stdio.rs:394-430).

**3b: Handle client's `notifications/initialized` (line ~562, in `handle_request_with_skills`):**

Before the `is_broadcast` routing check, intercept notifications from the client:
- `notifications/initialized` → silently consume (already handled during init)
- Other `notifications/*` → silently consume (return empty success)

This prevents the "method not found" error that currently occurs.

**3c: Preview context init (line ~2286-2296):**

Update the `get_or_build_preview_context` init request:
- Replace `"2024-11-05"` with `MCP_PROTOCOL_VERSION`
- Replace `"capabilities": {}` with `{ "sampling": {}, "elicitation": { "form": {} }, "roots": { "listChanged": true } }`
- After init broadcast, send `notifications/initialized` to each server

**3d: Update fallback protocol versions (lines 1133, 1287):**

Replace hardcoded `"2024-11-05"` with `MCP_PROTOCOL_VERSION` in the two fallback `MergedCapabilities` constructions.

### Step 4: Fix `get_mcp_server_capabilities` — add init handshake

**File:** `src-tauri/src/ui/commands_mcp.rs` (after line 2180)

Before the `tools/list` request, send:
1. `initialize` request with `MCP_PROTOCOL_VERSION`, capabilities `{ sampling: {}, elicitation: { form: {} }, roots: { listChanged: true } }`, and clientInfo
2. `notifications/initialized` notification

This ensures the server knows about client capabilities before returning tools.

### Step 5: Fix frontend — read actual values

**File:** `src/lib/mcp-client.ts`

**5a:** Store the declared capabilities in a local variable before passing to the Client constructor (line ~199). Use that variable at line ~300 instead of hardcoding `{ sampling: true, ... }`.

**5b:** At line 312, replace hardcoded `"2024-11-05"` with the actual negotiated protocol version from the transport: `(this.transport as any)?._protocolVersion || "unknown"`. The SDK stores this via `transport.setProtocolVersion()` after init.

### Step 6: Update test fixtures

Update hardcoded `"2024-11-05"` in test files to use `MCP_PROTOCOL_VERSION`:
- `crates/lr-mcp/src/gateway/tests.rs:141,157,180`
- `crates/lr-mcp/src/gateway/merger.rs:2308,2326,2346`

## Mandatory Final Steps

1. **Plan Review** — Check all changes against this plan for completeness
2. **Test Coverage** — Ensure `notifications/initialized` forwarding is tested; verify SSE connect without init
3. **Bug Hunt** — Check for race conditions (notification sent before server ready), error handling (notification send failure shouldn't block init), protocol version string comparison in tests

## Verification

1. `cargo test && cargo clippy && cargo fmt`
2. Manual test: configure a stdio MCP server that conditionally registers tools in `oninitialized` based on `getClientCapabilities()?.elicitation` — verify tools appear in Try-it-out and in the permissions tree
3. Check the Try-it-out connection tab shows actual protocol version (not "2024-11-05") and correct client capabilities
