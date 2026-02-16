# Auto-Sync Client Config to External Apps

## Context

The "Configure Permanently" feature writes config files for external apps (OpenCode, Cursor, Droid, etc.) as a one-shot action. When models change, secrets rotate, or strategies update, those external configs go stale. This plan replaces the one-shot button with an auto-sync toggle that keeps external app configs in sync automatically.

Key decisions:
- Toggle **replaces** the "Configure Permanently" button (not both)
- Disabling sync **does not** clean up config files (leaves last-written state)

---

## Files to Modify

### Backend (Rust)
1. `crates/lr-config/src/types.rs` — add `sync_config` field to `Client`
2. `src-tauri/src/launcher/mod.rs` — add `ConfigSyncContext` struct + `sync_config()` trait method
3. `src-tauri/src/launcher/integrations/opencode.rs` — override `sync_config()` with model list
4. `src-tauri/src/ui/commands_clients.rs` — add `toggle_client_sync_config`, `sync_client_config` commands + `sync_all_clients` helper; add `sync_config` to `ClientInfo`; update `rotate_client_secret` to trigger sync
5. `src-tauri/src/main.rs` — register new commands in `invoke_handler`; add debounced sync event listeners

### Frontend (TypeScript/React)
6. `src/types/tauri-commands.ts` — add `sync_config` to `ClientInfo`, add param types
7. `src/components/client/HowToConnect.tsx` — replace "Configure Permanently" button with sync toggle in `QuickSetupTab`
8. `website/src/components/demo/TauriMockSetup.ts` — add mock handlers

---

## Implementation

### 1. Add `sync_config` to Client config (`types.rs:1447`)

```rust
/// Auto-sync external app config files when models/secrets/config change.
/// Only effective when template_id is set.
#[serde(default)]
pub sync_config: bool,
```

After `template_id`. Defaults `false`, existing configs migrate cleanly.

### 2. Extend AppIntegration trait (`launcher/mod.rs`)

Add context struct and new trait method with default impl that delegates to `configure_permanent`:

```rust
pub struct ConfigSyncContext {
    pub base_url: String,
    pub client_secret: String,
    pub client_id: String,
    /// Model IDs available to this client (e.g. "anthropic/claude-sonnet-4-20250514")
    pub models: Vec<String>,
}
```

Add to `AppIntegration` trait:
- `fn needs_model_list(&self) -> bool { false }` — only OpenCode returns true
- `fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String>` — default calls `configure_permanent(ctx.base_url, ctx.client_secret, ctx.client_id)`

### 3. OpenCode `sync_config()` override (`integrations/opencode.rs`)

Override `needs_model_list() -> true` and `sync_config()` to:
- Build `models` map from `ctx.models` as `{ "model-id": { "name": "model-id" } }`
- Include `apiKey` in provider `options`
- Include `/v1` suffix on `baseURL`
- Same MCP entry as current `configure_permanent`

Also update the existing `configure_permanent` to include `apiKey` and `/v1` (align with sync_config).

### 4. Backend commands + helpers (`commands_clients.rs`)

**Add `sync_config` to `ClientInfo` struct** (line 49, after `template_id`):
```rust
pub sync_config: bool,
```
And populate it in `list_clients` mapping (line 75).

**Add public helper** (non-Tauri, callable from event listeners):
```rust
pub async fn sync_client_config_inner(
    client_id: &str,
    config_manager: &ConfigManager,
    client_manager: &Arc<lr_clients::ClientManager>,
    provider_registry: &Arc<lr_providers::registry::ProviderRegistry>,
) -> Result<Option<LaunchResult>, String>
```
Logic:
1. Get client from config, check `sync_config == true` and `template_id.is_some()`
2. Get integration via `launcher::get_integration(template_id)`
3. Get secret from `client_manager.get_secret()`
4. Build `base_url` from `config.server.host/port`
5. If `integration.needs_model_list()`: get strategy, call `provider_registry.list_all_models().await`, filter by `strategy.is_model_allowed()`, format as `"{provider}/{model_id}"`
6. Build `ConfigSyncContext`, call `integration.sync_config(&ctx)`

