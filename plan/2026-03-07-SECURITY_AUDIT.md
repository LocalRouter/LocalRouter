# LocalRouter Security Audit Report

**Date**: 2026-03-07
**Scope**: Full codebase security audit (backend, frontend, configuration, dependencies)
**Version**: 0.1.0

---

## Executive Summary

LocalRouter demonstrates **solid security fundamentals** overall: proper use of OS keychain for secrets, no telemetry, localhost-only defaults, React's built-in XSS protection, and correct authentication enforcement on API routes. However, the audit uncovered **6 critical**, **12 high**, **16 medium**, and **9 low** severity findings across authentication, cryptography, input validation, network security, MCP subsystem, coding agents, and frontend security.

The most urgent issues are: timing attacks on secret comparison, plaintext secret storage in file-based keychain mode, environment variable injection in MCP process spawning, and the CSP configuration allowing `unsafe-eval`.

---

## Findings by Severity

### CRITICAL (6)

#### C1. Non-Constant-Time Secret Comparison (Timing Attack)
- **Files**: `crates/lr-clients/src/manager.rs:178,241`
- **Issue**: Client secret verification uses standard `==` operator, which short-circuits on first mismatched byte. An attacker can measure response timing to deduce secrets character-by-character.
- **Fix**: Use `subtle::ConstantTimeEq` or similar constant-time comparison. Apply to all secret/token comparisons.

#### C2. File-Based Keychain Stores Secrets in Plain JSON
- **File**: `crates/lr-api-keys/src/keychain_trait.rs:102-196`
- **Issue**: `FileKeychain` (activated by `LOCALROUTER_KEYCHAIN=file`) writes all secrets as unencrypted JSON. The file is created without restrictive permissions (`0o600`). While documented as "development only", it's available in release builds.
- **Fix**: Either (a) disable `FileKeychain` in release builds via `#[cfg(debug_assertions)]`, or (b) encrypt the JSON with a derived key, and (c) always set file permissions to `0o600` on Unix.

#### C3. In-Memory Secret Cache Without Zeroization
- **File**: `crates/lr-api-keys/src/keychain_trait.rs:236-242`
- **Issue**: `CachedKeychain` stores secrets in a standard `HashMap<String, String>`. Rust's `String` does not zero memory on drop, so secrets persist in process memory and may appear in core dumps or swapped pages.
- **Fix**: Use `secrecy::SecretString` or `zeroize::Zeroizing<String>` for cached values. Consider TTL-based cache expiration.

#### C4. Environment Variable Injection in MCP Process Spawning
- **Files**: `crates/lr-mcp/src/manager.rs:418-424`, `crates/lr-mcp/src/transport/stdio.rs:89-107`
- **Issue**: Auth environment variables from `McpAuthConfig::EnvVars` and `SessionConfig.env` are merged directly into spawned process environments without validation. An attacker who controls config (or a malicious MCP server template) can inject `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, `NODE_OPTIONS`, or `PYTHONPATH` to achieve code execution.
- **Fix**: Implement an allowlist of permitted environment variable names. Block known dangerous variables (`LD_PRELOAD`, `DYLD_*`, `LD_LIBRARY_PATH`, `NODE_OPTIONS` with `--require`, etc.).

#### C5. CSP Allows `unsafe-eval` and `unsafe-inline`
- **File**: `src-tauri/tauri.conf.json:71`
- **Issue**: The Content Security Policy includes `'unsafe-eval'` in `default-src`, allowing `eval()`, `Function()`, and similar dynamic code execution in the webview. Combined with `'unsafe-inline'`, this largely defeats XSS protections.
- **Fix**: Remove `'unsafe-eval'` entirely (React/Vite apps should not need it). Replace `'unsafe-inline'` with nonce-based styles if possible, or at minimum scope it to `style-src` only.

#### C6. Internal Test Secret Logged to Disk
- **File**: `crates/lr-server/src/state.rs:721-727`
- **Issue**: A transient bearer token is generated on startup for UI testing. While the token value itself isn't logged, the pattern `lr-internal-{uuid}` is predictable and the token is accessible to the entire `AppState` via `get_internal_test_secret()`. Combined with log file access, this could allow authentication bypass.
- **Fix**: Don't expose the test secret via a public getter. Generate a fully random token. Consider gating behind `#[cfg(debug_assertions)]`. Never log the actual secret value.

---

### HIGH (12)

