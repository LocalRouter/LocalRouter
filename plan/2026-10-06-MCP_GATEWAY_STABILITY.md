# MCP Gateway Stability & Permission Inheritance

Date: 2026-10-06

## Reported problems

1. A client with global MCP access `allow` and one server set to `off` still
   received that server's tools.
2. The client permission tree showed per-tool toggles for a bearer-token
   server but nothing for the browser-OAuth servers (brief spinner, then empty).
3. "Tool Responses Indexing" and "MCP Catalog Indexing" were coupled: enabling
   response indexing hid every upstream tool behind IndexSearch; the catalog
   toggle had no effect of its own.
4. `tools/call` failed with `Tool not found: <server>__<tool>` for tools the
   client had just listed.
5. The OAuth "Test" button gave no feedback; the OAuth card lived on the Info tab.

## Root causes

- **Allowed servers** were computed as "all servers" whenever
  `mcp_permissions.global` was enabled (routes `mcp.rs`, `mcp_ws.rs`, `chat.rs`,
  `responses.rs`, `pipeline.rs`, the preview command and the permission-change
  hook), ignoring per-server overrides and the server `enabled` flag.
- **Session churn**: `handle_initialize` narrowed `session.allowed_servers` to
  the servers that started; `get_or_create_session` compared that narrowed list
  with the full requested list, so any server failing to start (e.g. a globally
  disabled one) rebuilt the session on every request, wiping `tool_mapping` →
  `Tool not found`.
- `tools/call` only resolved tools mapped by a `tools/list` in the same session.
- **Expired OAuth tokens**: transports bake the bearer token into headers at
  connect time and never refreshed it; the permission-tree command reused the
  long-lived global transport and swallowed all errors.
- **Indexing coupling**: the catalog compression plan was computed whenever
  context management was on; `catalog_compression_enabled` was never read.
- tools/resources/prompts lists were never filtered by tool/resource/prompt
  permissions; `resources/read` and `prompts/get` were never permission-checked.

## Changes

- `McpPermissions::allowed_server_ids(servers)` — single source of truth used by
  all entry points (enabled servers whose resolved permission, or any child
  grant, is enabled).
- `GatewaySession.requested_servers` — the server set used for change
  detection; `allowed_servers` remains the started subset. The permission-change
  hook narrows both; widening rebuilds the session on the next request.
- Lists are filtered via `GatewaySession::visible_{tools,resources,prompts}`
  (permissions, then catalog deferral). `resources/read` / `prompts/get` enforce
  permissions. Mappings of servers that fail a refresh are kept.
- `tools/call` / `prompts/get` refresh the catalog once on a mapping miss.
- Every server failing a list call returns an empty list + `_meta.partial_failure`
  instead of an error (virtual tools stay available).
- Context management: `response_indexing_enabled` and
  `catalog_compression_enabled` are independent; search tools are exposed when
  either is on; catalog indexing/deferral only when catalog compression is on;
  response compression only when response indexing is on; turning catalog
  compression off mid-session stops deferral; tools excluded from indexing are
  never deferred.
- SSE transport: shared mutable headers + `AuthRefresher`; on 401 the manager
  re-resolves auth (forcing a browser-OAuth refresh) and retries once. Browser
  tokens refresh 60s before expiry. Duplicated auth code in `start_*_server`
  replaced by `apply_auth_headers`.
- `get_mcp_server_capabilities` reports `tools/list` failures, follows
  pagination and restarts a stale running server once. The permission tree
  shows per-server errors and parses keys at the first `__` only.
- `test_mcp_oauth_connection` obtains a real token (refreshing if needed);
  the Test button runs a live connection test with a toast; the OAuth card is
  on the Settings tab.
- Virtual-server `list_changed` notifications are keyed to the owning session
  (`session_notification_key`) and forwarded by the SSE and
  `subscriptions/listen` streams (previously always dropped).
- SSE-path gateway timeout: 45s for `initialize`, none for `tools/call`
  (firewall approval popups), 15s otherwise.
- Gateway synthetic client carries the real client id and `memory_folder`
  (memory tools searched a random folder).
- Skills access uses `has_any_access` / `has_any_enabled_for_skill`.
- Teardown waits (bounded) for the session lock instead of skipping busy
  sessions (leaked stdio processes); no DashMap ref held across awaits; WS
  forwarders survive broadcast lag.

## Tests

- `lr-config`: allowed-server resolution (incl. global allow + server off +
  disabled server), resolution fallbacks, independent inheritance of both
  indexing flags, skills access.
- `lr-mcp` session: permission-filtered lists, catalog gating, ineligible tools
  never deferred, mapping survival on failure, requested-server stability.
- `lr-mcp` context mode: flags independent in create/update.
- `lr-mcp` SSE: 401 → refresh → retry once; no refresher; failed refresh.
- `src-tauri/tests/mcp_gateway_stability_tests.rs`: end-to-end against
  wiremock servers (session reuse with a failing server, tools/call without
  tools/list, tool/prompt permissions, all-servers-failing, indexing toggles).

## Known remaining gaps

- Upstream server notifications are broadcast per server id, not per session:
  clients sharing a server can receive each other's progress/log notifications.
- `virtual_indexing` settings are not applied (virtual tool responses are never
  compressed).
- Permission changes to marketplace/coding agents/memory/context management do
  not emit `list_changed`; a session whose lock is busy during the change is
  updated on its next request without a notification.
