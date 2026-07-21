# Plan: Two-Dimension Client Mode + Hand-Rolled HTTPS Inspection Proxy

## Context

Today a client has a single `ClientMode` (`Both | LlmOnly | McpOnly | McpViaLlm`,
`crates/lr-config/src/types.rs:2715`) that conflates two independent axes — LLM
access and MCP access. We want to (a) split these into an **LLM section** and an
**MCP section** in both config and UI, and (b) add a new **HTTPS inspection proxy**
so tools like Claude Code can point `HTTPS_PROXY` at LocalRouter and have their
traffic passively inspected in the Monitor without being rewritten.

The proxy's *passive* mode is a transparent MITM pass-through: LocalRouter
terminates TLS (using a root CA the client trusts), parses the request/response
for monitoring, and forwards the bytes **unchanged** to the real upstream
(e.g. `api.anthropic.com`) — the client's own auth flows straight through. This
is the ToS-compatible "transparent corporate proxy" posture (verified against
Anthropic's network-config docs + Consumer Terms). A later *active* mode (out of
scope here, but designed for) will rewrite requests: model allow-listing / forced
rewrite, JSON optimization, etc.

### Locked design decisions (from user)
- **Root CA:** one shared LocalRouter Proxy Root CA (installed per client). Not per-client CAs.
- **Client identity:** Proxy-Authorization basic-auth (client secret) for v1; design the identity layer behind a trait so mTLS client-cert (`CLAUDE_CODE_CLIENT_CERT`) can be added later.
- **Implementation:** hand-rolled MITM proxy on `hyper` 1.x + `tokio-rustls` + `rcgen` (no `hudsucker`). New `lr-proxy` crate. All inspection/rewrite behind a `ProxyInterceptor` trait.
- **Active proxy mode:** present in the enum + UI but **disabled / not implemented** (button disabled, backend rejects).

### Verified external facts
- Claude Code network config (`https://code.claude.com/docs/en/network-config.md`), all settable via `env` block in `~/.claude/settings.json`:
  - Proxy: `HTTPS_PROXY` (supports `http://user:pass@host:port` basic auth), `HTTP_PROXY`, `NO_PROXY`. **No SOCKS.**
  - Trust our MITM root: `NODE_EXTRA_CA_CERTS=/path/to/root-ca.pem`.
  - Future mTLS: `CLAUDE_CODE_CLIENT_CERT`, `CLAUDE_CODE_CLIENT_KEY`, `CLAUDE_CODE_CLIENT_KEY_PASSPHRASE`.
- Workspace already has `rustls 0.23`, `tokio-rustls 0.26`, `rcgen 0.14`, `hyper 1.x`, `hyper-util` transitively in `Cargo.lock` — none wired for serving yet.
- Server is currently **plain HTTP, single Axum listener, OpenAI-format inbound only** (`crates/lr-server/src/lib.rs`). Claude Code speaks the **Anthropic Messages** wire format (`/v1/messages`) to `api.anthropic.com`, which the existing routes do not parse — so the proxy must NOT reuse the OpenAI router; it forwards to the real upstream and parses Anthropic format itself for monitoring.

---

## Part A — Split ClientMode into LLM + MCP dimensions

### New enums (`crates/lr-config/src/types.rs`, near line 2715)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmMode {
    Off,                 // no LLM access
    #[default] Gateway,  // native /v1 endpoints (today's "on")
    ProxyInspect,        // NEW: passive HTTPS proxy (inspect only)
    ProxyRewrite,        // NEW: active proxy (rewrite) — NOT IMPLEMENTED (reject/deny)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpMode {
    Off,
    #[default] Gateway,  // direct MCP proxy (today's "on")
    ViaLlm,              // tools injected into LLM chat, executed server-side
}
```
Names are the recommendation; final UI labels in Part F. `ProxyInspect` = user's
"Proxy LLM Passive"; `ProxyRewrite` = "Proxy LLM Active".

### Client struct
Add `#[serde(default)] pub llm_mode: LlmMode` and `#[serde(default)] pub mcp_mode: McpMode`
to `Client` (`types.rs:2736`). Keep the old field as a **deserialize-only shim**:
`#[serde(default, skip_serializing)] client_mode: Option<ClientMode>` — read by the
migration; never written back. Keep the `ClientMode` enum (do not delete variants —
per repo policy) for the migration mapping only.

