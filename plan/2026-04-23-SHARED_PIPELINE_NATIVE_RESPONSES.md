# Shared Pipeline + Native Responses Refactor

## Status (2026-04-23)

- ✅ **Commit 1 landed** (ed08d705) — `routes/finalize.rs` created;
  `/v1/responses` telemetry gap fixed (streaming + non-streaming).
- ✅ **Commit 5 partial landed** (8a354791) — `/v1/completions`
  now emits `update_llm_call_routing` on all three paths. Full
  feature-parity uplift (compression / RouteLLM / MCP-via-LLM /
  free-tier fallback / JSON repair) still deferred behind the
  pipeline.rs extraction.
- ✅ **Commit 6 landed** (89380cc6 non-streaming, 7150d49b streaming)
  — ChatGPT Plus native Responses pass-through. Upstream SSE
  envelopes + non-streaming JSON are stashed on
  `CompletionResponse.extensions` / `CompletionChunk.extensions`
  under `NATIVE_RESPONSES_{API,SSE}_EXT_KEY`; `routes/responses.rs`
  prefers these over the lossy translator so reasoning items,
  encrypted-content carry-over, and built-in tool results reach
  clients intact. Only `response.id` / `response_id` is rewritten
  to LocalRouter's own id so session continuations still work.
- ✅ **Commit 7 landed** (0dec0d95) — MCP-via-LLM explicit session
  key via `previous_response_id` (3 new tests).
- ✅ **Finalize-helper migration landed** across all three endpoints
  and all 5 chat handlers. Non-streaming + streaming telemetry in
  chat.rs (0cf6e4b9 / db8af8c4 / 5402a77d) and completions.rs
  (64c8d212 / 4e1df314) now routes through the shared
  `finalize_metrics_and_monitor` +
  `update_response_body_and_record_generation` +
  `finalize_streaming_at_end` helpers. The MCP-via-LLM streaming
  path joined via a new `skip_monitor_completion: bool` flag on
  `FinalizeInputs` (5402a77d) — orchestrator owns the monitor
  event, but cost / metrics / tray / access log / generation row
  all fire through the shared helper.
- ✅ **Commit 2 landed** (ac646b37) — `routes/pipeline.rs` extracted
  from chat.rs (~1,600 LOC of 12 pipeline stages moved verbatim).
  chat.rs dropped from 4,715 to 3,135 LOC (-33.5%) in that commit
  alone; session total: 5,165 → 3,135 LOC (-39.3%).
- ✅ **`run_turn_pipeline` entry point landed** (291fbb51 /
  5576268e / 830c0244) — `PipelineCaps::{chat, responses,
  completions}` + `TurnContext` + a single `run_turn_pipeline()`
  call that drives all 7 stages (validate → access checks → rate
  limits → secret scan → guardrails → compression → RouteLLM →
  convert). All three endpoints adopted:
  - `/v1/chat/completions` uses `PipelineCaps::chat()` (parallel
    guardrails, compression spawned, RouteLLM spawned).
  - `/v1/responses` uses `PipelineCaps::responses()` (sequential
    guardrails, compression spawned, no RouteLLM).
  - `/v1/completions` uses `PipelineCaps::completions()` via a
    `CompletionRequest → ChatCompletionRequest` adapter — inherits
    the full feature set (compression, RouteLLM, auto-routing
    firewall, secret scan, guardrails). **Commit 5 landed**
    (60f71a6b).
- ⏭️ Commit 3 (dispatch.rs extraction) — the four handler variants
  (streaming × parallel) could still collapse. Structural only,
  would let chat.rs shrink further. Remaining open work.

## End-state summary

Every user-visible goal from the original plan is landed. Only the
dispatch.rs structural extraction remains open (purely organizational
— no behavior change).

| Metric                | Session start | End     | Δ        |
|-----------------------|---------------|---------|----------|
| chat.rs               | 5,165         | 2,947   | -42.9%   |
| completions.rs        | 2,118         | 1,600   | -24.5%   |
| responses.rs          | 1,032         | 1,453   | +40.8% ¹ |
| pipeline.rs (new)     | —             | ~1,960  | —        |
| finalize.rs (new)     | —             | ~460    | —        |

¹ Grew because it now carries full telemetry, native pass-through
(non-streaming + streaming), session persistence, and the dispatch
variant formerly implicit in chat.rs.

## Context

