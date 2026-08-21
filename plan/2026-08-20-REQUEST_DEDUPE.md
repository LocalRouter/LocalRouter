# Request Dedupe: detect multi-hop duplicates and passthrough

## Context

One logical LLM request can traverse LocalRouter several times — e.g. app → LR gateway client (`/v1/chat/completions`) → provider HTTP client honouring `HTTPS_PROXY` → LR MITM proxy (`lr-proxy`) → … → LR reverse-proxy listener (`lr-proxy::reverse`) → local provider. Today each hop is independent: each one runs the full "active" pipeline (compression, RouteLLM, guardrails, secret scan, firewall prompts, feature adapters, JSON repair) and each one records metrics, so one request shows up N times in dashboards and is counted N times in usage/rate-limit/cost stats.

Goal: every LocalRouter hop stamps the outgoing request with a trace header; any downstream LocalRouter hop that sees the header recognizes the request as already handled and **passes it through** — still parsed and shown in the Monitor (with a visible "duplicate hop" warning), but with no active rewriting/enforcement and no stats counting.

Decisions made with the user:
- **Detection: header only** (`X-LocalRouter-Trace`). No content-hash fallback (avoids false positives on identical retries).
- **Safety scans (guardrails, secret scan) are skipped** on duplicate hops, along with every other active behaviour. Routing/model resolution still runs (needed to forward).
- **Global toggle** `request_dedupe.enabled` (default `true`) with a Settings UI switch.

Implementation happens **in a git worktree** (user request): call `EnterWorktree` first, then `./copy-plan.sh work-in-a-worktree-zippy-glacier REQUEST_DEDUPE` before any code.

## Design

### Trace header
`X-LocalRouter-Trace: <trace_id>;hop=<n>` — `trace_id` is a UUID minted by the first LR hop; each forwarding hop re-emits it with `hop+1`. Parsing/formatting lives in one place (`lr-types` or a tiny new module `crates/lr-types/src/trace.rs`, since lr-server, lr-router, lr-providers and lr-proxy all need it and all already depend on `lr-types`):

```rust
pub const TRACE_HEADER: &str = "x-localrouter-trace";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestTrace { pub trace_id: String, pub hop: u32 }
impl RequestTrace {
    pub fn new() -> Self            // hop = 1, fresh uuid
    pub fn parse(header: &str) -> Option<Self>
    pub fn next_hop(&self) -> Self  // same id, hop+1
    pub fn header_value(&self) -> String
    pub fn is_duplicate(&self) -> bool { self.hop > 1 }  // semantics: inbound trace present ⇒ duplicate
}
```
Inbound rule at every hop: header present ⇒ this hop is a duplicate (`inbound.hop ≥ 1`); outbound = `inbound.next_hop()` or `RequestTrace::new()`. When `request_dedupe.enabled == false`: never stamp, never detect (treat inbound header as absent).

### Config (`crates/lr-config/src/types.rs`)
Add `RequestDedupeConfig { enabled: bool }` (Default → `true`), field `#[serde(default)] pub request_dedupe: RequestDedupeConfig` on `AppConfig` (next to `proxy: ProxyConfig`, ~L461). Additive only — no renames (config compat rule).

### Outbound stamping on the gateway path (lr-providers / lr-router)
Providers build all `reqwest::Client`s via `crates/lr-providers/src/http_client.rs` (`default_client`, `extended_client`, `discovery_client`) — the single choke point. Per-request header needs a middleware:
- Add workspace dep `reqwest-middleware` (same reqwest 0.12 line). `http_client.rs` returns `reqwest_middleware::ClientWithMiddleware` built with a `TraceHeaderMiddleware` that reads a `tokio::task_local! { static OUTBOUND_TRACE: Option<RequestTrace> }` and inserts the header when set.
- Change the ~20 provider structs' `client: Client` fields to `ClientWithMiddleware` (builder API is drop-in: `post/get/header/bearer_auth/json/multipart/send`). `send()` errors become `reqwest_middleware::Error` — existing `.map_err(|e| AppError::Provider(format!(..)))` sites keep compiling; `format_stream_error(&reqwest::Error)` is unaffected (response body streams are still `reqwest::Response`). Also `oauth/github_copilot.rs:123`, `openai_responses/http.rs`.
- `CompletionRequest` (`crates/lr-providers/src/lib.rs:1114`), `EmbeddingRequest`, audio request structs: add `#[serde(skip)] pub trace: Option<RequestTrace>`.
- `crates/lr-router/src/lib.rs`: in `execute_request` (L989) wrap `provider_instance.complete(modified_request)` in `OUTBOUND_TRACE.scope(request.trace.clone(), …)`; same for the streaming execute path, `complete_with_paid_fallback`/`stream_complete_with_paid_fallback` (L637/662), and the embeddings/transcribe/speech dispatches (~L2586/2694/2805). The scope wraps the `send()`; chunks polled later don't need it.

