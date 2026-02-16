# Two-Mode App Integration: Try It Out vs Permanent Config

## Context

The current launcher system has `configure()` and `launch()` but their semantics are blurry — CLI tools return terminal commands from `launch()`, GUI apps spawn processes, and some tools permanently modify config files while others don't. The user wants two clearly distinct options:

1. **Try It Out** — Run the app once with LocalRouter. Nothing permanently changes. The user gets a terminal command to copy-paste.
2. **Permanent Config** — Modify the app's config files so it always routes through LocalRouter.

Additionally, several MCP configurations are incorrect based on actual tool documentation and need to be fixed.

---

## Per-App Capability Matrix

| Tool | Try It Out | Permanent Config | MCP in Try It Out | MCP in Permanent |
|------|-----------|-----------------|-------------------|-----------------|
| **Claude Code** | ✅ env vars | ✅ `~/.claude.json` (MCP only) | ✅ `--mcp-config` inline JSON | ✅ mcpServers |
| **Codex** | ✅ env vars | ✅ `~/.codex/config.toml` (MCP) | ❌ | ✅ TOML mcp_servers |
| **Aider** | ✅ env vars | ✅ `~/.aider.conf.yml` (LLM) | ❌ no MCP | ❌ no MCP |
| **Goose** | ✅ env vars | ✅ `~/.config/goose/config.yaml` (MCP) | ❌ no auth via CLI | ✅ extensions YAML |
| **OpenCode** | ❌ config-only | ✅ `opencode.json` (LLM + MCP) | — | ✅ `mcp` key, `type: remote` |
| **Droid** | ❌ config-only | ✅ `settings.json` + `mcp.json` | — | ✅ separate `~/.factory/mcp.json` |
| **OpenClaw** | ❌ config-only | ✅ `openclaw.json` (LLM) | — | ❌ no MCP |
| **Cursor** | ❌ GUI app | ✅ `settings.json` + `~/.cursor/mcp.json` | — | ✅ separate `~/.cursor/mcp.json` |

---

## MCP Format Corrections (from documentation research)

1. **Codex**: Remove fake `MCP_SERVERS` env var. Use `~/.codex/config.toml` TOML format for permanent MCP.
2. **Droid**: MCP goes in `~/.factory/mcp.json` (separate file), NOT in `settings.json`.
3. **OpenCode**: MCP key is `"mcp"` (not `"mcpServers"`), type is `"remote"` (not `"http"`).
4. **Cursor**: Add MCP support — write to `~/.cursor/mcp.json` (separate from `settings.json`).
5. **Goose**: Add MCP support — write to `~/.config/goose/config.yaml` under `extensions`.
6. **Claude Code try-it-out**: Use `--mcp-config '<inline_json>'` CLI flag for one-time MCP.

---

## Phase 1: Refactor Rust Trait

**File: `src-tauri/src/launcher/mod.rs`**

Replace `configure()` + `launch()` with:

```rust
pub trait AppIntegration: Send + Sync {
    fn name(&self) -> &str;
    fn check_installed(&self) -> AppStatus;

    fn supports_try_it_out(&self) -> bool { false }
    fn supports_permanent_config(&self) -> bool { false }

    /// One-time terminal command. No permanent file changes.
    fn try_it_out(&self, base_url: &str, client_secret: &str, client_id: &str)
        -> Result<LaunchResult, String>;

    /// Permanently modify config files to route through LocalRouter.
    fn configure_permanent(&self, base_url: &str, client_secret: &str, client_id: &str)
        -> Result<LaunchResult, String>;
}
```

Default implementations return `Err("not supported")`.

---

## Phase 2: Update All 8 Integrations

### Claude Code (`claude_code.rs`)
- `supports_try_it_out: true`, `supports_permanent_config: true`
- **try_it_out**: Terminal command with inline MCP JSON via `--mcp-config`:
  ```
  ANTHROPIC_BASE_URL=<url> ANTHROPIC_API_KEY=<secret> claude --mcp-config '{"mcpServers":{"localrouter":{"type":"http","url":"<url>","headers":{"Authorization":"Bearer <secret>"}}}}'
  ```
  `modified_files: []` — zero side effects.
- **configure_permanent**: Write MCP to `~/.claude.json` under `mcpServers.localrouter` (keep current logic). Message: "MCP configured. For LLM routing, use env vars at launch time."

### Codex (`codex.rs`)
- `supports_try_it_out: true`, `supports_permanent_config: true`
- **try_it_out**: `OPENAI_BASE_URL=<url>/v1 OPENAI_API_KEY=<secret> codex --oss` (LLM only, no MCP)
- **configure_permanent**: Write MCP to `~/.codex/config.toml` as hand-formatted TOML (no toml crate):
  ```toml
  [mcp_servers.localrouter]
  url = "<base_url>"
  headers = { Authorization = "Bearer <secret>" }
  ```
  Read existing file, find/replace or append the `[mcp_servers.localrouter]` section. Use `write_with_backup()`.

### Aider (`aider.rs`)
- `supports_try_it_out: true`, `supports_permanent_config: true`
- **try_it_out**: `OPENAI_API_BASE=<url>/v1 OPENAI_API_KEY=<secret> aider` (no MCP)
- **configure_permanent**: Write to `~/.aider.conf.yml` using `serde_yaml` (already a dependency):
  ```yaml
  openai-api-base: <url>/v1
  openai-api-key: <secret>
  ```
  Read existing YAML, merge keys, write back with backup.