### Helper methods on `Client` (centralize the branching; used by gates)
- `llm_gateway_enabled() -> bool` = `llm_mode == Gateway`
- `llm_proxy_mode() -> Option<ProxyMode>` = `ProxyInspect | ProxyRewrite`
- `mcp_direct_enabled() -> bool` = `mcp_mode == Gateway`
- `is_mcp_via_llm() -> bool` = `mcp_mode == ViaLlm`

### Validity rules (`crates/lr-config/src/validation.rs`)
Reject on load/save:
- `llm_mode == Off && mcp_mode == Off` (nothing enabled).
- `mcp_mode == ViaLlm && llm_mode != Gateway` (Via-LLM needs the native chat path; explicitly **incompatible with proxy passive**; active may relax later).
- `llm_mode == ProxyRewrite` → not implemented: reject with a clear "not yet available" error (UI also disables it).

### Rewire the gates (behavior-preserving)
- `crates/lr-server/src/routes/helpers.rs` — `check_llm_access_with_state` (226) allow iff `llm_gateway_enabled()`; `check_mcp_access_with_state` (311) allow iff `mcp_direct_enabled()`; Via-LLM message unchanged. Error codes preserved where possible.
- `crates/lr-server/src/routes/chat.rs:145-160` and `responses.rs:235` — trigger `handle_mcp_via_llm` on `is_mcp_via_llm()` instead of `== McpViaLlm`.
- MCP sampling passthrough `mcp.rs:834` (`Both | McpOnly` → forward): becomes `mcp_direct_enabled()`.
- MCP gateway session mode plumbing (`crates/lr-mcp/src/gateway/session.rs:119`, `lr-mcp-via-llm/*`): carry `McpMode` (or a derived bool) instead of `ClientMode`.

### Old→new mapping (used by migration + the shim)
`Both → (Gateway, Gateway)` · `LlmOnly → (Gateway, Off)` · `McpOnly → (Off, Gateway)` · `McpViaLlm → (Gateway, ViaLlm)`.

---

## Part B — Hand-rolled HTTPS inspection proxy (`crates/lr-proxy`, new crate)

A forward proxy: `HTTPS_PROXY=http://<client_id>:<secret>@127.0.0.1:<proxy_port>`.
Plain-TCP listener; client sends an HTTP `CONNECT host:443` (with
`Proxy-Authorization`), we tunnel, and MITM only allow-listed LLM hosts.

**Deps (new, direct):** `hyper` 1.x + `hyper-util`, `tokio-rustls` 0.26, `rustls` 0.23,
`rcgen` 0.14, `rustls-native-certs` (or `webpki-roots`) for upstream validation,
`base64`, `tokio`, `bytes`, `http`. (All already resolve in `Cargo.lock`.)

### Connection flow (per accepted TCP conn)
1. **Parse `CONNECT`** line + headers. Extract target `host:port` and
   `Proxy-Authorization: Basic base64(client_id:secret)`.
2. **Authenticate** via existing `state.client_manager.verify_secret(...)`
   (`crates/lr-clients/src/manager.rs`) → resolve `client_id`. On failure →
   `407 Proxy Authentication Required`. Resolve the client's `llm_mode`.
3. **Decide** via `ProxyInterceptor::on_connect(host, client)`:
   - Host **not** in LLM allow-list, OR client not in a proxy `llm_mode` →
     **blind tunnel**: reply `200 Connection Established`, then splice bytes
     bidirectionally (`tokio::io::copy_bidirectional`). No decryption. (Critical:
     `claude.ai` / auth / telemetry hosts are always blind-tunneled.)
   - Otherwise → **MITM** (below).
4. **MITM**: reply `200`, then:
   - Look up / mint a **leaf cert** for `host` signed by the shared root CA
     (rcgen), cached in-memory by host. Build a `rustls::ServerConfig` that
     **offers only ALPN `http/1.1`** (forces HTTP/1.1 client-side — avoids
     HTTP/2 framing; Anthropic API works over 1.1). `TlsAcceptor` wraps the
     client socket.
   - Open **upstream** TLS to the real `host:443` via `TlsConnector` with the
     real system/webpki roots (we still validate the genuine upstream cert).
   - Run a **hyper server** (HTTP/1.1) on the decrypted client side; for each
     request, drive a **hyper client** to the upstream connection, ferrying
     request → upstream and streaming response ← upstream.
