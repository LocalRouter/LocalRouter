# Plan: Remove Direct MCP Proxy Routes

## Goal
Remove the `/mcp/:server_id` and `/mcp/:server_id/stream` proxy routes, keeping only the unified gateway (`/`). The "Direct MCP" mode in Try It Out will route through the gateway with access restricted to a single server.

## Changes

### 1. Backend: Extend `X-MCP-Access` header to support specific server IDs
**File:** `crates/lr-server/src/routes/mcp.rs`

Currently `X-MCP-Access` only accepts `"all"` or `"none"`. Change the parsing so it also accepts a specific server ID string (anything that's not "all" or "none" is treated as a server ID). Update both `mcp_gateway_handler` (~line 394) and `mcp_gateway_get_handler` (~line 138) to build `allowed_servers` with just that one ID.

### 2. Backend: Remove proxy route handlers
**File:** `crates/lr-server/src/routes/mcp.rs`

Delete the three proxy handler functions:
- `mcp_server_handler` (line 639, ~475 lines)
- `mcp_server_sse_handler` (line 1115, ~335 lines)
- `mcp_server_streaming_handler` (line 1450, ~160 lines)

### 3. Backend: Remove route registrations
**File:** `crates/lr-server/src/lib.rs`

Remove from the `mcp_routes` Router:
- `.route("/mcp/:server_id", get(...).post(...))`
- `.route("/mcp/:server_id/stream", post(...))`

### 4. Backend: Remove handler exports
**File:** `crates/lr-server/src/routes/mod.rs`

Remove exports for `mcp_server_handler`, `mcp_server_sse_handler`, `mcp_server_streaming_handler`.

### 5. Backend: Remove OpenAPI registrations
**File:** `crates/lr-server/src/openapi/mod.rs`

Remove OpenAPI path entries for the proxy routes.

### 6. Frontend: Remove `serverId` from MCP client
**File:** `src/lib/mcp-client.ts`

- Remove `serverId` from `McpClientConfig`
- Always connect to `http://localhost:${serverPort}/` (gateway)
- Remove the `serverId` branch in `getEndpointUrl()`

### 7. Frontend: Update Try It Out MCP tab
**File:** `src/views/try-it-out/mcp-tab/index.tsx`

- For "direct server" mode, instead of passing `serverId`, pass the server ID via `mcpAccess` header (e.g., `mcpAccess: directTarget.id` instead of `"all"`)
- Remove `isDirectServer` / `isGatewayTarget` distinctions — all modes go through gateway
- The endpoint URL display should always show the gateway URL

### 8. Check for mirror files in `src-tauri/`
The crate `lr-server` is used as a library. Check if `src-tauri/src/server/` has mirrored copies of route registrations, handler exports, and OpenAPI that also need updating.

## Verification
- `cargo test && cargo clippy && cargo fmt`
- Dev mode: all three Try It Out MCP modes (client, all, direct server, direct skill) should connect and list tools/resources via the gateway
- `/mcp/<id>` endpoints should return 404