LocalRouter exposes three LLM HTTP surfaces — `/v1/chat/completions`,
`/v1/responses`, `/v1/completions` — that all do essentially the same
"proxy plus value-add" work: auth, rate limits, guardrails, secret
scan, compression, RouteLLM, auto-router firewall, model firewall,
MCP-via-LLM orchestration, cost accounting, access logging, metrics,
tray updates, monitor events, JSON repair, free-tier fallback.

Today this work is implemented **three times**:

- `chat.rs` (5,143 LOC) is the canonical stack.
- `responses.rs` (newly built) calls some of chat.rs's `pub(crate)`
  helpers but bypasses the handler layer — so it skips: access
  logger, metrics recorder, tray graph tokens, `update_llm_call_routing`
  / `update_llm_call_response_body` / `complete_llm_call` events,
  cost calc, JSON repair, free-tier fallback.
- `completions.rs` has its own inline copies of everything and misses:
  compression, RouteLLM, auto-router firewall, model firewall, MCP-via-
  LLM dispatch, free-tier fallback, `update_llm_call_routing`, JSON
  repair.

Worse, `/v1/responses` against a Responses-native provider (ChatGPT
Plus OAuth) round-trips through ChatCompletions as the internal
representation, doubling the translation cost. And the MCP-via-LLM
orchestrator matches sessions via content hashing even when the client
passed a deterministic `previous_response_id`.

**Goal:** factor the proxy-plus features into three shared modules
(`pipeline`, `dispatch`, `finalize`) that all three endpoints consume
identically, eliminate the double translation for Responses-native
providers, and teach the MCP-via-LLM orchestrator to honor explicit
session keys when available. Preserve `CompletionRequest` /
`CompletionResponse` as the internal provider interchange (touching
those costs weeks and buys nothing this refactor needs).

## Non-goals

- Changing `CompletionRequest` / `CompletionResponse` shapes or the
  `ModelProvider` trait. 200+ files reference these; not in scope.
- Full protocol-neutral internal representation (`ResponseItem`-based
  orchestrator). Tier 3 from the prior discussion — defer.
- Promoting `ResponseItem` / `OutputItem` to workspace-neutral types.

## Architecture

### Canonical internal form: `ChatCompletionRequest`

Every endpoint translates its wire body to `crates/lr-server/src/
types.rs::ChatCompletionRequest` at the adapter boundary. All shared
pipeline helpers already consume this shape (we converged there during
the earlier `/responses` work). Legacy `/v1/completions` synthesizes a
single `ChatMessage { role: "user", content: Text(prompt) }` and feeds
it through the same canonical path (`completions.rs:752–771` already
does the equivalent inline).

### Three shared modules

**`crates/lr-server/src/routes/pipeline.rs`** — pre-LLM stages.

```rust
pub struct PipelineCaps {
    pub allow_routellm: bool,
    pub allow_compression: bool,
    pub allow_mcp_via_llm: bool,
    pub allow_model_firewall: bool,
    pub allow_free_tier_fallback: bool,
    pub parallel_guardrails_ok: bool,
}
impl PipelineCaps {
    pub fn chat() -> Self          // everything on
    pub fn responses() -> Self     // everything on (after answers: yes, gain features)
    pub fn completions() -> Self   // everything on (after answers: yes, gain features)
}

pub struct TurnContext {
    pub chat_req: ChatCompletionRequest,                  // compression may mutate
    pub provider_request: lr_providers::CompletionRequest,
    pub session_id: String,
    pub endpoint: &'static str,
    pub compression_tokens_saved: u64,
    pub routellm_routing: Option<PreComputedRouting>,
    pub guardrail_gate: GuardrailGate,
    pub client_mode: lr_config::ClientMode,
    pub mcp_session_key: Option<String>,                  // NEW: set from previous_response_id
    pub caps: PipelineCaps,
}

pub enum GuardrailGate {
    Disabled,
    Passed,
    Pending(tokio::task::JoinHandle<ApiResult<Option<SafetyCheckResult>>>),
}

pub async fn run_turn_pipeline(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    mut chat_req: ChatCompletionRequest,
    llm_guard: &mut LlmCallGuard,
    caps: PipelineCaps,
    mcp_session_key: Option<String>,
    endpoint: &'static str,
) -> ApiResult<TurnContext>;
```

