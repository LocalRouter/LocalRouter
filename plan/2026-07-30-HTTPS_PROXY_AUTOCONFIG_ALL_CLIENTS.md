# HTTPS Proxy Auto-Config — All Client Templates

**Date**: 2026-07-30
**Research basis**: `plan/2026-07-29-HTTPS_PROXY_CLIENT_AUTOCONFIG_RESEARCH.md`

Extend automated HTTPS-inspection-proxy setup from Claude Code + Codex to every
template where it is technically possible, **including the ones where gateway
mode already suffices** (Aider, Continue) — with the caveats surfaced in the UI
rather than used as a reason to skip.

## Design

New module `src-tauri/src/launcher/proxy_setup.rs` centralizes per-template
proxy knowledge behind one struct, replacing the growing if/else in
`commands_clients.rs`:

```rust
pub struct ProxyPlan {
    pub ca_env_var: &'static str,       // CA env var for the one-off command
    pub oneoff_command: Option<String>, // launch-once command
    pub fragment: Option<String>,       // copyable config snippet (manual tab)
    pub file: Option<PathBuf>,          // file auto-config writes
    pub auto: Option<AutoWrite>,        // how to produce the new file body
    pub requires_system_ca: bool,       // CA must be in the OS trust store
    pub notes: Vec<String>,             // caveats rendered in the UI
    pub restart_hint: Option<String>,
}
```

`plan_for(template_id, proxy_url, ca_path)` builds it; `apply(plan)` writes with
`backup::write_with_backup`. Per-template merge helpers stay in their existing
`integrations/*.rs` modules (pure functions + unit tests, no filesystem).

Shared helper extracted: `integrations/dotenv.rs` (`merge_env`) — currently
private to `codex.rs`, reused by OpenClaw and Aider.

## Per-template matrix (what gets built)

| Template | File written | Keys | Caveats surfaced in UI |
|---|---|---|---|
| claude-code | `~/.claude/settings.json` | `env.HTTPS_PROXY`, `env.NODE_EXTRA_CA_CERTS` | (existing) |
| codex | `~/.codex/.env` | `HTTPS_PROXY`, `SSL_CERT_FILE` | (existing) |
| openclaw | `~/.openclaw/.env` | `HTTPS_PROXY`, `NODE_EXTRA_CA_CERTS` | restart gateway; installed services need `openclaw gateway install --force`; routes ALL egress incl. messaging transports |
| opencode | `~/.config/opencode/plugin/localrouter-proxy.ts` | sets `process.env` at startup | `NO_PROXY` loopback mandatory (TUI↔server loop); Bun 1.3.14 CONNECT bug oven-sh/bun#30381 — verify streaming; generated file, safe to delete |
| aider | `~/.env` | `HTTPS_PROXY`, `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `AIOHTTP_TRUST_ENV` | Python CA vars **replace** the trust store → we generate a **combined bundle**; `~/.env` is shared user namespace; gateway mode already covers Aider |
| vscode-continue | `~/.continue/config.yaml` | per-model `requestOptions.proxy` + `caBundlePath` | additive CA (best-behaved); only anthropic/openai models touched; VS Code `http.proxySupport=override` can shadow it; gateway mode already covers Continue |
| goose | `~/.config/goose/config.yaml` | `GOOSE_CA_CERT_PATH` | **no file mechanism for the proxy env var** → one-off command; chatgpt-codex provider ignores `GOOSE_CA_CERT_PATH` → needs system-keychain CA; Desktop app unreachable by config |
| zed | `~/.config/zed/settings.json` | `proxy` | JSONC comments are dropped on rewrite; CA needs system keychain (rustls platform verifier); agent may ignore proxy (zed#46570) |
| cline / roo-code | VS Code `settings.json` | `http.proxy` | **user-global — routes ALL VS Code traffic through LocalRouter**; CA needs system keychain; per-fork settings paths |
| cursor | — | — | not interceptable (all inference via `*.cursor.sh`) — stays off |

## System keychain CA trust (unlocks Goose, Zed, Cline, Roo, Crush)

New commands: `get_proxy_ca_trust_status`, `trust_proxy_ca`, `untrust_proxy_ca`.

macOS uses the **login keychain** (user-level trust, no admin/root):
`security add-trusted-cert -r trustRoot -k <login.keychain-db> <ca.pem>`;
removal via `security remove-trusted-cert`. Trust status probed with
`security verify-cert`. Non-macOS returns a structured "manual steps" result
rather than half-doing it. This is an explicit user action behind a confirm
dialog that states plainly what trusting a MITM root CA means — never automatic.

## Steps

1. Shared infra: `integrations/dotenv.rs`, `proxy_setup.rs` skeleton, combined
   CA bundle generator (`certifi` discovery + system-bundle fallback).
2. Per-template merge helpers + unit tests (openclaw, opencode, aider,
   continue, goose, zed, vscode).
3. Rewire `configure_client_proxy` / `get_client_proxy_setup` onto `ProxyPlan`.
4. Keychain trust commands + tests.
5. Frontend: `supportsProxy` flags, `ProxySetupInfo` fields (notes,
   requires_system_ca, supports_auto), HowToConnect renders notes + CA-trust
   button, types + website demo mock.
6. **Mandatory final steps** — plan review, test-coverage review, bug hunt, commit.

## Implementation notes (added during build)

Deviations and additions relative to the plan above:

- **Undo was added as a first-class part of the design.** Writing to eight
  different user config files without a removal path would be irresponsible, so
  `ProxyPlan` carries an `Undo` alongside `AutoWrite`, `unapply()` mirrors
  `apply()`, and `unconfigure_client_proxy` exposes it. A test asserts every
  template that can be applied can also be undone, and that undo stays
  available while the proxy is stopped (a stopped proxy must not strand config
  that is already on disk).
- **`jsonc.rs`** was needed because VS Code and Zed settings are JSONC:
  `serde_json` rejects comments and trailing commas. Rewriting drops comments,
  so `has_comments()` drives an explicit warning in the result message and a
  backup is always taken first.
- **Malformed config is never overwritten.** Every parse failure returns an
  error instead of falling back to an empty document — the previous Claude Code
  path silently replaced unparseable `settings.json`, which would have
  destroyed a user's settings.
- **Continue reports how many models it touched**; zero is an error ("no
  Anthropic/OpenAI models found") rather than a hollow success.
- **The combined CA bundle is generated on the read path too**, so Aider's
  manual instructions never name a file that doesn't exist yet; it short-circuits
  when the existing bundle already contains the CA, keeping that path cheap.
- **CA trust uses the login keychain, never `-d`/System.keychain**, so no admin
  rights are needed and the change is scoped to the current user. It is a
  separate, individually-confirmed action in the UI — never bundled into
  "Configure" — and always paired with a Remove-trust control.
- **Not implemented (deliberate):** Crush is not added as a new template — it is
  a candidate, not an existing template, and adding one is a separate change.
  Widening `MITM_HOST_ALLOWLIST` (Gemini CLI, Copilot CLI, `cloud.zed.dev`,
  `api.factory.ai`) is also out of scope: that expands what LocalRouter
  decrypts and deserves its own decision.