#### H1. O(n) Linear Secret Lookup Amplifies Timing Attack
- **File**: `crates/lr-clients/src/manager.rs:222-261`
- **Issue**: `verify_secret()` iterates through ALL clients doing keychain lookups and `==` comparisons. Combined with C1, this leaks how many clients exist and which client index matched.
- **Fix**: Use a hash-based lookup table (e.g., `HashMap<HashedSecret, ClientId>`). Ensure constant-time comparison regardless of match position.

#### H2. Insecure OAuth State Parameter Generation
- **File**: `crates/lr-oauth/src/browser/pkce.rs:87-99`
- **Issue**: OAuth `state` parameter is generated using `thread_rng()`, which is NOT guaranteed to be cryptographically secure. Predictable state values undermine CSRF protection in the OAuth flow.
- **Fix**: Use `ring::rand::SystemRandom` or `getrandom` crate for state generation.

#### H3. No Rate Limiting on Token/Auth Endpoints
- **Files**: `crates/lr-server/src/routes/oauth.rs`, `crates/lr-clients/src/token_store.rs:83-95`
- **Issue**: The `/oauth/token` endpoint has no rate limiting. Token generation is unlimited per client. Combined with H2, brute-force attacks on authorization codes are feasible.
- **Fix**: Apply rate limiting to `/oauth/token` (e.g., 5 req/min per IP, 10 tokens/hour per client). Add max active tokens per client.

#### H4. Bearer Token Format Not Validated
- **File**: `crates/lr-server/src/middleware/client_auth.rs:33-46`
- **Issue**: Any non-empty string after `Bearer ` is accepted and looked up in the keychain. No length limit, no character validation, no format check.
- **Fix**: Enforce max token length (e.g., 256 chars), validate character set (alphanumeric + `-_`), and optionally require `lr-` prefix.

#### H5. CORS Allows Any Origin
- **File**: `crates/lr-server/src/lib.rs:238-254`
- **Issue**: When CORS is enabled, `allow_origin(Any)` + `allow_headers(Any)` + `expose_headers(Any)` allows any website to make API requests. While `allow_credentials(false)` prevents cookie-based CSRF, bearer tokens in headers are still accessible to any origin.
- **Fix**: Restrict to `http://localhost:*` and `http://127.0.0.1:*`. If binding changes to `0.0.0.0`, CORS must be strictly scoped.

#### H6. No Security Headers on API Responses
- **File**: `crates/lr-server/src/lib.rs`
- **Issue**: Missing `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy`, and `Cache-Control` on API responses.
- **Fix**: Add a security headers middleware layer to all routes.

#### H7. Sensitive Data Logged via Debug Derive
- **File**: `src-tauri/src/ui/commands_mcp.rs:77-87`
- **Issue**: `FrontendAuthConfig` derives `Debug` and is logged at debug/error level. Bearer tokens, OAuth secrets, and custom headers appear in plaintext in logs.
- **Fix**: Implement custom `Debug` that redacts sensitive fields. Remove logging of raw auth JSON values.

#### H8. Config File Created Without Restrictive Permissions
- **File**: `crates/lr-config/src/storage.rs:155-178`
- **Issue**: `fs::File::create()` uses default permissions. On shared systems, config files may be world-readable, exposing client IDs, server names, and keychain references.
- **Fix**: Set `0o600` permissions on Unix after file creation.

#### H9. YAML Deserialization Without Size Limits
- **File**: `crates/lr-config/src/storage.rs:91`
- **Issue**: Config YAML is parsed without file size limits or depth limits. A malicious config with deeply nested structures or massive arrays could exhaust memory.
- **Fix**: Enforce max file size (e.g., 10MB) before parsing. Consider `serde_yaml` depth limits.

#### H10. No MCP Server Config Validation
- **File**: `crates/lr-config/src/validation.rs:84-121`
- **Issue**: Provider configs are validated but MCP server configs have NO validation function. Invalid URLs, injection patterns in commands, and insecure OAuth endpoints are accepted silently.
- **Fix**: Add `validate_mcp_server_config()` - validate URLs (reject `javascript:`, `file://`), validate commands against known agent types, validate OAuth endpoints use HTTPS.

#### H11. Path Traversal in Coding Agent Working Directory
- **File**: `crates/lr-coding-agents/src/manager.rs:106`
- **Issue**: The `working_directory` parameter is used in `cmd.current_dir()` without validation. A caller could set it to `../../../` or any absolute path.
- **Fix**: Canonicalize the path and validate it's within an allowed base directory.