5. **Tap** (passive): for each request/response, hand the parsed head + a
   **teed, size-capped copy** of the body to the interceptor (Part D). Body bytes
   pass through unbuffered to the peer; monitoring gets a bounded clone
   (streaming SSE parsed incrementally).

### The interceptor abstraction (enables active mode later)
```rust
pub enum ConnectDecision { Mitm, Tunnel, Reject(StatusReason) }
pub enum RequestAction  { Forward, Replace(http::Request<Body>) } // passive: always Forward
pub enum ResponseAction { Forward, Replace(http::Response<Body>) }

#[async_trait]
pub trait ProxyInterceptor: Send + Sync {
    fn on_connect(&self, host: &str, client: &ClientCtx) -> ConnectDecision;
    async fn on_request(&self, ctx: &mut ProxyReqCtx)  -> RequestAction;
    async fn on_response(&self, ctx: &mut ProxyResCtx) -> ResponseAction;
}
```
- **v1 `PassiveInterceptor`**: `on_connect` = Mitm for allow-listed LLM hosts else Tunnel; `on_request`/`on_response` record to Monitor then `Forward`. Never mutates.
- **Future `ActiveInterceptor`**: same trait; returns `Replace(...)` for model rewrite / JSON optimization / allow-list enforcement. No transport changes needed.

