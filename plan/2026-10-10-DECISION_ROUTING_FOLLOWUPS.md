# Decision routing follow-ups (pre-release review of d661ab97)

Follow-ups to "replace RouteLLM with configurable decision policies". Each item
was checked against the code before changing anything.

## Tasks

- [x] 1. Migrated RouteLLM policy silently degrades (visible setup state)
- [x] 2. `localrouter/auto` advertised but unusable (listing and requests agree)
- [x] 3. Tray toggle shown too broadly
- [x] 4. OpenAPI: document `localrouter.mode`
- [x] 5. `/v1/responses` forwards client `metadata` upstream
- [x] 6. Long prompts skip classification
- [x] 7. YAML-level 0.0.153 migration test
- [x] Plan review
- [x] Test coverage review
- [x] Bug hunt
- [x] Verification and commit (no push, per task instructions)

## Findings and decisions

### 1. Migrated policy reports "enabled" but cannot route — confirmed
A 0.0.153 strategy with RouteLLM on migrates to an enabled policy with no
decision model, so every request takes `thorough` (the priority list). The
migration result and the routing behaviour are unchanged.

`RoutingPolicy::setup_issue()` (lr-config) returns why a policy cannot send any
request away from its default route: `needs_decision_model` (semantic, no
decision model, no mode rules), `needs_mode_rule` (client mode, no rules) or
`needs_destination_models` (every option inherits the priority list). The
frontend mirrors it in `src/components/strategy/routing-policy-status.ts`.
Surfaces:
- RoutingPolicyEditor: warning banner while the draft is enabled and has an issue,
  naming the fallback route.
- Client info pill: yellow dot, "Needs a decision model, using default route".
- Decision Routing view: `get_feature_clients_status('decision_routing')` now
  sets `effective_value` to `routing` or the issue code; the row shows a warning.
- Tray: see item 3.
- Demo mock and demo tray mirror the same rules.

### 2. Auto listed but every request fails — confirmed
`has_chat_candidates()` counted any policy option's models. Every request ends
with the policy's default route and then the priority list, so when the default
route inherits an empty priority list, fallback requests (no decision model,
timeout, low probability, missing mode) have no model. Routing such requests to
a different route's models (for example the old weak pool) would override the
user's routing intent, so that was rejected. The invariant is now: auto is
listed and accepted exactly when every request has a candidate — the priority
list is non-empty, or the policy is enabled and its default route has its own
models. Policy-only strategies (the documented "plan → Astra, otherwise → Sol"
setup) keep working. The legacy empty-priority config again gets a clear 400 at
the pipeline, with a message that says what to add.

### 3. Tray toggle — confirmed, changed
`decision_routing_tray_entry()` shows the toggle only when switching it changes
routing (auto routing on, `setup_issue()` is `None`). An enabled policy that
cannot route shows a disabled status line ("Decision Routing: needs a decision
model") instead of a checkmark; a disabled one that cannot route is hidden.

### 4. OpenAPI — confirmed, documented
`ChatCompletionRequest.metadata` describes the reserved `localrouter.mode` key
with an example. `/v1/responses` was not in the spec at all; it now has a
`#[utoipa::path]`, and `CreateResponseRequest` (with `ResponseInput`,
`ReasoningRequest`) is a registered schema whose `metadata` documents the key.

### 5. Responses metadata upstream — confirmed
Only the OpenAI provider forwards `metadata`. OpenAI Chat Completions rejects it
unless `store: true` ("The 'metadata' parameter is only allowed when 'store' is
enabled"). Responses metadata does not depend on `store`, and Responses clients
usually omit `store`, so `/v1/responses` requests carrying metadata would fail.
The OpenAI provider now sends `metadata` only with `store: true`, which applies
to `/v1/chat/completions` and `/v1/responses` alike. Without `store`, metadata
had no upstream effect, so nothing is lost.

### 6. Long prompts — changed
A latest request over `max_context_chars` skipped classification, and coding
agents routinely send long requests. Now the request keeps its beginning and end
(the middle is replaced by an omission marker, total within the limit), history
gets no room, and `latest_request_omitted_chars` is in the decision context.
`routing_context` can no longer fail. The editor explains the limit.

### 7. YAML test — added
`storage::tests::routellm_0_0_153_yaml_loads_validates_and_saves_routing_policy`
loads a v28 (0.0.153) settings file with `routellm_settings` and a strategy
`routellm_config`, validates it, checks the migrated policy and its setup issue,
saves, asserts the file has `routing_policy:` and no `routellm`, and reloads.

## Tests added or updated
- lr-config: default-route candidate rule, migrated empty-priority policy has no
  candidates, `setup_issue` cases, 0.0.153 YAML round trip.
- lr-router: head/tail truncation; listed auto model always selects a candidate
  on fallback (and the migrated config selects none and is not listed).
- lr-providers: helper test and a wiremock test of the request body for
  store None/false/true.
- lr-server: OpenAPI documents `localrouter.mode` on chat and Responses and lists
  `/v1/responses`.
- src-tauri: tray entry states, feature status codes, auto model listing.
- TS unit: `routing-policy-status` mirrors the Rust rules.

## Final steps
- Plan review: every item has a code change or a recorded decision. Migration
  output and routing order are unchanged (existing migration tests pass).
- Test coverage review: each new function has direct tests (setup_issue, label
  via tray, default_route_models, has_chat_candidates, head_and_tail,
  stored_completion_metadata, decision_routing_status,
  decision_routing_tray_entry, TS helper).
- Bug hunt: the omission marker is at most 41 characters for any realistic
  length, below the 48 reserved, and `max_context_chars >= 128`, so truncated
  text stays within the limit. The truncation counts characters, not bytes, so
  multibyte text is never split. A semantic policy with mode rules but no
  decision model is routable (rules apply) and is not flagged. In client mode
  the decision model is ignored, matching `evaluate_routing_policy`. The disabled
  tray status item has its own id prefix and cannot be clicked.

## Verification
Rust 1.99 stable with PATH/RUSTC pinned, shared target dir:
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all -- --check`: clean
- `cargo test -p lr-config -p lr-router -p lr-providers -p lr-server`,
  `cargo test -p localrouter --lib`, `--test router_strategy_tests`: all pass
- `npx tsc --noEmit`: clean. Website `tsc` reports only `react-router-dom`
  missing, because this worktree has no `website/node_modules`.
- TS unit suite: 46 passed.
