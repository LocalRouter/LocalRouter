# Plan: Re-implement gardinermichael:feat/provider-expansion (PR #4)

## Context

PR https://github.com/LocalRouter/LocalRouter/pull/4 (`feat/provider-expansion`)
adds a new DigitalOcean Gradient provider, lets Mistral target the Codestral
endpoint via an optional `base_url`, broadens the generic OpenAI-compatible
model-list parser to accept GitHub Models (bare array) and Cloudflare Workers
AI (`{result: [...]}`) shapes, and fixes GitHub Copilot device-flow OAuth bugs
(RFC 8628 §3.5 `slow_down` backoff, deadlock on a `RwLock` read guard held
across `.await`, terminal-error state cleanup). It also adds a `Drop` impl
for the MCP stdio transport. We are re-implementing it ourselves rather than
merging the branch so we can vet each change.

**Excluded from this re-implementation (decided):**
- All `.github/` Gemini-CLI workflow + command files (11 files, ~1500 lines).
- `run-release` `LOCALROUTER_KEYCHAIN=file` export — flips prod-release secret
  storage to plain-text-on-disk (CLAUDE.md flags this env var as risky).
- `.gitignore` additions for `src-tauri/gen/schemas/{acl-manifests,desktop-schema,macOS-schema}.json` — those files are currently tracked; ignoring would conflict.
- `.gitignore` for `crates/lr-catalog/catalog/modelsdev_raw.json` — also tracked
  today; catalog tests read it.
- `.gemini/` and `gha-creds-*.json` gitignore lines — only relevant to the
  excluded Gemini workflows.

**Prompt-injection / supply-chain notes:**
- Research sub-agent confirmed no injection content in the PR body, commits,
  or substantive code.
- No new Rust crates introduced (Cargo.toml diff is whitespace only).
- Sole npm change is bumping `@tauri-apps/plugin-dialog` `^2.6.0` → `^2.7.0`
  (official Tauri team package, low risk).
- Only new network host is `https://inference.do-ai.run/v1` (DigitalOcean's
  documented Gradient inference endpoint), reachable only on user-driven
  requests. Privacy policy preserved (no telemetry, no external assets).

---

## Implementation steps

Tracked via the in-conversation task list at execution time. Order is
roughly low-risk → higher-risk so each step lands cleanly.

### Step 1 — Lenient OpenAI-compat model-list parser

File: `crates/lr-providers/src/openai_compatible.rs`

- Make `OpenAIModel.object`, `created`, `owned_by` `Option<...>` with
  `#[serde(default)]`. They are currently required (with `#[allow(dead_code)]`)
  at lines 50–59, which makes parsing fail for GitHub Models / Cloudflare
  responses that omit them. The struct fields are not read anywhere, so
  changing them is safe.
- Around the existing two-shape fallback near line 283 (`OpenAIModelsResponse`
  → bare `Vec<OpenAIModel>`), add a third try for the Cloudflare envelope:
  `{ "result": [...] }`. Use a small private wrapper struct
  (`#[derive(Deserialize)] struct CfWrapper { result: Vec<OpenAIModel> }`).
- Order matters: `OpenAIModelsResponse` first (canonical), then bare array,
  then Cloudflare envelope. Surface only the final error if all three fail.
- Add a focused unit test per shape (3 tests) inside the same file.

### Step 2 — Mistral optional `base_url` (Codestral)

File: `crates/lr-providers/src/mistral.rs` and the matching factory in
`crates/lr-providers/src/factory.rs`.

- Add `base_url: Option<String>` field to `MistralProvider`.
- Add `with_base_url(api_key, base_url) -> AppResult<Self>` constructor; keep
  `new(api_key)` as a thin wrapper that calls it with `None`.
- Replace every reference to the existing `MISTRAL_API_BASE` const inside
  the provider body with `self.base_url.as_deref().unwrap_or(MISTRAL_API_BASE)`.
- Normalize input: strip trailing `/`, treat empty string as `None`,
  reject non-`http(s)://` schemes in factory `validate_config`.
