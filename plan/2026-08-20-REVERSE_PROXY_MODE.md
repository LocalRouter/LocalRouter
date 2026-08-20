# Reverse Proxy Client Mode (wrap a local LLM provider)

**Date**: 2026-08-20

## Progress checklist

- [x] Plan saved to `plan/`
- [x] lr-config: `LlmMode::ReverseProxy`, `ClientReverseProxy` struct, predicates, validation
- [x] lr-proxy: `reverse` module — plain-HTTP reverse-proxy listener with streaming pass-through + capture
- [x] Monitoring: record proxied exchanges (Ollama-native NDJSON + OpenAI formats) to monitor/metrics
- [x] src-tauri: `ReverseProxyService` (per-client listeners, config-driven sync)
- [x] src-tauri: `reverse_setup.rs` — per-provider relocation plans (auto/one-off/manual + undo)
- [x] Tauri commands (setup/status/configure/unconfigure) + create_client support
- [x] Server gating: deny native /v1 for reverse-proxy clients with a clear reason
- [x] Frontend: LlmMode union, templates (6 local providers), mode selector, wizard, HowToConnect setup UI, client-detail gating
- [x] Demo mock updates (`website/src/components/demo/TauriMockSetup.ts`)
- [x] Tests (config validation, forwarder pass-through/streaming, relocation plans)
- [x] Mandatory final steps: plan review, test-coverage review, bug hunt, commit
- [x] Spin up debug LocalRouter (`cargo tauri dev --no-watch`) for user testing

## Context / use case

The user runs a local Ollama on `11434` with many apps pointed at it and no
visibility. New client LLM mode **Reverse Proxy**: move the provider to a
different port (e.g. `11435`), and LocalRouter listens on the **original** port
(`11434`), transparently forwarding **all** traffic (native `/api/*` and
OpenAI `/v1/*`) to the relocated upstream while recording every exchange in the
Monitor/metrics and attributing it to the client. Combination of gateway mode
(a listener apps point at) and HTTPS proxy mode (transparent pass-through +
passive inspection) — but plain-HTTP, port-impersonating, and auth-free
(client identity is implied by the listener).

Supported provider templates (all existing local providers): **Ollama**
(11434→11435), **LM Studio** (1234→1235), **Jan** (1337→1338), **GPT4All**
(4891→4892), **LocalAI** (8080→8081), **llama.cpp** (8080→8082).

## Design

### 1. Config (`crates/lr-config`)

- `LlmMode` (`types.rs:2825`): add `ReverseProxy` variant (`reverse_proxy` in
  serde). Never remove/rename existing variants (compat rule).
- New struct near `Client`:
  ```rust
  pub struct ClientReverseProxy {
      pub listen_host: String,          // default "127.0.0.1"
      pub listen_port: u16,             // original provider port, e.g. 11434
      pub upstream_url: String,         // relocated provider root, e.g. http://127.0.0.1:11435
      pub provider_instance: Option<String>, // linked provider instance name (kept in sync on relocation)
  }
  ```
- `Client`: `#[serde(default, skip_serializing_if = "Option::is_none")] pub reverse_proxy: Option<ClientReverseProxy>`.
- Predicates (`types.rs:3069+`): `llm_reverse_proxy_enabled()`. `effective_client_mode()` collapses like proxy.
- Validation (`validation.rs:301 validate_client_modes` + new checks):
  - `ReverseProxy` requires `reverse_proxy` config present.
  - `ViaLlm` still requires `Gateway` (reverse proxy incompatible with via-llm).
  - Listen port must not equal `server.port` or `proxy.port`; no duplicate
    listen ports across **enabled** reverse-proxy clients.
- No CONFIG_VERSION bump needed (purely additive, serde defaults).

### 2. Data path (`crates/lr-proxy/src/reverse.rs`, new module)

Plain-HTTP reverse proxy on hyper 1.x (already a dep) + reqwest for upstream:

- `ReverseProxyHandle::start(host, port, upstream_url, recorder) -> Result<Handle>`
  — binds TcpListener, accept loop, per-conn `hyper::server::conn::http1`
  serve with a `service_fn` forwarder; graceful shutdown via oneshot (mirrors
  `ProxyManager::serve` / `ProxyService` shape).