#### H12. No Process Sandboxing for Coding Agents
- **File**: `crates/lr-coding-agents/src/manager.rs:509-625`
- **Issue**: Coding agents run with full user privileges. No `seccomp`, `AppArmor`, namespace isolation, or resource limits (`rlimit`). A compromised agent can read LocalRouter's memory, access all user files, and exfiltrate data.
- **Fix**: This is architectural and intentional (agents need FS access), but should be clearly documented in a trust model. Consider adding `rlimit` for CPU/memory, and a configurable agent binary allowlist.

---

### MEDIUM (16)

#### M1. No Request Body Size Limits
- **File**: `crates/lr-server/src/lib.rs`
- **Issue**: No `RequestBodyLimitLayer` configured. Attackers can send arbitrarily large payloads.
- **Fix**: Add `tower_http::limit::RequestBodyLimitLayer::new(10 * 1024 * 1024)`.

#### M2. No OAuth Redirect URI Validation
- **File**: `crates/lr-config/src/types.rs:2121-2143`
- **Issue**: OAuth `redirect_uri` accepts any string. Could be set to `http://attacker.com/callback`.
- **Fix**: Validate redirect URIs are `http://localhost:*` or `http://127.0.0.1:*`.

#### M3. DNS Rebinding Not Addressed
- **File**: `crates/lr-server/src/lib.rs`
- **Issue**: No `Host` header validation. If server is bound to `0.0.0.0`, DNS rebinding attacks could allow remote websites to access the API through a custom domain resolving to 127.0.0.1.
- **Fix**: Validate `Host` header against expected values when binding to non-localhost.

#### M4. Expired Tokens Not Automatically Cleaned
- **File**: `crates/lr-clients/src/token_store.rs:101-117`
- **Issue**: Tokens are only removed when `verify_token()` is called (lazy cleanup). `cleanup_expired()` exists but is never called automatically.
- **Fix**: Spawn a background task to run `cleanup_expired()` periodically (e.g., every 5 minutes).

#### M5. Keychain Failures Log Account Names
- **File**: `crates/lr-mcp/src/manager.rs:461-474`
- **Issue**: When keychain lookup fails, warning logs include the account name, which could be enumerated by log-reading attackers.
- **Fix**: Log generic messages without account identifiers.

#### M6. Missing Timeout on OpenAI-Compatible Provider
- **File**: Provider client initialization
- **Issue**: Some provider HTTP clients use bare `Client::new()` without timeout configuration. Requests could hang indefinitely.
- **Fix**: Add 60-second timeout to all provider HTTP clients.

#### M7. Firewall Approval Requests Not Rate Limited
- **File**: `crates/lr-mcp/src/gateway/firewall.rs:164-196`
- **Issue**: No rate limiting on MCP tool approval popups. Attacker could spam approval requests to DoS the user or exploit auto-timeout behavior.
- **Fix**: Rate-limit approval requests (max 1 per tool per 5 seconds). Add exponential backoff for repeated denials.

#### M8. Health Check Timeout Unbounded
- **File**: `crates/lr-config/src/types.rs:486-498`
- **Issue**: `timeout_secs: u64` has no bounds validation. Values < 1 or > 300 cause issues.
- **Fix**: Validate in `validation.rs`: require 1-300 seconds.

#### M9. Unvalidated URLs in Frontend href Attributes
- **Files**: `src/components/add-resource/MarketplaceSearchPanel.tsx`, `src/components/ProviderForm.tsx`, `src/components/client/HowToConnect.tsx`
- **Issue**: URLs from backend data (marketplace, OAuth, docs) placed directly in `<a href>`. Could execute `javascript:` URIs.
- **Fix**: Validate that URLs use `http:` or `https:` protocol before rendering.

#### M10. Client Secret Retrieval Without Rate Limiting
- **File**: `src-tauri/src/ui/commands_clients.rs`
- **Issue**: `get_client_value()` returns raw secrets without rate limiting or audit logging.
- **Fix**: Add rate limiting and audit logging.

#### M11. Process Execution Without Timeout (Coding Agent Version Check)
- **File**: `src-tauri/src/ui/commands_coding_agents.rs:113-120`
- **Issue**: `get_coding_agent_version()` spawns processes without timeout.
- **Fix**: Wrap in `tokio::time::timeout(Duration::from_secs(5), ...)`.

#### M12. Coding Agent Environment Variable Injection
- **File**: `crates/lr-coding-agents/src/manager.rs:528-536`
- **Issue**: `SessionConfig.env` HashMap passed directly to spawned processes without allowlist.
- **Fix**: Same approach as C4 - implement env var allowlist.