**Add `sync_all_clients` helper:**
```rust
pub async fn sync_all_clients(
    config_manager: &ConfigManager,
    client_manager: &Arc<lr_clients::ClientManager>,
    provider_registry: &Arc<lr_providers::registry::ProviderRegistry>,
)
```
Iterates all clients with `sync_config == true && template_id.is_some()`, calls `sync_client_config_inner` for each, logs warnings on failure.

**Add Tauri commands:**

`toggle_client_sync_config(client_id, enabled)`:
- Updates `client.sync_config` in config, saves
- If enabling: calls `sync_client_config_inner` immediately, returns `Option<LaunchResult>`
- Emits `"clients-changed"`

`sync_client_config(client_id)`:
- Manual trigger, calls `sync_client_config_inner`

**Update `rotate_client_secret`:**
- Add `config_manager` and `provider_registry` State params
- After rotation, check if client has `sync_config == true`, if so spawn `sync_client_config_inner`

### 5. Event listeners with debounce (`main.rs`)

In `.setup()`, after existing listener setup:

Create `tokio::sync::mpsc::channel::<()>(16)` for sync signals.

Spawn a debounced sync task:
```
loop {
    recv -> drain queue -> sleep 500ms -> drain again -> sync_all_clients()
}
```

Register listeners on existing events:
- `"models-changed"` — provider model list changed
- `"strategies-changed"` — strategy routes changed

Each just does `tx.try_send(())`.

Register new commands in `invoke_handler`: `toggle_client_sync_config`, `sync_client_config`.

### 6. TypeScript types (`tauri-commands.ts`)

Add to `ClientInfo`:
```typescript
sync_config: boolean
```

Add param interfaces:
```typescript
export interface ToggleClientSyncConfigParams { clientId: string; enabled: boolean }
export interface SyncClientConfigParams { clientId: string }
```

### 7. Frontend UI (`HowToConnect.tsx`)

In `QuickSetupTab`, the component needs `sync_config` state. It should receive it as a prop (passed from parent which has client info), or fetch it from the client list.

**Replace the "Configure Permanently" button** (lines 295-311) with:

- A Switch/toggle labeled "Keep config in sync"
- When toggled on: calls `toggle_client_sync_config` with `enabled: true`
  - Shows result (modified files) on success
- When toggled off: calls `toggle_client_sync_config` with `enabled: false`
- When on: show subtle "Synced" indicator + small refresh button for manual re-sync
- Keep "Try It Out" button unchanged

The parent (`HowToConnect`) needs to pass down `syncConfig` boolean. It should come from client info. Check where `HowToConnect` is rendered and ensure `ClientInfo` is available.

### 8. Demo mock (`TauriMockSetup.ts`)

Add handlers for `toggle_client_sync_config` and `sync_client_config` returning mock `LaunchResult`.

---

## Which templates benefit from model syncing?

| Template | Needs model list? | Why |
|----------|:-:|---|
| **OpenCode** | Yes | No auto-discovery; models must be listed in config |
| Droid | No | Uses single `localrouter` model entry, auto-routes |
| Cursor | No | Only stores baseURL + apiKey |
| Claude Code | No | Env vars + MCP config, no model list |
| Others | No | Env vars or manual setup |

Only OpenCode overrides `sync_config()`. All other integrations benefit from secret/URL sync via the default delegation to `configure_permanent`.

---

## Verification

1. `cargo test` — existing integration tests in `launcher/integrations/mod.rs` should pass
2. `cargo clippy` + `npx tsc --noEmit` — no warnings
3. Manual test flow:
   - Create client with OpenCode template
   - Enable sync toggle → verify `opencode.json` is written with models + apiKey + MCP
   - Add/remove a provider model → verify `opencode.json` updates within ~1s
   - Rotate client secret → verify new secret appears in `opencode.json`
   - Disable sync toggle → verify changes no longer propagate
4. Test with other templates (Cursor, Claude Code) — sync toggle writes config, secret rotation updates it
