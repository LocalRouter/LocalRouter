# Plan: Notify Connected MCP Clients on Permission Changes

## Context

When a user changes MCP/skills permissions in the UI (e.g., toggling a tool from Allow to Off), the config is saved and the UI refreshes, but connected MCP clients are **not notified**. Clients must re-connect to see updated permissions.

Instead of manually adding notification calls to each Tauri permission command (fragile), we'll **listen for the `clients-changed` Tauri event** and automatically detect which clients' effective permissions changed by comparing snapshots. This is robust — any future command that modifies client config automatically triggers the check.

## Approach

1. **Store permission snapshots** in `GatewaySession` (MCP permissions + skills permissions)
2. **Listen for `clients-changed`** event in `main.rs` (already emitted by all permission commands)
3. **Compare snapshots** with current config — if different, invalidate caches + send notifications
4. **Deliver via per-client broadcast channel** that SSE and WS handlers subscribe to

## Files to Modify

### 1. `crates/lr-mcp/src/gateway/session.rs` — Store MCP permission snapshot

Add field to `GatewaySession` (alongside existing `skills_permissions` at line 82):
```rust
pub mcp_permissions: lr_config::McpPermissions,
```

Update `GatewaySession::new()` to initialize it with `Default::default()`.

### 2. `crates/lr-mcp/src/gateway/gateway.rs` — Update snapshot on request + add public methods

**In `handle_request_with_skills()`** (around line 340-356): Also update the stored `mcp_permissions` snapshot on the session, similar to how `skills_permissions` is already updated. We need the caller to pass in the client's current `McpPermissions`. (But actually — we can just store the `allowed_servers` + the full `McpPermissions` and update them in the same place skills_permissions is updated.)

Actually, simpler: pass `mcp_permissions` into `handle_request_with_skills()` and store it on the session. This is used purely for change detection later.

**Add public methods** (after line ~1409, matching existing `invalidate_tools_cache` pattern):
- `pub fn invalidate_prompts_cache(&self, client_id: &str)`
- `pub fn invalidate_all_caches(&self, client_id: &str)`
- `pub fn check_and_notify_permission_changes(&self, clients: &[lr_config::Client], notify: impl Fn(&str, bool, bool, bool))` — for each active session, compare stored permissions with the matching client's current permissions. If different, invalidate caches, update snapshot, call `notify` callback with `(client_id, tools_changed, resources_changed, prompts_changed)`.

The `check_and_notify_permission_changes` logic:
```
for each active session:
  find matching client in `clients` by id
  if not found → skip (client may have been deleted)

  compute new_allowed_servers from client.mcp_permissions (same logic as mcp.rs lines 500-513)

  old_mcp = session.mcp_permissions
  old_skills = session.skills_permissions
  new_mcp = client.mcp_permissions
  new_skills = client.skills_permissions

  if old_mcp != new_mcp || old_skills != new_skills:
    // Determine what changed
    tools_changed = old_mcp.global != new_mcp.global
                 || old_mcp.servers != new_mcp.servers
                 || old_mcp.tools != new_mcp.tools
                 || old_skills != new_skills
    resources_changed = old_mcp.global != new_mcp.global
                     || old_mcp.servers != new_mcp.servers
                     || old_mcp.resources != new_mcp.resources
    prompts_changed = old_mcp.global != new_mcp.global
                   || old_mcp.servers != new_mcp.servers
                   || old_mcp.prompts != new_mcp.prompts

    invalidate relevant caches
    update stored snapshots
    call notify(client_id, tools_changed, resources_changed, prompts_changed)
```

All permission types (`McpPermissions`, `SkillsPermissions`, `PermissionState`, etc.) already derive `PartialEq`.

### 3. `crates/lr-server/src/state.rs` — Add per-client notification broadcast

**New field** on `AppState` (alongside `mcp_notification_broadcast` at line 404):
```rust
pub client_notification_broadcast: Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,
```

**Initialize** in `AppState::new()` (around line 458):
```rust
let (client_notification_tx, _) = tokio::sync::broadcast::channel(100);
```

### 4. `crates/lr-server/src/routes/mcp.rs` — SSE subscribes to per-client channel

- Subscribe: `let mut client_notification_rx = state.client_notification_broadcast.subscribe();` (around line 204)
- Add third `tokio::select!` branch in the SSE stream loop that filters by `target_client_id == client_id` and yields as SSE `message` event

### 5. `crates/lr-server/src/routes/mcp_ws.rs` — WS subscribes to per-client channel

- Subscribe: `let mut client_notification_rx = state.client_notification_broadcast.subscribe();` (around line 132)
- Add third `tokio::select!` branch in Task 1 (forward task) that filters by `target_client_id == client_id_forward` and sends as WS text message

### 6. `crates/lr-server/src/routes/mcp.rs` — Pass `mcp_permissions` to gateway

In `mcp_gateway_handler()` (around line 658), pass the client's `mcp_permissions` to `handle_request_with_skills()` so the session snapshot stays up-to-date on each request.

### 7. `src-tauri/src/main.rs` — Register `clients-changed` event listener

After `AppState` is managed (around line 530), register an event listener:

```rust
let app_handle_for_clients = app.handle().clone();
app.listen("clients-changed", move |_event| {
    if let Some(app_state) = app_handle_for_clients.try_state::<Arc<lr_server::state::AppState>>() {
        let config = config_manager_for_notify.get();
        let all_server_ids: Vec<String> = config.mcp_servers.iter()
            .filter(|s| s.enabled)
            .map(|s| s.id.clone())
            .collect();

        let broadcast = app_state.client_notification_broadcast.clone();
        app_state.mcp_gateway.check_and_notify_permission_changes(
            &config.clients,
            &all_server_ids,
            |client_id, tools, resources, prompts| {
                use lr_mcp::gateway::streaming_notifications::StreamingNotificationType;
                if tools {
                    let _ = broadcast.send((client_id.to_string(),
                        StreamingNotificationType::ToolsListChanged.to_notification()));
                }
                if resources {
                    let _ = broadcast.send((client_id.to_string(),
                        StreamingNotificationType::ResourcesListChanged.to_notification()));
                }
                if prompts {
                    let _ = broadcast.send((client_id.to_string(),
                        StreamingNotificationType::PromptsListChanged.to_notification()));
                }
            },
        );
    }
});
```

No changes needed to any Tauri permission commands — the event listener catches all of them.

## Verification

1. `cargo test` — existing tests pass
2. `cargo clippy` — lint check
3. Manual test: Connect an MCP client → toggle a tool/resource/prompt permission in UI → verify client receives notification and refreshes
4. Check logs for notification messages
5. Verify that changing global/server-level permissions sends all three notification types
6. Verify that changing only a tool permission sends only `tools/list_changed`
