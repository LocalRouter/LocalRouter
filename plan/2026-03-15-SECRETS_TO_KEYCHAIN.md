# Plan: Move Secrets from Config to Keychain

## Context

Two categories of secrets are stored in **plaintext** in the YAML config file:

1. **Provider API keys** — stored inside the `provider_config` JSON blob when a provider is created
2. **MCP CustomHeaders & EnvVars** — header values (which may contain auth tokens) and env var values (which may contain API keys) stored directly in the `McpAuthConfig` enum variants

The keychain infrastructure (`lr-api-keys`) and provider `key_storage` module already exist — they just need to be wired in.

---

## Part A: Provider API Keys

### A1. `crates/lr-config/src/types.rs`
- Add `pub const PROVIDER_KEYRING_SERVICE: &str = "LocalRouter-Providers";`
- Bump `CONFIG_VERSION` from 22 to 23

### A2. `crates/lr-providers/src/key_storage.rs`
- Replace local `KEYRING_SERVICE` constant with `lr_config::PROVIDER_KEYRING_SERVICE`

### A3. `crates/lr-config/src/migration.rs`
- Add `migrate_to_v23()`: For each provider, extract `api_key` from `provider_config` JSON blob, store in keychain, remove from JSON. Also migrate MCP CustomHeaders/EnvVars (see B3). If keychain store fails for any entry, put value back so it's not lost.

### A4. `src-tauri/src/main.rs` (lines ~278-298)
- After building `config_map` from `provider_config` JSON, if `api_key` is not in the map, load from keychain via `key_storage::get_provider_key()` and inject it
- Handles both post-migration and legacy cases

### A5. `src-tauri/src/ui/commands_providers.rs`
- **`create_provider_instance`**: Pass full config to registry (in-memory), then extract `api_key`, store in keychain, strip before saving to disk
- **`update_provider_instance`**: Same pattern
- **`remove_provider_instance`**: Also delete keychain entry (best-effort)
- **`rename_provider_instance`**: Migrate keychain entry old→new name

### Files NOT changed for Part A
- Frontend, TypeScript types, demo mock — command signatures unchanged
- Factory code (`factory.rs`) — still reads `config.get("api_key")` from in-memory HashMap
- Registry (`registry.rs`) — still stores full config in-memory

---

## Part B: MCP CustomHeaders & EnvVars

### B1. `crates/lr-config/src/types.rs` — Change `McpAuthConfig` enum
Keep HashMap structure but values become keychain references instead of actual secrets:

```rust
// Before:
CustomHeaders { headers: HashMap<String, String> }  // key=header name, value=actual secret
EnvVars { env: HashMap<String, String> }             // key=env var name, value=actual secret

// After:
CustomHeaders { header_refs: HashMap<String, String> }  // key=header name, value=keychain account key
EnvVars { env_refs: HashMap<String, String> }            // key=env var name, value=keychain account key
```

Each value stored in keychain (service `"LocalRouter-McpServers"`) under a generated UUID as the account key. The HashMap value in config IS the UUID reference used for keychain lookup.

### B2. `src-tauri/src/ui/commands_mcp.rs` — `process_auth_config()`
Currently these pass through directly. Change to:
- **CustomHeaders**: For each `(name, value)` in the HashMap, generate a UUID ref, store value in keychain under that ref, build `header_refs` HashMap with `name → ref`
- **EnvVars**: Same pattern — generate UUID ref per entry, store value in keychain, build `env_refs` HashMap with `name → ref`

### B3. `crates/lr-config/src/migration.rs` — Include in `migrate_to_v23()`
For each MCP server with `CustomHeaders` or `EnvVars` auth:
- For each entry in the HashMap, generate UUID ref, store value in keychain, replace value with the ref
- Rename the variant field from `headers`→`header_refs` / `env`→`env_refs`

### B4. `crates/lr-mcp/src/manager.rs` — Runtime keychain reads
Update the 3 locations that consume these auth types:
- **STDIO `start_stdio_server`** (~line 423): Iterate `env_refs`, look up each reference value from keychain, build resolved HashMap, merge into subprocess env
- **SSE `start_sse_server`** (~line 477): Iterate `header_refs`, look up each reference value from keychain, build resolved HashMap, merge into request headers
- **WebSocket handler** (~line 660): Same as SSE for headers

Pattern matches existing BearerToken code (lines 464-475) which already reads from keychain.

### B5. MCP server deletion/update
- In `commands_mcp.rs` delete/update handlers: clean up old keychain entries for each header/env var (iterate refs, delete each keychain entry)

---

## Verification

1. `cargo test -p lr-config` — migration tests
2. `cargo test -p lr-providers` — key_storage tests
3. `cargo clippy` — no warnings
4. Manual: create provider, check YAML has no `api_key`, restart app, provider still works
5. Manual: create MCP server with CustomHeaders, check YAML has only `headers_ref`, restart, server connects
