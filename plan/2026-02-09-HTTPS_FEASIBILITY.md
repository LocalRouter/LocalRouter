# HTTPS Support for LocalRouter - Feasibility Assessment

## Context

LocalRouter currently serves all endpoints (OpenAI API, MCP Gateway, OAuth, health) over **plain HTTP** via `axum::serve()` with a raw `TcpListener`. There are zero TLS dependencies in the project. This document assesses the difficulty of adding HTTPS with self-signed certificates and optional system trust store installation.

## Difficulty Assessment: **Moderate**

The core HTTPS change is straightforward (~2-3 days). System cert installation adds complexity due to cross-platform differences and permission requirements (~1-2 days additional). The main risk is not code complexity but UX edge cases (cert expiry, trust prompts, WebSocket+TLS).

---

## What Changes

### 1. New Dependencies (Cargo.toml)
- `axum-server` with `tls-rustls` feature — replaces `axum::serve()` for TLS binding
- `rcgen` — programmatic self-signed certificate generation (pure Rust, no OpenSSL)
- `rustls` + `rustls-pemfile` — TLS implementation (already transitive via other crates likely)

### 2. Certificate Generation & Management
**New module**: `crates/lr-server/src/tls.rs` (or similar)

- On first startup (when HTTPS enabled), generate a self-signed CA + server cert using `rcgen`
- Store PEM files in the app data directory (`~/.localrouter/certs/` or platform equivalent)
- SANs: `localhost`, `127.0.0.1`, `::1`, and optionally user-configured hostnames
- Cert validity: 1-2 years, with auto-regeneration on expiry
- Reuse existing certs on subsequent startups (don't regenerate every launch)

### 3. Server Startup Changes
**File**: `crates/lr-server/src/lib.rs` (lines 68-153)

Current:
```rust
let listener = TcpListener::bind(addr).await?;
axum::serve(listener, app).await?;
```

With HTTPS:
```rust
let rustls_config = RustlsConfig::from_pem_file(&cert_path, &key_path).await?;
axum_server::bind_rustls(addr, rustls_config)
    .serve(app.into_make_service())
    .await?;
```

- The port fallback logic needs adaptation (axum-server has a slightly different API)
- Optionally serve both HTTP and HTTPS simultaneously (HTTP redirects to HTTPS, or just HTTPS)

### 4. Configuration Changes
**File**: `crates/lr-config/src/types.rs` — `ServerConfig` struct (lines 577-588)

Add fields:
```rust
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub enable_cors: bool,
    // New:
    pub enable_tls: bool,           // Default: false (opt-in)
    pub tls_cert_path: Option<String>,  // Custom cert, None = auto-generate
    pub tls_key_path: Option<String>,   // Custom key, None = auto-generate
}
```

Config migration from v6 → v7 to add defaults.

### 5. Frontend Updates
- **`src/components/client/HowToConnect.tsx`**: Change hardcoded `http://` to dynamic based on `enable_tls` config
- **`src-tauri/tauri.conf.json`**: Add `https://localhost:*` and `https://127.0.0.1:*` to CSP `connect-src`
- **`src-tauri/src/ui/commands.rs`**: Expose `enable_tls` in `get_server_config()` / `update_server_config()`
- **`src/types/tauri-commands.ts`**: Add TLS fields to TypeScript types
- **`website/src/components/demo/TauriMockSetup.ts`**: Add mock TLS fields

### 6. System Trust Store Installation (Optional, config-gated)

This is the hardest part. When enabled, the app would install the generated CA cert into the OS trust store so clients (curl, Python, browsers) trust the self-signed cert without `--insecure` flags.

**Per-platform approach** (shell commands from Rust):

| Platform | Command | Requires |
|----------|---------|----------|
| macOS | `security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain cert.pem` | Admin password (sudo) |
| Linux (Debian/Ubuntu) | Copy to `/usr/local/share/ca-certificates/` + `update-ca-certificates` | sudo |
| Linux (Fedora/RHEL) | Copy to `/etc/pki/ca-trust/source/anchors/` + `update-ca-trust` | sudo |
| Windows | `certutil -addstore -user Root cert.pem` | User-level (no admin needed for `-user`) |

**Implementation approach**:
- **Never auto-install** — only via explicit user action (button in Settings or config flag)
- Prompt user with explanation of what will happen
- On macOS/Linux: requires sudo, so spawn a privileged helper or instruct user
- On Windows: `-user` flag avoids admin, but only trusts for current user
- Provide "uninstall cert" action as well
- Alternative: provide instructions for manual installation instead of automating it

### 7. Dual HTTP/HTTPS Mode (Optional)

Could serve HTTP on one port and HTTPS on another simultaneously. This simplifies migration — existing clients keep working on HTTP while new clients can opt into HTTPS. This would mean spawning two `axum::serve` tasks.

---

## Risks & Edge Cases

1. **WebSocket over TLS (WSS)**: MCP WebSocket endpoint (`/mcp/ws`) needs to work over `wss://` — should work automatically with axum-server TLS but needs testing
2. **SSE over TLS**: MCP SSE transport should also work transparently
3. **OAuth callback**: The OAuth callback server (`lr-oauth`) runs on separate ports — those would remain HTTP (they're ephemeral, browser-only)
4. **Port fallback**: `axum-server::bind_rustls` may have different error handling than `TcpListener::bind` — the retry loop needs adaptation
5. **Cert regeneration**: If host config changes (e.g., `0.0.0.0` → `192.168.1.x`), SANs need updating and cert needs regeneration
6. **Client compatibility**: Some LLM clients may not handle self-signed certs well without system trust — the trust store installation feature addresses this

## Files to Modify

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add `axum-server`, `rcgen`, `rustls-pemfile` |
| `crates/lr-server/Cargo.toml` | Add deps |
| `crates/lr-server/src/lib.rs` | TLS-aware server startup |
| `crates/lr-server/src/tls.rs` | **New** — cert generation, loading, trust store installation |
| `crates/lr-config/src/types.rs` | Add TLS config fields |
| `src-tauri/src/ui/commands.rs` | Expose TLS config |
| `src-tauri/src/main.rs` | Pass TLS config to server |
| `src-tauri/tauri.conf.json` | Update CSP for HTTPS |
| `src/components/client/HowToConnect.tsx` | Dynamic http/https |
| `src/types/tauri-commands.ts` | TLS type fields |
| `website/src/components/demo/TauriMockSetup.ts` | Mock TLS fields |

## Verification

1. Enable TLS in config, start server, verify HTTPS endpoint responds
2. Test `curl -k https://localhost:3625/health` (with `-k` to skip cert verification)
3. Test MCP SSE and WebSocket over TLS
4. Test OpenAI chat completions endpoint over HTTPS
5. Install cert to system trust store, verify `curl https://localhost:3625/health` works without `-k`
6. Test with an LLM client (e.g., Claude Code) pointing at `https://localhost:3625`
7. Verify HTTP-only mode still works (default, `enable_tls: false`)
8. Test cert auto-generation on first startup and reuse on subsequent startups