Runs in order, each stage checking `caps`:
validate → access_checks → rate_limits → secret_scan → (spawn)
compression → (spawn) RouteLLM → spawn guardrails (parallel-safe)
or await (sequential) → `convert_to_provider_request`. Existing
helpers move from `chat.rs` into `pipeline.rs` (visibility flips only,
no logic changes).

**`crates/lr-server/src/routes/dispatch.rs`** — routing + retry.

```rust
pub enum TurnOutcome {
    NonStreaming(lr_providers::CompletionResponse),
    Streaming(BoxStream<'static, AppResult<lr_providers::CompletionChunk>>),
}

pub async fn dispatch_turn(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    ctx: &mut TurnContext,
    llm_event_id: &str,
) -> ApiResult<TurnOutcome>;
```

Collapses the four handler variants in `chat.rs` (streaming ×
parallel) into two (streaming / non-streaming) — the
parallel/sequential choice becomes an internal concern, gated by
`has_side_effects(&ctx.chat_req) || !ctx.caps.parallel_guardrails_ok`.
Owns:

- The MCP-via-LLM vs router branch (reads `ctx.client_mode` and
  `ctx.caps.allow_mcp_via_llm`).
- The free-tier-fallback retry loop (extracted from
  `check_free_tier_fallback` at `chat.rs:2858`).
- Awaiting the guardrail gate before mutating side effects.
- Passing `ctx.mcp_session_key` into
  `McpViaLlmManager::handle_request` (see "MCP-via-LLM session key"
  below).

**`crates/lr-server/src/routes/finalize.rs`** — post-LLM processing.

```rust
pub async fn finalize_non_streaming(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    ctx: &TurnContext,
    response: &mut lr_providers::CompletionResponse,  // mut for JSON repair
    llm_event_id: &str,
    started_at: Instant,
) -> ApiResult<()>;

pub fn wrap_stream_with_finalize(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    ctx: TurnContext,
    llm_event_id: String,
    stream: BoxStream<'static, AppResult<CompletionChunk>>,
) -> BoxStream<'static, AppResult<CompletionChunk>>;
```

Both variants run the same telemetry: cost calc (via provider
pricing in `lr-providers`), generation-tracker entry, access log
write, metrics recorder update, tray graph token push,
`metrics-updated` tray event, `update_llm_call_routing`,
`update_llm_call_response_body`, `complete_llm_call`, and
`maybe_repair_json_content` (gated on response_format in
`ctx.chat_req`). Neither knows the wire format — each adapter wraps
the returned stream into its own SSE shape downstream.

### Adapters become thin

Each route file ends up ~30–40 lines:

```rust
// responses.rs (shape same for chat.rs and completions.rs)
pub async fn create_response(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(req): Json<CreateResponseRequest>,
) -> ApiResult<Response> {
    let session_id = Uuid::new_v4().to_string();
    let response_id = format!("resp_{}", Uuid::new_v4().simple());
    let mut guard = emit_llm_call(&state, client_auth.as_ref(), Some(&session_id),
                                  "/v1/responses", &req.model, req.stream, &request_json);
    state.record_client_activity(&auth.api_key_id);

    // Load session (if any) and build canonical request
    let (prior_messages, prior_tools, mcp_session_key) = load_session(&state, &req);
    let chat_req = build_chat_completion_request(&req, prior_messages, prior_tools)?;

    let mut ctx = run_turn_pipeline(&state, &auth, client_auth.as_ref(),
                                    chat_req, &mut guard, PipelineCaps::responses(),
                                    mcp_session_key, "/v1/responses").await?;
    let event_id = guard.into_event_id();
    let started = Instant::now();

    match dispatch_turn(&state, &auth, client_auth.as_ref(), &mut ctx, &event_id).await? {
        TurnOutcome::NonStreaming(mut resp) => {
            finalize_non_streaming(&state, &auth, client_auth.as_ref(), &ctx,
                                   &mut resp, &event_id, started).await?;
            persist_session(&state, &ctx, &resp, &response_id, /*store=*/req.store);
            Ok(Json(completion_to_response_object(&resp, &response_id, /*...*/)).into_response())
        }
        TurnOutcome::Streaming(stream) => {
            let wrapped = wrap_stream_with_finalize(state.clone(), auth.clone(), client_auth,
                                                    ctx, event_id, stream);
            Ok(responses_sse_wrap(wrapped, response_id, req).into_response())
        }
    }
}
```