### Goose (`goose.rs`)
- `supports_try_it_out: true`, `supports_permanent_config: true`
- **try_it_out**: `OPENAI_BASE_URL=<url>/v1 OPENAI_API_KEY=<secret> goose` (LLM only)
- **configure_permanent**: Write MCP extension to `~/.config/goose/config.yaml` using `serde_yaml`:
  ```yaml
  extensions:
    localrouter:
      type: streamable_http
      name: LocalRouter
      uri: <base_url>
      enabled: true
      headers:
        Authorization: "Bearer <secret>"
  ```
  Read existing YAML, merge under `extensions` key, write back with backup.

### OpenCode (`opencode.rs`)
- `supports_try_it_out: false`, `supports_permanent_config: true`
- **configure_permanent**: Same config path. **Fix MCP format**:
  - Key: `"mcp"` (not `"mcpServers"`)
  - Type: `"remote"` (not `"http"`)

### Droid (`droid.rs`)
- `supports_try_it_out: false`, `supports_permanent_config: true`
- **configure_permanent**: Write TWO files:
  1. LLM → `~/.factory/settings.json` (customModels, same as current)
  2. MCP → `~/.factory/mcp.json` (**new separate file**)
  Remove MCP write from `settings.json`. Both files use `write_with_backup()`.

### OpenClaw (`openclaw.rs`)
- `supports_try_it_out: false`, `supports_permanent_config: true`
- **configure_permanent**: Same as current. LLM only, no MCP. No format changes.

### Cursor (`cursor.rs`)
- `supports_try_it_out: false`, `supports_permanent_config: true`
- **configure_permanent**: Write TWO files:
  1. LLM → `settings.json` (same as current)
  2. MCP → `~/.cursor/mcp.json` (**new**)
  Remove the process spawning (`Command::new`, `Stdio` imports).

---

## Phase 3: Tauri Commands

**File: `src-tauri/src/ui/commands_clients.rs`**

New struct:
```rust
#[derive(Debug, Serialize)]
pub struct AppCapabilities {
    pub installed: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub supports_try_it_out: bool,
    pub supports_permanent_config: bool,
}
```

Replace commands:
- `check_app_installed` → `get_app_capabilities` (returns `AppCapabilities`)
- `configure_app` → `configure_app_permanent` (calls `integration.configure_permanent()`)
- `launch_app` → `try_it_out_app` (calls `integration.try_it_out()`)

**File: `src-tauri/src/main.rs`** — Update invoke_handler registrations.

---

## Phase 4: TypeScript Types

**File: `src/types/tauri-commands.ts`**

- Add `AppCapabilities` interface (replaces `AppStatus`)
- New param types: `GetAppCapabilitiesParams`, `TryItOutAppParams`, `ConfigureAppPermanentParams`
- Remove old: `CheckAppInstalledParams`, `ConfigureAppParams`, `LaunchAppParams`, `AppStatus`
- Keep `LaunchResult` as-is

---

## Phase 5: Frontend Template Updates

**File: `src/components/client/ClientTemplates.tsx`**

Update for apps that now have MCP support:
- `goose`: `supportsMcp: true`, `defaultMode: 'both'`
- `cursor`: `supportsMcp: true`, `defaultMode: 'both'`

---

## Phase 6: QuickSetupTab UI

**File: `src/components/client/HowToConnect.tsx`**

1. Fetch `AppCapabilities` from `get_app_capabilities` on mount
2. Replace Launch + Configure Only with two distinct sections:
   - **"Try It Out"** button (only if `supports_try_it_out`): calls `try_it_out_app`, shows terminal command. Subtitle: "One-time — no files modified"
   - **"Configure Permanently"** button (only if `supports_permanent_config`): calls `configure_app_permanent`, shows modified files. Subtitle: "Modifies config files"
3. When app only has permanent config, show just that as the primary button
4. Terminal command result: prominent copyable code block

---

## Phase 7: Demo Mock

**File: `website/src/components/demo/TauriMockSetup.ts`**

Replace old handlers with `get_app_capabilities`, `try_it_out_app`, `configure_app_permanent`.

---

## Phase 8: Tests

**File: `src-tauri/src/launcher/integrations/mod.rs`**

- Update tests to use new method names (`try_it_out`, `configure_permanent`)
- Add `test_capability_flags` — verify each integration returns correct bools
- Add `test_try_it_out_returns_terminal_command` — for claude-code, codex, aider, goose
- Update `test_config_file_integrations` to call `configure_permanent()`

---

## Implementation Order

1. Trait refactor (`mod.rs`)
2. All 8 integrations (fix MCP formats, implement new methods)
3. Tauri commands (`commands_clients.rs`) + `main.rs` registration
4. TypeScript types
5. Frontend templates (`ClientTemplates.tsx`)
6. Frontend UI (`HowToConnect.tsx`)
7. Demo mock
8. Tests
9. `cargo test && cargo clippy` + `npx tsc --noEmit`

---

## Verification

1. `cargo test -p localrouter --lib launcher` — all tests pass
2. `cargo clippy -p localrouter` — no warnings in launcher code
3. `npx tsc --noEmit` — TypeScript compiles
4. Create Claude Code client → Quick Setup shows both buttons, try-it-out returns inline MCP command
5. Create Cursor client → only "Configure Permanently" button, writes both settings.json and mcp.json
6. Demo site mock works with correct data shapes