#### M13. No Resource Limits on Agent Processes
- **File**: `crates/lr-coding-agents/src/manager.rs:509-625`
- **Issue**: No `rlimit`, cgroup, or timeout on spawned coding agents. A single session can exhaust all system resources.
- **Fix**: Add `rlimit` for memory (e.g., 1GB) and CPU time. Add session timeout.

#### M14. Unvalidated Path Opening
- **File**: `src-tauri/src/ui/commands.rs` (open_path)
- **Issue**: `open_path()` only checks path exists, doesn't restrict which directories can be opened.
- **Fix**: Validate path is within expected directories (config dir, etc.).

#### M15. Provider Error Messages May Leak Upstream Details
- **File**: `crates/lr-server/src/middleware/error.rs:115-155`
- **Issue**: `format!("Provider error: {}", msg)` could relay upstream API error messages containing internal details.
- **Fix**: Sanitize provider error messages before including in responses.

#### M16. Client Secret Exposed via Environment Variable
- **File**: `crates/lr-mcp/src/bridge/stdio_bridge.rs:290`
- **Issue**: `LOCALROUTER_CLIENT_SECRET` env var is visible in `ps` output and shell history.
- **Fix**: Document as CI/CD only. Prefer keychain storage.

---

### LOW (9)

#### L1. Integer Casting Without Bounds Check
- **File**: `crates/lr-server/src/routes/chat.rs:161-184`
- **Issue**: `u64` to `u32` cast via `as u32` can silently truncate.
- **Fix**: Use `u32::try_from(n).ok()`.

#### L2. Admin Functions Bypass Client Ownership
- **File**: `crates/lr-coding-agents/src/manager.rs:439-485`
- **Issue**: `get_session_detail()`, `list_all_sessions()` don't validate client ownership.
- **Fix**: Add optional `client_id` parameter for access control.

#### L3. Config Migration Can Lose Data
- **File**: `crates/lr-config/src/migration.rs:443-475`
- **Issue**: Some migrations (e.g., v12) reset fields to defaults.
- **Fix**: Document data loss in migration comments.

#### L4. State Parameter Size Not Limited
- **File**: `crates/lr-oauth/src/browser/callback_server.rs`
- **Issue**: Incoming OAuth state parameter has no max length check.
- **Fix**: Reject state parameters > 256 chars.

#### L5. No Audit Logging for Auth Events
- **Files**: Multiple authentication points
- **Issue**: Failed auth attempts, secret rotation, token generation not logged at INFO level.
- **Fix**: Implement structured audit logging.

#### L6. Unused API Key Hash Functions
- **File**: `crates/lr-utils/src/crypto.rs:21-30`
- **Issue**: `hash_api_key()` and `verify_api_key()` are dead code (`#[allow(dead_code)]`). Suggests incomplete migration to hashed key storage.
- **Fix**: Either implement hash-based verification everywhere or remove dead code.

#### L7. YAML Config Accepts Unknown Fields
- **File**: `crates/lr-config/src/storage.rs`
- **Issue**: Serde default allows unknown fields in config.
- **Fix**: Consider `#[serde(deny_unknown_fields)]` on critical config structs.

#### L8. Backup Cleanup Race Condition
- **File**: `crates/lr-config/src/storage.rs:241-253`
- **Issue**: Backup listing and deletion aren't atomic.
- **Fix**: Serialize with a lock or make idempotent.

#### L9. Frontend Console Logging
- **Files**: Multiple frontend components (228 instances)
- **Issue**: Development `console.log` statements in production code.
- **Fix**: Gate behind development mode checks or remove.

---

## Positive Findings

The audit also identified several areas of strong security practice:

| Area | Assessment |
|------|-----------|
| **No telemetry/tracking** | Confirmed - zero analytics, no phoning home |
| **No external assets** | All resources bundled at build time |
| **Localhost-only default** | Server binds to `127.0.0.1` by default |
| **OS keychain integration** | Secrets stored in macOS Keychain / Windows Credential Manager / Linux Secret Service |
| **API key generation** | Uses `ring::rand::SystemRandom` with 256-bit entropy |
| **Bug report sanitization** | Comprehensive redaction of secrets before clipboard copy |
| **React XSS protection** | No `dangerouslySetInnerHTML`, no `eval()`, no dynamic script injection |
| **WebSocket authentication** | Bearer tokens required, per-client permission checks enforced |
| **Rate limiting engine** | Well-implemented sliding window with per-key limits |
| **Update signature verification** | Configured with public key verification |
| **Process group management** | Coding agents use `command_group` for proper cleanup |
| **OAuth PKCE** | Implements PKCE with code verifier/challenge |