### Native Responses pass-through

**`crates/lr-providers/src/lib.rs`** adds:

```rust
#[async_trait]
pub trait ResponsesProvider: Send + Sync {
    async fn create_response(
        &self,
        request: ResponsesApiRequest,
    ) -> AppResult<ResponseObject>;

    async fn stream_response(
        &self,
        request: ResponsesApiRequest,
    ) -> AppResult<BoxStream<'static, AppResult<ResponsesSseEnvelope>>>;
}

// Blanket impl: every ModelProvider gets a Responses surface for free
// via the existing bi-directional translators in openai_responses/.
impl<P: ModelProvider + ?Sized> ResponsesProvider for P { /* default body */ }
```

**`OpenAIProvider` override** (`crates/lr-providers/src/openai.rs`):
when `is_chatgpt_backend()`, `create_response` / `stream_response`
call `openai_responses::http::create_response` /
`stream_response` directly — no ChatCompletions round-trip.

**`routes/responses.rs`** checks whether the resolved provider's
native `create_response` is preferred over the router path. If yes
(and MCP-via-LLM is disabled for this turn), bypass
`convert_to_provider_request` and call the trait. Responses API
clients of ChatGPT Plus then get the full native feature set (encrypted
reasoning carry-over, built-in tools, background mode, `include[]`)
without the double-translation cost described in the earlier loss
inventory.

### MCP-via-LLM session key

**Cut points** (from orchestrator audit):

1. `CompletionRequest` wrapper gains `session_key: Option<String>`
   (or a parallel param into manager methods — prefer the param since
   `CompletionRequest` is workspace-wide).
   - Thread through `McpViaLlmManager::handle_request` /
     `handle_streaming_request` at
     `crates/lr-mcp-via-llm/src/manager.rs:344` and `:545` — add
     `session_key: Option<String>` parameter.

2. `McpViaLlmManager::get_or_create_session` at `manager.rs:149`
   short-circuits on explicit session_key:
   ```rust
   if let Some(key) = session_key {
       if let Some(existing) = self.sessions_by_client
           .get(&client_id)?.iter()
           .find(|s| s.read().explicit_key.as_deref() == Some(key)) {
           return existing.clone();
       }
   }
   // fallback: existing hash-matching at :176–218
   ```

3. `McpViaLlmSession` at `session.rs:72` gains
   `explicit_key: Option<String>`. When the manager creates a new
   session with an explicit key, stamp it. Hash-matching remains the
   fallback path (for chat.rs and legacy completions).

This is ~150 LOC, surgical, with current tests untouched.

## File-by-file map

**New files**
- `crates/lr-server/src/routes/pipeline.rs` (~500 LOC — mostly moved
  from chat.rs)
- `crates/lr-server/src/routes/dispatch.rs` (~600 LOC — moved
  handler-variant logic + free-tier fallback + MCP-via-LLM branch)
- `crates/lr-server/src/routes/finalize.rs` (~400 LOC — telemetry +
  streaming wrapper + JSON repair)
- `crates/lr-server/tests/shared_pipeline_contract.rs` (new contract
  fixture suite)

**Modified**
- `crates/lr-server/src/routes/chat.rs` — shrinks to ~1,500 LOC
  (adapter + provider-request conversion helper + helpers not yet
  movable). Handler function becomes ~35 LOC.
- `crates/lr-server/src/routes/responses.rs` — shrinks to ~200 LOC
  (adapter + request parser + session persist).
- `crates/lr-server/src/routes/completions.rs` — shrinks to ~200 LOC
  (adapter + prompt-to-message synth + legacy response serializer).
- `crates/lr-server/src/routes/mod.rs` — re-exports.
- `crates/lr-providers/src/lib.rs` — adds `ResponsesProvider` trait
  + blanket impl.
- `crates/lr-providers/src/openai.rs` — overrides `ResponsesProvider`
  when `is_chatgpt_backend()`.
- `crates/lr-mcp-via-llm/src/manager.rs` — adds `session_key` param to
  `handle_request` + `handle_streaming_request`, short-circuits
  `get_or_create_session`.
- `crates/lr-mcp-via-llm/src/session.rs` — adds `explicit_key` field.