### Wiring / lifecycle
- `ProxyManager` (mirror `crates/lr-server/src/manager.rs`) owns the listener; `start`/`stop`/`get_actual_port`.
- Started from `src-tauri/src/launcher` alongside the server; **auto-enabled when any client has a proxy `llm_mode`** (else not bound). Config: add `ProxyConfig { enabled, host=127.0.0.1, port }` to `crates/lr-config/src/types.rs` (default port e.g. 33626 debug / 3626 release, mirroring the server's 33625/3625 pattern at `types.rs:3650`).
- **Host allow-list** for MITM (const, extensible): `api.anthropic.com` (+ later `api.openai.com`, etc.). Everything else tunnels.

---

## Part C — Certificates & identity

New module (in `lr-proxy`, e.g. `cert.rs`):
- **Root CA:** generate once with rcgen (CA cert + key). Store at
  `config_dir()/proxy/root-ca.pem` + `root-ca.key` (reuse `lr_utils::paths::config_dir()`
  + `ensure_dir_exists`, `crates/lr-utils/src/paths.rs`), key file `0600`.
  (Option noted for later: store key in keychain via `lr-api-keys`.) Idempotent:
  reuse if present.
- **Leaf signing:** `sign_leaf(host) -> (cert_chain, key)` signed by root, cached.
- Expose `root_ca_pem_path()` for setup flows (→ `NODE_EXTRA_CA_CERTS`).
- **Identity trait** (v1 basic-auth): `ProxyIdentity::from_connect(headers) -> Option<ClientId>` reading `Proxy-Authorization`. mTLS impl added later behind the same trait (validates `CLAUDE_CODE_CLIENT_CERT` against a client-cert CA).
- **Secret redaction:** before recording, strip `Authorization` / `Proxy-Authorization` / `x-api-key` headers and known token fields from the captured copy. We inspect but never persist upstream credentials (privacy-policy requirement).

---

## Part D — Monitoring integration

Reuse the live event store — `MonitorEventStore::push` / `.update`
(`crates/lr-monitor/src/store.rs:42/91`) take parsed JSON + metadata, so the proxy
parses then pushes directly (no `AppState`/`ClientAuthContext` coupling).

- Extend `MonitorEventData::LlmCall` (`crates/lr-monitor/src/types.rs:174`) with markers: `source: LlmCallSource { Api, Proxy }` and `protocol: { Openai, Anthropic }` (both default to today's Api/Openai for existing callers).
- **Anthropic parsing (new, in `lr-proxy`):** request → `model`, `stream`, `message_count`, `has_tools`, redacted `request_body`. Response → `status_code`, usage `input_tokens`/`output_tokens`, and for streaming SSE reconstruct `content_preview` from `message_start`/`content_block_delta`/`message_delta`(usage)/`message_stop`. Reuse Anthropic request/response types from `crates/lr-providers/src/anthropic.rs` where convenient (types only — not the outbound client).
- **Cost:** compute `cost_usd` from tokens via existing catalog pricing (`lr-catalog`), same as native path.
- Frontend Monitor view: show a "via proxy" badge (source marker); passive rows are read-only (no re-run).

---

## Part E — Client setup (manual + Claude automated)

Extend the existing `AppIntegration` system (`src-tauri/src/launcher/mod.rs:44`,
`integrations/claude_code.rs`) — do **not** invent a parallel mechanism.

- `ConfigSyncContext` (`launcher/mod.rs:16`): add `proxy_url`, `ca_cert_path`, and the LLM/MCP modes so integrations know when to emit proxy config.
- **Claude Code automated setup** (`integrations/claude_code.rs`): when `llm_mode == ProxyInspect`, write `~/.claude/settings.json` `env` block (this is the *settings.json* the network-config docs describe — distinct from the existing `~/.claude.json` MCP writer, which stays for MCP modes), via `launcher/backup.rs::write_with_backup` (atomic + backup):
  ```json
  { "env": {
      "HTTPS_PROXY": "http://<client_id>:<secret>@127.0.0.1:<proxy_port>",
      "NODE_EXTRA_CA_CERTS": "<config_dir>/proxy/root-ca.pem"
  } }
  ```
  Merge (don't clobber) existing `env`. `sync_config` removes these keys when the client leaves proxy mode.
- **Manual instructions** (`src/components/client/ClientTemplates.tsx` + `HowToConnect.tsx`): a new "HTTPS Proxy" connection panel showing proxy URL (with embedded basic-auth secret), the CA cert path to trust, and the env-var / settings.json snippet. Add a Tauri command to reveal/copy the root-CA path (reuse `get_config_dir`).
- New/renamed Tauri commands (update all 4 layers per `CLAUDE.md`): replace `set_client_mode` (`src-tauri/src/ui/commands_clients.rs:2993`) with `set_client_llm_mode` + `set_client_mcp_mode` (or one `set_client_modes`); extend `create_client`. Update `src/types/tauri-commands.ts` and `website/src/components/demo/TauriMockSetup.ts` mocks.

---

## Part F — Frontend (two-section selector + gating)

- **`src/components/client/ClientModeSelector.tsx`**: replace the single 4-option control with **two labeled sections**:
  - **LLM:** Off · Gateway · Passive Proxy (inspect) · Active Proxy (rewrite — **disabled**, "coming soon").
  - **MCP:** Off · Gateway · Via LLM.
  - Cross-constraints in the UI: selecting a proxy LLM mode disables **Via LLM** (and vice-versa); disallow Off+Off; keep template capability gating (`isModeAllowed`).
- **Types:** `src/types/tauri-commands.ts:183` — replace `ClientMode` union with `LlmMode` (`'off'|'gateway'|'proxy_inspect'|'proxy_rewrite'`) + `McpMode` (`'off'|'gateway'|'via_llm'`); update `ClientInfo`, `CreateClientParams`, new set-mode params.
- **Tab / feature gating** (`src/views/clients/client-detail.tsx:67`, `info-tab.tsx`, `config-tab.tsx`): Models tab, per-client optimizations, and **Try-It-Out LLM** require `llm_mode == Gateway` — **hidden in proxy modes** (we can't construct requests). MCP tab/Try-It-Out require `mcp_mode == Gateway`. In passive proxy the client detail surfaces the **proxy setup** panel instead.
- **`src/components/connection-graph/utils/buildGraph.ts:468`** (`applyClientMode`): add a proxy layout (client → proxy → upstream) and split the LLM/MCP legs.
- **Demo mock** (`website/src/components/demo/TauriMockSetup.ts:159/282`): update create/set handlers, add `llm_mode`/`mcp_mode`, proxy path + CA fields.

---

## Part G — Config migration

`crates/lr-config/src/migration.rs`: add `migrate_to_v27` mapping each client's
`client_mode` → (`llm_mode`, `mcp_mode`) using Part A's table; bump `CONFIG_VERSION`
to 27 (`types.rs:7`). Keep the deserialize-only `client_mode` shim so configs
written before the migration still load. Add `#[serde(alias=...)]` if any variant
is ever renamed (none renamed here). No existing `ClientMode` variant is removed.

---

## Extensibility for the (later) active proxy
- Same `ProxyInterceptor` trait — active swaps `PassiveInterceptor` for `ActiveInterceptor`, returning `Replace(...)`; transport untouched.
- `ProxyRewrite` enum variant + disabled UI already in place.
- Active-mode rewrites (model allow-list / forced model / JSON optimization) can call the existing router/optimization code, since by then the request is fully parsed. Via-LLM-over-proxy becomes possible only in active mode (relax the Part A validity rule then).

## Representative files to touch
- Config/enums/migration: `crates/lr-config/src/types.rs`, `validation.rs`, `migration.rs`.
- Gates/routing: `crates/lr-server/src/routes/helpers.rs`, `chat.rs`, `responses.rs`, `mcp.rs`; `crates/lr-mcp/src/gateway/session.rs`, `crates/lr-mcp-via-llm/*`.
- New crate: `crates/lr-proxy/` (`listener.rs`, `mitm.rs`, `cert.rs`, `interceptor.rs`, `anthropic_parse.rs`, `manager.rs`).
- Monitor: `crates/lr-monitor/src/types.rs`, `store.rs` (consumer only).
- Launcher/commands: `src-tauri/src/launcher/{mod.rs,integrations/claude_code.rs}`, `src-tauri/src/ui/commands_clients.rs`, `src-tauri/src/main.rs` (command registration + proxy start).
- Frontend: `src/components/client/{ClientModeSelector,ClientTemplates,HowToConnect}.tsx`, `src/views/clients/*`, `src/components/connection-graph/utils/buildGraph.ts`, `src/types/tauri-commands.ts`, `website/src/components/demo/TauriMockSetup.ts`.

---

## Verification
1. **Unit tests:** v26→v27 migration for all 4 old modes; validity rejections (Off+Off, ViaLlm+proxy, ProxyRewrite); rcgen root+leaf generation and reuse; Anthropic SSE reconstruction (tokens + content_preview); secret redaction.
2. **Manual proxy smoke:** `curl -x http://<cid>:<secret>@127.0.0.1:<port> https://api.anthropic.com/v1/messages --cacert <config>/proxy/root-ca.pem ...` → succeeds, appears in Monitor with the proxy badge; a non-allow-listed host (e.g. `example.com`) tunnels without decryption.
3. **End-to-end (real app):** create a client in **Passive Proxy** mode → run Claude Code automated setup → confirm `~/.claude/settings.json` env block → run `claude` with a prompt using the Claude subscription → response works transparently, request/response visible in Monitor, upstream `Authorization` **not** logged. Confirm native `/v1` is denied for that client, and Models/Optimize/Try-It-Out are hidden.
4. **Regression:** existing Both/LlmOnly/McpOnly/McpViaLlm clients behave identically after migration; MCP-via-LLM still works.
5. **CI parity** (per `CLAUDE.md`): `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`, `fmt --check`, targeted `cargo test -p lr-proxy -p lr-config -p lr-server`; `npx tsc --noEmit`.

## Mandatory workflow steps
- **First:** create a todo list for all steps above; save this plan with `./copy-plan.sh <plan-name> HTTPS_PROXY_MODE` before writing code.
- **Final:** (1) Plan review vs implementation; (2) test-coverage review of new/changed paths; (3) fresh-eyes bug hunt (TLS lifetimes, streaming back-pressure, cert cache races, redaction gaps, migration edge cases); (4) commit only files I modified (no push unless asked), Matus's identity, no bot attribution.

## Notes / risks
- **Security/privacy (CLAUDE.md):** MITM only allow-listed LLM hosts; blind-tunnel everything else (never decrypt `claude.ai` auth); no telemetry; root-CA key `0600`; redact upstream credentials; surface a clear "LocalRouter can read this client's LLM traffic" notice in the proxy setup UI.
- **ToS:** passive mode is a transparent pass-through preserving the client's own subscription auth — the compatible corporate-proxy posture. We do not re-issue or store the OAuth token.
- **HTTP/1.1 forcing** (client-side ALPN = `http/1.1` only) is the key simplification; revisit if a target LLM host requires h2.