---

## Remediation Priority

### Immediate (Next 48 Hours)
| ID | Finding | Effort |
|----|---------|--------|
| C1 | Constant-time secret comparison | Small - add `subtle` crate |
| C5 | Remove `unsafe-eval` from CSP | Small - config change |
| C6 | Stop logging test secret, gate behind debug | Small |
| H7 | Redact secrets in Debug impl | Small |

### Urgent (Next Week)
| ID | Finding | Effort |
|----|---------|--------|
| C4 | Env var allowlist for MCP spawning | Medium |
| C2 | Encrypt or disable FileKeychain in release | Medium |
| H2 | Use SystemRandom for OAuth state | Small |
| H3 | Rate limit /oauth/token | Medium |
| H4 | Validate bearer token format | Small |
| H5 | Restrict CORS to localhost origins | Small |
| H6 | Add security headers middleware | Small |
| H8 | Set 0o600 on config files | Small |

### Important (Next 2 Weeks)
| ID | Finding | Effort |
|----|---------|--------|
| C3 | Use SecretString for cached secrets | Medium |
| H1 | Hash-based secret lookup | Medium |
| H9 | Config file size limit | Small |
| H10 | MCP server config validation | Medium |
| H11 | Coding agent path traversal fix | Small |
| M1 | Request body size limit | Small |
| M2 | OAuth redirect URI validation | Small |
| M4 | Auto-cleanup expired tokens | Small |
| M6 | Add timeout to all provider clients | Small |
| M9 | Validate frontend URLs | Small |

### Normal (Next Month)
| ID | Finding | Effort |
|----|---------|--------|
| H12 | Document coding agent trust model | Medium |
| M3 | DNS rebinding protection | Medium |
| M5 | Redact keychain account names in logs | Small |
| M7 | Rate limit firewall approvals | Small |
| M8-M16 | Remaining medium findings | Various |
| L1-L9 | Low severity findings | Various |

---

## Threat Model Summary

### Attack Surfaces

1. **HTTP API (port 3625)** - Authenticated, localhost-only by default. Primary risk: timing attacks, DoS via large payloads.
2. **MCP Gateway** - Proxies to user-configured MCP servers. Risk: SSRF, env var injection in STDIO transport.
3. **Coding Agents** - Spawns full-privilege processes. Risk: path traversal, resource exhaustion, env var injection.
4. **Configuration Files** - YAML parsed at startup. Risk: DoS via large files, env var injection via auth config.
5. **Tauri IPC** - Frontend to backend commands. Risk: no per-command ACLs (acceptable for single-user desktop app).
6. **OAuth Flow** - Browser-based auth. Risk: weak state parameter, no redirect URI validation.

### Trust Boundaries

```
[Internet] <-- HTTPS --> [Provider APIs]
                              ^
                              |
[User Browser] <-- IPC --> [Tauri App] <-- HTTP --> [Axum Server :3625]
                              |                          |
                              |                     [MCP Gateway]
                              |                          |
                              |                  [MCP Servers (stdio/SSE/HTTP)]
                              |
                          [Coding Agents (claude, codex, etc.)]
```

Key trust assumptions:
- Config files are as trusted as code (user-controlled)
- MCP servers run with user privileges (no sandboxing)
- Coding agents run with full user access (intentional)
- Localhost binding prevents external access (broken if user changes to 0.0.0.0)

---

## Methodology

This audit was conducted by 7 parallel analysis agents, each specializing in a domain:

1. **Authentication, Authorization & Cryptography** - API keys, secrets, keychain, OAuth, encryption
2. **Input Validation & Injection** - Command injection, path traversal, XSS, SSRF, deserialization
3. **Network & Transport Security** - Binding, CORS, CSP, TLS, rate limiting, DNS rebinding
4. **MCP & Configuration Security** - MCP gateway/transport, config parsing, file permissions
5. **Dependencies & Information Leakage** - Dependency audit, error handling, logging, privacy, unsafe code
6. **Tauri & Frontend Security** - Tauri config, IPC, auto-updater, XSS, URL handling
7. **Coding Agents Security** - Agent execution, sandboxing, process management

Each agent performed deep code review of relevant source files, searching for patterns associated with OWASP Top 10, CWE Top 25, and Rust-specific security concerns.