**Reused helpers** (no logic change, may need `pub(crate)` flips):
- `apply_firewall_request_edits` — chat.rs:862
- `has_side_effects` — chat.rs:1154
- `estimate_token_count` — chat.rs:4448
- `check_model_firewall_permission` — chat.rs:704
- `check_free_tier_fallback` — chat.rs:2858 (moves into dispatch.rs)
- `spawn_routellm_classification` — chat.rs:429 (moves into pipeline.rs)
- `maybe_repair_json_content` — chat.rs:4410 (moves into finalize.rs)
- `monitor_helpers::*` — already shared.

## Phasing — 7 commits

Each commit leaves `master` green: clippy-clean, fmt-clean, test-green.
Golden-fixture contract test suite is written in commit 0 and runs
after every subsequent commit; failures mean regression.

**Commit 0: Golden contract test suite + baseline snapshot**
- Build `crates/lr-server/tests/shared_pipeline_contract.rs` with
  8 fixtures: basic, streaming, tool-call, guardrail-deny,
  secret-scan-hit, rate-limit, compression-on, routellm-auto,
  previous_response_id, mcp-via-llm.
- Each fixture POSTs to all three endpoints with equivalent inputs.
- Normalize event IDs + timestamps + uuid fields.
- Assert structural equality of emitted monitor events and access-log
  rows (modulo capability-disabled stages for legacy completions).
- Snapshot the current behavior as golden — explicitly tag the gaps
  (responses.rs missing metrics/tray; completions.rs missing
  compression/routellm) as EXPECTED in the snapshots.
- Add `state::AppState::for_tests(mock_providers)` constructor
  wiring real Arcs for non-LLM services and a `MockProvider` for the
  LLM path.

**Commit 1: Extract `finalize.rs`**
- Move `complete_llm_call`, `update_llm_call_response_body`, access
  log, metrics, tray graph, cost calc, `maybe_repair_json_content`
  out of `chat.rs` inline code into `routes/finalize.rs`.
- chat.rs still calls them — zero behavior change for chat.rs.
- responses.rs and completions.rs start calling them too; contract
  tests' "expected gaps" for responses/completions flip to green.
- **Gate: contract suite passes; responses/completions now report
  metrics + tray + completion event.**

**Commit 2: Extract `pipeline.rs` + `TurnContext`**
- Move `validate_request`, `apply_model_access_checks`,
  `check_rate_limits`, `run_guardrails_scan`, `run_secret_scan_check`,
  `run_prompt_compression`, `spawn_routellm_classification`,
  `convert_to_provider_request` into `routes/pipeline.rs`.
- Introduce `PipelineCaps` and `run_turn_pipeline`.
- Refactor `chat_completions` handler to call `run_turn_pipeline`.
- **Gate: contract suite passes with chat.rs-equivalent output.**

**Commit 3: Extract `dispatch.rs` + collapse handler variants**
- Move the 4 handler variants (`handle_streaming`,
  `handle_streaming_parallel`, `handle_non_streaming`,
  `handle_non_streaming_parallel`) + `handle_mcp_via_llm` +
  `check_free_tier_fallback` into `routes/dispatch.rs` as
  `dispatch_turn`.
- Collapse streaming/parallel matrix to 2 variants (choice internal).
- chat.rs handler now calls `dispatch_turn`.
- chat.rs down to ~1,500 LOC.
- **Gate: contract suite passes; chat.rs streaming + non-streaming
  behavior unchanged.**

**Commit 4: Port `responses.rs` to shared pipeline**
- Responses adapter becomes: parse → `build_chat_completion_request`
  → `run_turn_pipeline(PipelineCaps::responses())` → `dispatch_turn`
  → `finalize_*` → Responses-SSE or `ResponseObject` wrap → persist
  session.
- Deletes the duplicate `build_provider_request`-equivalents now
  that pipeline owns them.
- **Gate: contract suite passes; responses.rs no longer misses any
  telemetry.**

**Commit 5: Port `completions.rs` to shared pipeline with parity features**
- Synthesize `ChatMessage` from `PromptInput`, feed through
  `run_turn_pipeline(PipelineCaps::completions())` with all features
  enabled (per user answer: "gain the features").
- RouteLLM classification now works for completions text prompts too
  (text extraction is already content-agnostic).
- Legacy wire format still emitted at the adapter boundary.
- **Gate: contract suite passes; completions.rs gains compression,
  routellm, model firewall, free-tier fallback, `update_llm_call_*`
  events, and JSON repair (JSON repair no-ops unless legacy endpoint
  added response_format support).**

