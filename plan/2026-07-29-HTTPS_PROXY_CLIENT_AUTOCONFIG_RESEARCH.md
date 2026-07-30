# HTTPS Inspection Proxy — Client Auto-Config Research (all templates)

**Date**: 2026-07-29
**Question**: beyond Claude Code and Codex, which client templates should get
automated HTTPS-proxy config setup, and what mechanism would each use?

Context recap: the proxy MITMs only `api.anthropic.com`, `api.openai.com`,
`chatgpt.com` (`lr-proxy/src/lib.rs MITM_HOST_ALLOWLIST`); everything else is
blind-tunneled. Proxy mode only adds value where LLM traffic **cannot be
repointed** at the gateway (subscription OAuth with hardcoded endpoints).
Auto-config needs three things per tool: (a) `HTTPS_PROXY` honored (with
embedded basic-auth), (b) custom root-CA trust that is **additive** to system
roots, (c) a persistent file LocalRouter can write.

Ecosystem note: Anthropic banned third-party use of Claude Pro/Max OAuth
(server-side blocks Jan 2026, ToS Feb 2026). opencode removed it (1.3.0),
Roo Code removed it, Crush removed it; OpenClaw still ships it (impersonates
Claude Code's client id). The live unrepointable-subscription story everywhere
else is **ChatGPT Plus/Pro (Codex) OAuth → chatgpt.com**, which our allowlist
covers.

## Feasibility matrix

| Tool | Interceptable traffic | HTTPS_PROXY | Custom CA | Persistent file | Verdict |
|---|---|---|---|---|---|
| OpenClaw | Anthropic OAuth¹ + ChatGPT OAuth + api.openai.com | undici EnvHttpProxyAgent, creds OK | `NODE_EXTRA_CA_CERTS` (additive, documented for MITM) | **`~/.openclaw/.env`** (purpose-built, baked into service installs) | **Auto-config now** — same pattern as Codex `.env` |
| opencode | ChatGPT OAuth (HTTP + wss) ; anthropic API-key² | Bun fetch env-proxy, creds verified; wss made proxy-aware by opencode itself | `NODE_EXTRA_CA_CERTS` additive (verified); binary always runs `--use-system-ca` | No env block / no dotenv. Only hook: auto-loaded global plugin `~/.config/opencode/plugin/*.ts` setting `process.env` | **supportsProxy + one-off now**; plugin-file auto-config after testing Bun CONNECT bug³. `NO_PROXY=localhost,127.0.0.1,::1` mandatory (TUI↔server loop) |
| Goose | ChatGPT Codex OAuth + api.openai/anthropic (API-key) | reqwest default env-proxy, creds OK | Platform verifier → **OS trust store only** for the chatgpt-codex client (`GOOSE_CA_CERT_PATH` exists but that client ignores it; `SSL_CERT_FILE` ignored on macOS) | None for env vars (no .env, no config key for proxy; Desktop unreachable) | Blocked on **OS-keychain CA install**; then one-off CLI command |
| Cline / Roo Code | ChatGPT-Codex subscription providers → chatgpt.com (unrepointable, via VS Code patched fetch) | VS Code `http.proxy` (creds-in-URL allowed); `http.fetchAdditionalSupport` covers fetch | OS trust store via `http.systemCertificates` (default on) | VS Code `settings.json` — but **user-global**: routes ALL VS Code traffic through us | Feasible but heavyweight; manual instructions / explicit opt-in only |
| Zed | BYO-key anthropic/openai + **ChatGPT-subscription provider** (`CODEX_BASE_URL = chatgpt.com`) | `proxy` key in settings.json + env vars (agent bug zed#46570) | rustls platform verifier → OS trust store only | `~/.config/zed/settings.json` | Real value; blocked on OS-keychain CA install |
| Aider | api-key only, all repointable | litellm/httpx yes | `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE` **replace** certifi → needs combined bundle | `~/.env`, `.aider.conf.yml` | **Skip** — gateway mode fully covers it |
| Continue | api-key only, repointable | per-model `requestOptions.proxy` | `caBundlePath` truly additive | `~/.continue/config.yaml` | **Skip** — gateway mode fully covers it |
| Droid/Factory | managed → api.factory.ai (invisible); BYOK repointable | — | — | — | **Skip** (gateway covers BYOK) |
| Cursor | all inference via `*.cursor.sh` even BYOK | — | — | — | **Not interceptable** |
| JetBrains AI | subscription via Grazie backend; BYOK likely direct | proxy OK | OS store | — | Subscription invisible; BYOK gateway-servable. Skip |
| Xcode | Apple relay / cert-pinned ChatGPT / custom base URL | — | — | — | **Not interceptable**; custom-provider field → gateway instead |
| Amp | inference is server-side (ampcode.com) | — | — | — | **Structurally impossible** |

¹ ToS-banned by Anthropic; OpenClaw ships it anyway — inspection still works, not our liability.
² opencode removed Claude Max OAuth in 1.3.0 (Anthropic legal); anthropic API-key traffic is repointable via `baseURL` so MITM optional.
³ opencode ships Bun 1.3.14 which carries CONNECT-tunnel bug oven-sh/bun#30381 (proxy-header leak / hang on chunked keep-alive; fix merged upstream, unreleased). Must integration-test SSE streaming through our tunnel against the real binary before flipping auto-config on.

## Future host-allowlist candidates (separate decision — expands MITM surface)

- Gemini CLI: `cloudcode-pa.googleapis.com`, `generativelanguage.googleapis.com`; clean undici proxy + `NODE_EXTRA_CA_CERTS`; also `CODE_ASSIST_ENDPOINT` env allows repointing without MITM.
- GitHub Copilot CLI: `api*.githubcopilot.com`; proxy + `NODE_EXTRA_CA_CERTS` OK.
- Crush (charmbracelet): hits api.anthropic/openai directly (in-allowlist already!); Go env-proxy free; but Go ignores `SSL_CERT_FILE` on macOS entirely → OS keychain required. Candidate template once keychain install exists.
- Zed-hosted (`cloud.zed.dev`), Factory (`api.factory.ai`), JetBrains Grazie: possible but non-OpenAI wire formats; low value.

## Recommended sequence

1. **OpenClaw auto-config** — mirror the Codex `.env` merge: append
   `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` to `~/.openclaw/.env`, note
   "restart the gateway (`openclaw gateway restart`; installed services:
   `openclaw gateway install --force`)". Add `supportsProxy: true`. Low
   effort, best-in-class fit.
2. **opencode**: set `supportsProxy: true` + one-off command
   (`HTTPS_PROXY=… NO_PROXY=localhost,127.0.0.1,::1 NODE_EXTRA_CA_CERTS=… opencode`).
   Auto-config = write `~/.config/opencode/plugin/localrouter-proxy.ts`
   (sets `process.env` at startup) — gate behind an integration test for the
   Bun CONNECT bug. Also verify our wss interception against its Codex
   websocket transport.
3. **"Trust LocalRouter CA in the system keychain" feature** (explicit user
   consent, `security add-trusted-cert`, admin prompt; removal path too).
   This one feature unlocks Goose, Zed, Crush, and makes the Cline/Roo
   VS Code path viable. Highest-leverage single investment.
4. After (3): Goose one-off command, Zed `settings.json` `proxy` key
   auto-config, optional Cline/Roo manual instructions for VS Code
   `http.proxy` (explicit opt-in only — it routes all VS Code traffic).
5. **Skip permanently** (document why in template comments): Aider, Continue
   (gateway sufficient), Cursor, Amp, Xcode built-in, JetBrains subscription
   (not interceptable).

Full per-tool citations live in the four research agent reports (session
2026-07-29); key upstream refs: openclaw `docs/security/network-proxy.md` +
`docs/help/environment.md`; opencode `packages/web/.../network.mdx`,
`plugin/openai/codex.ts`, build.ts `--use-system-ca`; goose
`providers/chatgpt_codex.rs`, reqwest 0.13 platform-verifier default; vscode
`request.ts` (`http.proxy` creds pattern, `http.systemCertificates`,
`http.fetchAdditionalSupport`); zed `openai_subscribed.rs`; Go
`root_darwin.go` (env vars ignored on macOS); oven-sh/bun#30381.