### Inbound detection on the gateway path (lr-server)
- New middleware `trace_middleware` in `crates/lr-server/src/lib.rs` `build_app` (next to `logging_middleware`, L323): reads `X-LocalRouter-Trace`, checks `state.config_manager.get().request_dedupe.enabled`, inserts `Extension<InboundTrace(Option<RequestTrace>)>`. Handlers currently take no `HeaderMap`; they gain `inbound: Option<Extension<InboundTrace>>` (chat, responses, completions, embeddings, audio).
- `run_turn_pipeline` (`routes/pipeline.rs:2028`) gets `trace: Option<RequestTrace>` → stored on `TurnContext` as `trace: RequestTrace` (outbound) + `is_duplicate: bool`, and copied into `provider_request.trace`. When `is_duplicate`:
  - skip `apply_firewall_request_edits`/auto-routing approval prompts (`apply_model_access_checks` still resolves the model/strategy but uses "allow without asking"),
  - skip `check_rate_limits` (L1675), `run_secret_scan_check` (L1344), `run_guardrails_scan` (L891), `run_prompt_compression` (L790), `spawn_routellm_classification` (L1869) — i.e. force `PipelineCaps { allow_compression:false, allow_routellm:false, .. }` plus new `allow_safety_scans:false`, `allow_firewall_prompts:false`, `allow_rate_limits:false`.
  - `convert_to_provider_request`: drop `extensions` (so feature adapters in `lr-router::execute_request` L1012-1061 don't run).
- `routes/chat.rs`: skip `handle_mcp_via_llm` branch (L137-152) when duplicate; skip `StreamingJsonRepairer` construction (L498/1475/1880) and `maybe_repair_json_content` (L1226 via `finalize.rs:46`) when duplicate.
- Stats exclusion — `routes/finalize.rs::finalize_metrics_and_monitor` (L161) and every direct `record_success/record_failure` site (`chat.rs:1109/1402/1756`, `completions.rs`, `embeddings.rs:184/240`, `audio.rs`): add `is_duplicate` to `FinalizeInputs`; when set, skip `metrics_collector.record_success/failure`, `record_feature_event`, and `emit_event("metrics-updated")`, but **still** call `access_logger.log_success` (with new field, below) and `complete_llm_call`. Rate-limit/free-tier charging in `lr-router` (`record_api_key_usage` L1120/1853/2259/2586/2694/2805, `free_tier.record_usage` L1136, streaming `wrap_stream_with_usage_tracking` L325): skip when `request.trace.as_ref().is_some_and(|t| t.hop > 1)` — gate through a helper `fn should_count(&request)`.
- Access log: `AccessLogEntry` (`crates/lr-monitoring/src/logger.rs:21`) gains `#[serde(default)] pub trace_id: Option<String>`, `#[serde(default)] pub duplicate_hop: Option<u32>`; `log_success/log_failure` take the trace.

### Monitor event
`MonitorEventData::LlmCall` (`crates/lr-monitor/src/types.rs:198`): add
```rust
#[serde(default, skip_serializing_if = "Option::is_none")] trace_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")] duplicate_hop: Option<u32>,  // Some(n) ⇒ hop n>1, not counted
```
Set in `monitor_helpers::emit_llm_call` (gateway) and in `lr-proxy` `PassiveInterceptor::emit_pending/record` (proxy + reverse proxy). Also push a `"duplicate-hop (not counted)"` label into `transformations_applied` so existing renderers show it. Mirror `trace_id`/`duplicate_hop` on `MonitorEventSummary` so the list can badge without fetching detail.

### HTTPS inspection proxy (`crates/lr-proxy`)
- `transport.rs::proxy_request` (L141): after buffering, read `X-LocalRouter-Trace` from `parts.headers` into a new `ObservedExchange.inbound_trace: Option<RequestTrace>`; then `parts.headers.insert(TRACE_HEADER, outbound.header_value())` (next hop or new). Gate on a new `ProxyContext.dedupe_enabled: Arc<AtomicBool>` (set by the launcher from config; updated by the settings command).
- `active.rs::ActiveInterceptor::on_request`: if `ex.inbound_trace.is_some()` → `RequestAction::Forward` without consulting the firewall.
- `passive.rs::finalize` (L134): skip `metrics.record_success/failure` when duplicate; `emit_pending/complete/record` populate `trace_id`/`duplicate_hop`.
- Websocket (`websocket.rs`) upgrade path: stamp header on the upgrade request too (passes through `proxy_request` already — verify).

### Reverse proxy (`crates/lr-proxy/src/reverse.rs`)
- `forward` (L194): same read-then-stamp on `parts.headers` (custom headers already survive — only `HOP_BY_HOP`/`accept-encoding`/`content-length` are stripped). Add `ReverseExchange.inbound_trace`. The app recorder in `src-tauri/src/launcher/reverse_proxy.rs` (wraps `PassiveInterceptor`) skips metrics + flags the event when duplicate. `ReverseProxy::new` gets the `dedupe_enabled` flag.

### Tauri / UI
- Commands (`src-tauri/src/ui/commands.rs`): `get_request_dedupe_config() -> RequestDedupeConfig`, `set_request_dedupe_enabled(enabled: bool)` (pattern: `set_max_coding_sessions` in `commands_coding_agents.rs:184` — `config_manager.update(..)` + `save()`), also flips the proxy/reverse-proxy `AtomicBool`s. Register in `src-tauri/src/main.rs`.
- `src/types/tauri-commands.ts`: `RequestDedupeConfig`, `SetRequestDedupeEnabledParams`; extend `MonitorEventSummary`/`MonitorEvent` data typing with `trace_id?`, `duplicate_hop?`.
- `website/src/components/demo/TauriMockSetup.ts`: mock both commands; add one mock monitor event with `duplicate_hop: 2`.
- Settings → `src/views/settings/server-tab.tsx`: new `Card` "Duplicate request detection" with a Switch + description ("When a request passes through LocalRouter more than once (gateway → HTTPS proxy → reverse proxy), downstream hops pass it through unmodified and don't count it twice").
- Monitor → `src/views/monitor/event-list.tsx`: amber `AlertTriangle` badge "dup" when `duplicate_hop`; `event-detail.tsx`: warning banner near the status badge (L150) "Duplicate hop N of trace <id> — passthrough, not counted in stats", with a "show all hops" action that sets the search filter to the trace id (needs `trace_id` included in `match_filter` text search in `lr-monitor/src/store.rs` and the client mirror in `hooks/useMonitorEvents.ts:17-32`).

## Files to touch (representative)
- New: `crates/lr-types/src/trace.rs`
- `crates/lr-config/src/types.rs`
- `crates/lr-providers/src/http_client.rs`, `lib.rs` (request structs), all `*.rs` providers (client field type), `Cargo.toml` (workspace `reqwest-middleware`)
- `crates/lr-router/src/lib.rs`
- `crates/lr-server/src/lib.rs`, `routes/{pipeline,chat,responses,completions,embeddings,audio,finalize,monitor_helpers}.rs`
- `crates/lr-monitoring/src/logger.rs`; `crates/lr-monitor/src/{types,store}.rs`
- `crates/lr-proxy/src/{transport,interceptor,active,passive,reverse}.rs`
- `src-tauri/src/ui/commands.rs`, `main.rs`, `launcher/{proxy,reverse_proxy}.rs`
- `src/types/tauri-commands.ts`, `src/views/settings/server-tab.tsx`, `src/views/monitor/{event-list,event-detail}.tsx`, `src/views/monitor/hooks/useMonitorEvents.ts`, `website/src/components/demo/TauriMockSetup.ts`

## Task list (todo)
1. EnterWorktree; `./copy-plan.sh work-in-a-worktree-zippy-glacier REQUEST_DEDUPE`
2. `RequestTrace` + header parse/format in lr-types (+ unit tests)
3. Config field + default
4. reqwest-middleware in `http_client.rs`, task-local, provider client type sweep; compile
5. `trace` on request structs; `OUTBOUND_TRACE.scope` at lr-router dispatch sites; skip usage charging on duplicates
6. lr-server: `trace_middleware`, handler params, pipeline caps/skips, chat.rs skips (MCP-via-LLM, JSON repair), finalize stats exclusion, access log fields
7. Monitor event fields (Rust) + store text filter
8. lr-proxy: transport read/stamp, ActiveInterceptor passthrough, passive metrics skip, reverse.rs, launcher flags
9. Tauri commands + TS types + demo mock
10. Settings card + Monitor badge/banner
11. Final: plan review, test coverage review, bug hunt, CI-parity checks, commit

## Tests
- lr-types: header round-trip, malformed header → `None`, `next_hop`.
- lr-providers: middleware injects header when task-local set, not when unset (use a local `axum`/`wiremock`-style echo — check what test HTTP helpers already exist in the crate).
- lr-server: integration test through `build_app` — request with `X-LocalRouter-Trace` against a mock provider asserts (a) upstream receives `hop=2`, (b) `metrics_collector` totals unchanged, (c) monitor event has `duplicate_hop: Some(2)`, (d) no guardrail/secret-scan events emitted; request without header asserts outbound `hop=1` and normal counting; same with config disabled asserts no header + normal counting.
- lr-proxy: extend `passive.rs`/`active.rs` tests (`records_messages_exchange_to_monitor` pattern) with `inbound_trace` set → no metrics, `Forward` despite a denying firewall; reverse.rs forward stamps header.
- Frontend: `npx tsc --noEmit`.

## Verification (end-to-end)
1. `cargo tauri dev --no-watch`; enable HTTPS proxy mode for a client and set `HTTPS_PROXY` on LocalRouter's own process (or run a reverse-proxy client wrapping Ollama whose LR provider instance points at the relocated port).
2. `curl localhost:33625/v1/chat/completions` with a client key → Monitor shows hop 1 normally and hop 2 with the amber duplicate badge; Dashboard request/token counters increase by exactly one request.
3. Toggle the setting off → both hops count, no badge; toggle back on.
4. `curl -H 'X-LocalRouter-Trace: abc;hop=1'` directly → event flagged, no guardrail/secret-scan/compression events, no metric increment.
5. CI parity: `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, targeted `cargo test -p lr-types -p lr-providers -p lr-proxy -p lr-server -p lr-router`.