- Factory updates:
  - Add `SetupParameter` for an optional `base_url` (with helper text mentioning
    Codestral's endpoint as an example).
  - In `create()`, pull `base_url` from config and forward to
    `with_base_url`.
- Roundtrip test: serialize/deserialize a Mistral config with a custom
  `base_url` and assert it survives.

### Step 3 — DigitalOcean Gradient provider

Several files. The catalog already has a `digitalocean` entry with API
`https://inference.do-ai.run/v1` and ~20 models, so wiring is mostly enum +
factory + UI.

3a. `crates/lr-config/src/types.rs` (~line 3404):
- Add `#[serde(rename = "digitalocean")] DigitalOcean` variant to
  `ProviderType`. Backward-compatible — does not rename or remove existing
  variants (per memory rule on serde compatibility).
- Extend the existing JSON + YAML roundtrip tests (~line 3947) to cover
  `DigitalOcean`.

3b. `crates/lr-providers/src/factory.rs`:
- Add `DigitalOceanProviderFactory` near the existing single-tenant factories
  (Cerebras / Groq / DeepInfra are the closest pattern — fixed base URL
  wrapper around `OpenAICompatibleProvider`).
- `provider_type()` returns `"digitalocean"`.
- `display_name()` `"DigitalOcean Gradient"`, `category()` matches the
  hosted-OpenAI-compatible bucket the others use.
- `setup_parameters()` exposes only `api_key` (label "Gradient Model Access
  Key" — DO docs distinguish this from a Personal Access Token).
- `create()` instantiates `OpenAICompatibleProvider::new(name, "https://inference.do-ai.run/v1".into(), Some(api_key))`.
- `catalog_provider_id()` returns `Some("digitalocean")` to map onto the
  existing catalog entry.
- `default_free_tier()` matches the others.

3c. `src-tauri/src/ui/commands_providers.rs` (~line 433):
- Add arm `"digitalocean" => ProviderType::DigitalOcean` to
  `provider_type_str_to_enum`.

3d. `src-tauri/src/main.rs`:
- Register the new factory alongside the existing ones.
- Add the inverse `ProviderType::DigitalOcean => "digitalocean"` arm if there
  is an enum-to-str helper there (per the PR research).

3e. `src/components/ServiceIcon.tsx` (~lines 24, 125):
- Add a `'digitalocean': 'digitalocean.png'` entry to `ICON_MAP` only if a
  bundled asset exists; otherwise stick to the emoji fallback.
- Add a `'digitalocean': '🌊'` entry to `EMOJI_MAP` (PR uses the wave emoji).

### Step 4 — GitHub Copilot OAuth fixes

File: `crates/lr-providers/src/oauth/github_copilot.rs`

Four discrete fixes:

4a. **Deadlock fix (lines ~152–164, 209+):** snapshot needed values from the
read guard, drop the guard, then perform HTTP work and any subsequent write
lock. The current code holds a `read().await` guard across the polling
request and only `drop`s explicitly on the success arm, so error arms
implicitly hold it through an `await` that later takes a write lock.
Rewrite as:
```text
let (device_code, interval, started_at, expires_in, next_poll_after) = {
    let flow = self.current_flow.read().await;
    let s = flow.as_ref().ok_or(...)?;
    (s.device_code.clone(), s.interval, s.started_at, s.expires_in, s.next_poll_after)
};
// no guard held past this point
```

4b. **`next_poll_after` rate gate:** add `next_poll_after: i64` (unix seconds)
to `FlowState`. On every poll, if `now < next_poll_after`, return `Pending`
*without* an HTTP call. After every server response, set
`next_poll_after = now + interval`.

4c. **`slow_down` backoff (RFC 8628 §3.5):** when the server returns
`slow_down`, increment `interval` by 5 seconds before updating
`next_poll_after`. Persist the new interval back into `FlowState` (write
lock for that field only).

4d. **Clear flow state on terminal errors:** on `expired_token`,
`access_denied`, or any unrecognized error code, set
`*self.current_flow.write().await = None` before returning the error.
Currently only success / cancel paths clear it.

Tests: add (or extend) unit tests that drive `poll_oauth_status` through:
- `authorization_pending` keeps state, sets `next_poll_after`.
- `slow_down` bumps interval by exactly 5 s.
- `expired_token` returns Error and clears state.
- A second `poll` called within `next_poll_after` returns Pending without
  hitting the HTTP mock.

### Step 5 — MCP stdio transport `Drop`

File: `crates/lr-mcp/src/transport/stdio.rs`

- Implement `Drop` for `StdioTransport`. The struct already uses
  `Arc<RwLock<Option<...>>>` for `child` and `reader_task`, and the child is
  spawned with `.kill_on_drop(true)`, so the bug is specifically the reader
  task lingering after the transport drops.
- In `Drop::drop`, take the `JoinHandle` out of `reader_task` (using
  `try_write` + `take`, no async needed) and call `.abort()` on it. Also
  pull the `Child` out and call `start_kill()` for parity with the explicit
  `kill()` method (defense in depth — `kill_on_drop` covers the common case,
  but if any other `Arc` clone keeps the child alive we still want a kill
  signal).
- No async work in `Drop`; only synchronous handle manipulation.

### Step 6 — Frontend dependency + scripts

File: `package.json`

- Bump `@tauri-apps/plugin-dialog` from `^2.6.0` → `^2.7.0`.
- Run `npm install` to regenerate `package-lock.json` (no manual edits).
- Add scripts:
  - `"debugging": "RUST_LOG=lr_providers=debug,info npm run tauri dev"`
  - `"dist": "npm run tauri build"`
- Verify these names don't collide with anything in the existing scripts
  (current set: `dev`, `build`, `preview`, `tauri`, `test:e2e*`).

### Step 7 — Plan doc

Create `plan/2026-05-04-DIGITALOCEAN_PROVIDER.md` (per CLAUDE.md's "every
plan must be saved to ./plan/"). Use `./copy-plan.sh` to convert *this*
plan after approval, or write it directly with sections matching repo
convention: **Context**, **Changes** (numbered, file paths + line numbers),
**Verification**.

---

## Critical files (modified)

| File | Step |
|------|------|
| `crates/lr-providers/src/openai_compatible.rs` | 1 |
| `crates/lr-providers/src/mistral.rs` | 2 |
| `crates/lr-providers/src/factory.rs` | 2, 3b |
| `crates/lr-config/src/types.rs` | 3a |
| `src-tauri/src/ui/commands_providers.rs` | 3c |
| `src-tauri/src/main.rs` | 3d |
| `src/components/ServiceIcon.tsx` | 3e |
| `crates/lr-providers/src/oauth/github_copilot.rs` | 4 |
| `crates/lr-mcp/src/transport/stdio.rs` | 5 |
| `package.json` + `package-lock.json` | 6 |
| `plan/2026-05-04-DIGITALOCEAN_PROVIDER.md` (new) | 7 |

Reused utilities (no new code needed):
- `OpenAICompatibleProvider::new(name, base_url, Some(api_key))` —
  `crates/lr-providers/src/openai_compatible.rs:21–46`. The DigitalOcean
  factory is a thin wrapper.
- Existing `SetupParameter` machinery for the Mistral `base_url` field.
- Existing `kill_on_drop(true)` on the stdio child — Drop impl just adds
  reader-task cancellation on top.

---

## Verification

Run from repo root, in this order:

1. **Type + build sanity (fast feedback):**
   - `npx tsc --noEmit` — confirms TypeScript types still compile after
     `ServiceIcon.tsx` edit.
   - `rustup run stable cargo check --workspace`.

2. **Unit + integration tests (target the touched crates first):**
   - `rustup run stable cargo test --package lr-providers openai_compatible::`
     — covers the three model-list shapes.
   - `rustup run stable cargo test --package lr-providers mistral::` —
     covers Codestral `base_url` plumbing.
   - `rustup run stable cargo test --package lr-providers oauth::github_copilot`
     — covers all four OAuth fixes.
   - `rustup run stable cargo test --package lr-config types::` — covers the
     new enum variant roundtrips.
   - `rustup run stable cargo test --package lr-mcp transport::stdio` — covers
     the Drop impl.

3. **Full pre-commit CI parity (per CLAUDE.md, before each commit):**
   - `rustup update stable`
   - `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`
   - `rustup run stable cargo fmt --all -- --check`
   - `rustup run stable cargo test --workspace`

4. **End-to-end smoke (Tauri dev):**
   - `cargo tauri dev`
   - In the UI, add a DigitalOcean provider via the wizard with a Gradient
     Model Access Key. Verify it appears in `GET http://localhost:3625/v1/models`
     output and that a `chat/completions` round-trip succeeds.
   - Add a Mistral provider with a custom Codestral `base_url`. Confirm it
     hits the Codestral endpoint (check log line for the URL with
     `RUST_LOG=lr_providers=debug` via the new `npm run debugging` script).
   - Trigger the GitHub Copilot device flow, intentionally wait past the
     `interval` to provoke `slow_down`, and confirm logs show interval bumped
     by 5 s and the next poll waits accordingly.
   - Stop and restart an MCP stdio server in the UI and confirm no orphaned
     reader tasks (check process tree).

5. **OpenAPI:** no endpoint surface changed; `GET /openapi.json` should be
   unchanged.

6. **Privacy policy check:** confirm no new outbound URLs other than
   `inference.do-ai.run` (only fired on user-driven requests) appear in
   the diff.