- Forwarder: buffer request body (16 MB cap), rebuild upstream request (same
  method/path/query, hop-by-hop headers stripped, Host rewritten), send via
  shared `reqwest::Client`, **stream** the response back
  (`http_body_util::StreamBody` over `bytes_stream()`) while teeing up to
  `MAX_CAPTURE` (1 MiB, same constant policy as transport.rs) into a capture
  buffer. On stream end, hand `ReverseExchange { method, path, status,
  request_body, response_body(truncated flag), duration, streamed }` to the
  `ReverseRecorder` trait (async, spawned — never blocks the data path).
- Upstream connect failure → 502 with a JSON body naming the upstream, so
  misconfigured relocation is diagnosable from the calling app.
- No WebSocket/upgrade support in v1 (local providers don't use it); upgrade
  requests are forwarded as normal requests.

### 3. Recording (src-tauri glue)

`ReverseRecorder` impl in `src-tauri/src/launcher/reverse_proxy.rs`, modeled on
`PassiveInterceptor`: parse known endpoints, build monitor `LlmCall` +
metrics; unknown paths recorded as lightweight passthrough events (or skipped —
decide during impl based on monitor event model):
- Ollama native: `/api/chat`, `/api/generate` (NDJSON stream; final object
  carries `prompt_eval_count`/`eval_count` → token counts), `/api/embed`.
- OpenAI-style: `/v1/chat/completions`, `/v1/completions`, `/v1/embeddings`
  (SSE/JSON `usage`), reusing `lr-proxy` OpenAI parsing where practical.
- Cost is 0 (local providers); provider name from linked instance.

### 4. Listener lifecycle (`src-tauri/src/launcher/reverse_proxy.rs`)

`ReverseProxyService` (managed Tauri state, mirrors `ProxyService`):
- `HashMap<client_id, Running { port, shutdown, error }>` behind a Mutex.
- `sync(config)` reconciles listeners with all enabled clients in
  `ReverseProxy` mode: start missing, stop removed/changed (port/upstream
  diff), record per-client bind errors (port still occupied by the provider →
  surfaced in status, not fatal).
- Called at startup (`main.rs`, after main server start) and from the
  clients/config-changed path (`sync_all_clients` call sites).
- Bind retry: a few short retries on `AddrInUse` when started right after a
  relocation (old process releasing the port).

### 5. Relocation setup (`src-tauri/src/launcher/reverse_setup.rs`)

Per-template plan struct in the spirit of `proxy_setup::ProxyPlan`:

```rust
pub struct ReversePlan {
    pub provider_label: &'static str,
    pub auto: Option<AutoRelocate>,     // executable relocation
    pub oneoff_command: Option<String>, // copyable command
    pub manual_steps: Vec<String>,      // GUI instructions
    pub undo: Option<AutoRelocate>,
    pub notes: Vec<String>,
    pub restart_hint: Option<String>,
}
```

Per provider (auto only where reliable; everything else one-off/manual):
- **ollama** (macOS auto): `launchctl setenv OLLAMA_HOST 127.0.0.1:<new>` +
  quit/reopen Ollama.app (`osascript quit` + `open -a Ollama`); undo =
  `launchctl unsetenv OLLAMA_HOST` + restart. Linux: manual systemd
  drop-in instructions (`Environment="OLLAMA_HOST=..."`). One-off:
  `OLLAMA_HOST=127.0.0.1:<new> ollama serve`.
- **lmstudio**: auto via `lms` CLI when on PATH: `lms server stop && lms
  server start --port <new>`; else manual (GUI server-port setting).
- **jan**, **gpt4all**: manual (GUI-only port settings).
- **localai**: one-off `local-ai run --addr 127.0.0.1:<new>` + manual.
- **llamacpp**: one-off `llama-server --port <new>` + manual.

`configure_client_reverse_proxy` flow:
1. Update linked provider instance `base_url` → upstream URL (registry
   hot-applies; catalog/health keep working).
2. Run auto relocation if available (else return the manual/one-off plan —
   status stays "waiting for upstream").
3. Poll upstream health briefly (reuse the provider `/models` / `/api/tags`
   style probe).
4. Start (or retry) the listener on the original port via
   `ReverseProxyService`.
5. Return `LaunchResult` + status.

`unconfigure` reverses: stop listener, run undo relocation, restore provider
`base_url`.

### 6. Tauri commands (`commands_clients.rs`, registered in `main.rs`)

- `get_client_reverse_proxy_setup(client_id) -> ReverseProxySetupInfo`
  (ports, upstream, plan: auto-supported, one-off command, manual steps,
  notes).
- `get_client_reverse_proxy_status(client_id) -> ReverseProxyStatus`
  { listener_running, listener_error, upstream_healthy, listen_port,
  upstream_url }.
- `configure_client_reverse_proxy(client_id) -> LaunchResult`.
- `unconfigure_client_reverse_proxy(client_id) -> LaunchResult`.
- `set_client_reverse_proxy_config(client_id, listen_port, upstream_url)`.
- `create_client` (`:147`): optional `reverse_proxy` param
  `{ listenPort, upstreamUrl, providerInstance? }`; template ids
  (`reverse-ollama` etc.) get backend defaults when omitted.

### 7. Server gating

`routes/helpers.rs:226/:234`: reverse-proxy clients denied on native `/v1`
with reason `llm_reverse_proxy_client_native` (parallel to proxy clients).

### 8. Frontend

- `types/tauri-commands.ts`: `LlmMode` union + new response/param types.
- `ClientTemplates.tsx`: new category `local_providers`; 6 templates
  (`reverse-ollama`, `reverse-lmstudio`, `reverse-jan`, `reverse-gpt4all`,
  `reverse-localai`, `reverse-llamacpp`) with `supportsReverseProxy: true`,
  `reverseProxy: { listenPort, upstreamPort }` metadata, icons, docs URLs.
- `ClientModeSelector.tsx`: `reverse_proxy` LLM option ("Reverse Proxy — wrap
  a local provider's port"); gating: only for templates with
  `supportsReverseProxy` (and the reverse templates default to it, MCP off).
- Wizard: `templateDefaultModes` returns `reverse_proxy`/`off` for these
  templates; `createClient` passes the template's port defaults.
- `HowToConnect.tsx`: `ReverseProxySetup` section (status card with
  listener/upstream health, Configure/Undo buttons, one-off + manual tabs,
  port summary "apps keep using :11434 — nothing to change in your apps").
- `client-detail.tsx` tab gating (models tab restricted view like proxy;
  no Try-It-Out).
- Demo mocks in `website/src/components/demo/TauriMockSetup.ts`.

### 9. Tests

- lr-config: validation legal/illegal combos, port-conflict rules, serde
  round-trip of new fields.
- lr-proxy reverse: end-to-end forward test against an in-process axum dummy
  upstream (status/headers/body pass-through, streaming chunk pass-through,
  502 on dead upstream, capture truncation).
- reverse_setup: per-template plan content unit tests.
- helpers gating test for the new denial reason.

## Mandatory final steps

1. **Plan Review** — re-read this plan vs. implementation; close gaps.
2. **Test Coverage Review** — add tests for uncovered paths.
3. **Bug Hunt** — fresh-eyes pass over new code.
4. **Commit** — only files touched by this work; CI-parity checks first
   (`rustup run stable cargo clippy --workspace --all-targets -- -D warnings`,
   `fmt --check`, targeted tests).
5. Spin up `cargo tauri dev --no-watch` for interactive testing.


## Implementation notes (added during build)

Deviations and additions relative to the plan above:

- **Ollama's native protocol is now a first-class wire format.** `lr-proxy`
  gained `ollama.rs` (`/api/chat`, `/api/generate`; NDJSON reconstruction,
  `prompt_eval_count`/`eval_count` → tokens, `thinking` → reasoning preview) and
  `WireFormat::Ollama`. `ObservedExchange` gained `response_is_ndjson`,
  `provider_override`, `source` and `error`, and `PassiveInterceptor` now
  implements `ReverseRecorder` directly — so reverse-proxied calls produce the
  *same* monitor events and metrics as MITM-proxied ones, tagged
  `LlmCallSource::ReverseProxy`.
- **Only inference calls become monitor events.** `/api/tags`, `/api/pull`,
  `/v1/models`, health probes etc. are forwarded but not recorded, matching the
  existing gateway/MITM policy. Embeddings are likewise not recorded (the
  existing `wire::detect` doesn't cover them either).
- **Status is reported as three separate facts**, not one boolean: listener
  bound / provider relocated / provider instance retargeted. That triple is what
  actually goes wrong, so the UI shows each one.
- **Relocation is per-provider and honest about it.** Automatic only where it is
  reliable: Ollama on macOS (`launchctl setenv` + app restart) and Windows
  (`setx`), LM Studio via the `lms` CLI when present. Linux Ollama (systemd,
  needs root), Jan and GPT4All (GUI-only) get manual steps; LocalAI and
  llama.cpp get a one-off command. The exact commands are shown in the UI
  before the user runs them.
- **`upstream_url` is plain `http://` only** — every supported local provider
  speaks it, and pretending to support `https://` upstreams would have meant an
  untested TLS path.
- **Bugs found and fixed during the hunt:**
  - self-forwarding loop: an upstream equal to the listen address (including the
    `localhost`/`127.0.0.1` spelling mix) is now rejected by validation;
  - listener churn: `sync()` compared the *normalized* upstream against the
    configured one, restarting the listener on every sync whenever the URL
    carried a path — it now compares the configured form;
  - mode-switch dead end: switching an existing client to reverse-proxy mode in
    the UI had no binding and failed validation; the template's defaults are now
    filled in, mirroring creation;
  - a provider entry with no `base_url` produced `http://:11435`; it now falls
    back to `127.0.0.1`;
  - cloning a reverse-proxy client would have duplicated a port binding; clones
    now drop the binding and fall back to gateway mode.
- **Not done (deliberate):** no firewall/model enforcement on the reverse path
  (it is verbatim pass-through, so the Models tab is hidden for these clients
  rather than shown and not enforced); no tray entries; no websocket upgrade
  support (no local provider needs it).


## Post-testing fixes (live run against a real Ollama)

The first live attempt failed with "Address already in use". Four bugs, found by
diagnosing the machine rather than re-reading the code:

1. **Ollama refuses a scripted quit.** `osascript -e 'quit app "Ollama"'`
   returns `User canceled. (-128)`. That step was marked best-effort, so the
   failure was swallowed, `open -a Ollama` then no-op'd (the app was still
   running), and the sequence reported success while nothing had moved.
2. **The GUI app must restart, not just the server.** `launchctl setenv` only
   seeds *newly launched* processes; the running app's environment was fixed
   (verified: zero `OLLAMA_HOST` entries in its env), so any server it respawned
   would keep the old port. Escalation is now `pkill -x Ollama` + `open -a
   Ollama`, and it is shown in the UI up front, marked "only if the port is
   still held".
3. **LocalRouter's own listener held the port being observed.** After a restart,
   startup sync binds the original port; the relocation then waited for *that*
   port to be released and blamed the provider when it never was. `configure`
   now releases its own listener first.
4. **`pkill` exits non-zero when nothing matches**, which is a success for our
   purposes. Every stop command is now best-effort and the *port state* is the
   only gate.

The underlying design error was trusting exit codes. `relocate()` now verifies
outcomes — old port released, new port answering — and is idempotent (a wrap
that is already in the desired state returns immediately instead of restarting
a healthy provider).

Cross-provider follow-ups from the same pass:
- `supports_undo()` required a non-empty `unconfigure`, which silently dropped
  LM Studio's Undo button (its port is a CLI argument, so it has nothing to
  un-configure). Undo is now offered for anything we can relocate.
- Added tests asserting, for *every* provider in `DEFAULT_PORTS`: automation
  implies reversibility, all stop commands are best-effort, and no provider
  leaves the user with a blank panel.

**Try it out** (`test_client_reverse_proxy`): calls the *wrapped port* — the
address the user's apps use — and reports status, latency and discovered
models. Hitting the listener rather than the provider is deliberate: a pass
proves listener → forwarding → provider, not merely that the provider is alive.

Verified end to end against a real Ollama: native `/api/chat` NDJSON streaming
and OpenAI-shaped `/v1/chat/completions` both flow through the wrap on 11434 to
Ollama on 11435.