**Commit 6: `ResponsesProvider` trait + ChatGPT-backend native override**
- Add `ResponsesProvider` trait + blanket impl in `lr-providers`.
- Override in `OpenAIProvider` for `is_chatgpt_backend()` → native
  `POST /responses` via existing `openai_responses::http`.
- `responses.rs` adapter checks resolved provider; if native and no
  MCP-via-LLM, bypass `convert_to_provider_request` + dispatch and
  call the trait directly, then wrap the native stream/response into
  Responses SSE with minimal shape adjustment.
- Double-hop elimination for ChatGPT Plus → direct `/v1/responses`.
- **Gate: new fixture in contract suite — ChatGPT Plus + /responses
  + native path — asserts no intermediate `ChatCompletions` form is
  constructed (verify via tracing::subscriber in test). Token usage
  telemetry still emits correctly.**

**Commit 7: MCP-via-LLM explicit session key**
- Add `session_key: Option<String>` param to
  `McpViaLlmManager::handle_request` + `handle_streaming_request`.
- `McpViaLlmSession::explicit_key: Option<String>` field.
- `get_or_create_session` short-circuits when `session_key.is_some()`.
- `dispatch_turn` passes `ctx.mcp_session_key` through.
- `responses.rs` populates `ctx.mcp_session_key` from
  `req.previous_response_id` when set.
- **Gate: new fixture — /responses + previous_response_id + MCP-via-
  LLM — asserts session lookup used the explicit key (instrumented
  log assertion) rather than hash-matching.**

## Testing strategy

**Golden contract tests** (commit 0, updated each phase)
- Location: `crates/lr-server/tests/shared_pipeline_contract.rs`.
- Harness: real `AppState::for_tests` with a `MockProvider` that
  returns scripted responses + a `MemoryMonitorStore` that the test
  can read back.
- Each fixture is a 3-tuple of (chat payload, responses payload, legacy
  completions payload) constructed to be semantically equivalent. The
  test runs all three, collects the emitted monitor events + access
  log rows + metrics deltas, normalizes IDs, and asserts structural
  equality modulo the capability-disabled stages.

**Unit tests per module**
- `pipeline.rs`: each stage tested in isolation with a minimal
  `ChatCompletionRequest` and stubbed helpers (guardrails / secret-scan
  disabled for unit focus).
- `dispatch.rs`: one test each for (streaming, non-streaming) ×
  (router, mcp-via-llm) × (free-tier-success, free-tier-fallback) —
  8 cases, each with a `MockProvider` and `MockMcpManager`.
- `finalize.rs`: fixture-based — feed a known `CompletionResponse` +
  `TurnContext`, assert exact set of monitor events + access log rows
  + metrics increments.

**Provider-level**
- `ResponsesProvider` blanket impl: given a `MockModelProvider`
  returning a canned `CompletionResponse`, assert the default
  `create_response` produces the expected `ResponseObject`.
- `OpenAIProvider` override: uses wiremock to serve a canned
  `/responses` body and asserts the native path emits a
  `ResponseObject` without ever building a `CompletionResponse`.

**MCP-via-LLM**
- Existing tests at `crates/lr-mcp-via-llm/src/tests.rs` continue
  to pass unmodified (hash-matching fallback preserved).
- New test: explicit `session_key` routes to the right session even
  when the message history has drifted significantly (would fail hash
  matching) — proves the optimization holds.

## Verification

End-to-end after commit 7:

1. `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`
2. `rustup run stable cargo fmt --all -- --check`
3. `rustup run stable cargo test --workspace`
4. `cargo tauri dev`, then manual smoke:
   - Chat via `/v1/chat/completions` against OpenAI API key — works,
     monitor shows `update_llm_call_routing` + `complete_llm_call`.
   - Chat via `/v1/responses` against ChatGPT Plus OAuth — works,
     streams natively (tracing shows no `CompletionResponse` built
     between provider and wire serializer).
   - Legacy `/v1/completions` — works, monitor now shows routing
     metadata + compression tokens when applicable.
   - `/v1/responses` + `previous_response_id` continuation — session
     lookup uses explicit key (log-inspection confirms).
   - MCP-via-LLM client on `/v1/responses` — tool loop runs, session
     persists, agent continues across turns.
5. Contract fixture suite green; all formerly-gap fixtures now pass
   for every endpoint.
