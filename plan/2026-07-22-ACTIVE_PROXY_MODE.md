# Plan: Active HTTPS Proxy Mode (intercept · firewall · rewrite)

## Context

The passive inspection proxy (shipped) decrypts proxied LLM traffic, records it
(monitor + metrics + cost), and forwards it **unchanged**. **Active mode** is the
next step: the proxy may **pause, inspect, modify, redirect, or block** a request
(and response) before it reaches the provider — a firewall for LLM traffic.

Everything is built onto the seam already in place: the
[`ProxyInterceptor`](../crates/lr-proxy/src/interceptor.rs) trait, whose
`on_request`/`on_response` hooks already return `Forward` today and are typed to
return `Replace(..)` for a future active implementation. The transport already
**buffers the full request body** before forwarding, which is exactly what
rewriting needs.

The LLM axis already reserves the mode: `LlmMode::ProxyRewrite` exists (UI
disabled, backend rejected). This plan implements it.

### ⚠️ ToS note (read first)
Passive mode is a transparent pass-through — the client's own subscription token
reaches Anthropic unchanged, which is the compatible "corporate inspection proxy"
posture. **Active rewriting changes that calculus.** Silently rewriting the model,
injecting tools, or redirecting a subscription-authenticated request edges toward
"using the OAuth token in another tool/service," which the Feb-2026 Consumer Terms
prohibit. Recommendation: **gate destructive/rewriting active features to API-key
clients**, and for subscription clients limit active mode to *firewall block/allow*
(deny is always safe) and *observation-only* transforms. Surface this in the UI.

---

## What active mode does (feature set)

Ordered roughly by value / build order:

1. **Firewall interception (allow / ask / deny)** — the headline feature.
   Per-client, rule-based policy evaluated on each proxied request:
   - **Deny** → return a synthesized 403/error to the client, never contact the
     provider. (Always ToS-safe; works for subscription clients.)
   - **Ask** → pause the request, surface it in a UI approval popup (host, model,
     message preview, tools, token estimate), and wait for the user's decision
     (Allow once / Allow always / Deny / Deny always). Mirrors the existing MCP
     firewall approval flow.
   - **Allow** → forward (optionally with transforms below).
   Rules match on: host, model, path, message content (regex/keyword), tool
   names, request size/token estimate, time-of-day, client.

2. **Model access enforcement** — reuse the client's strategy `model_permissions`.
   - **Allow-list**: block requests whose `model` isn't permitted (deny with a
     clear message).
   - **Forced rewrite**: map the requested model → a configured target
     (e.g. force `claude-*-opus` → `claude-*-sonnet` for cost control). Rewrites
     the `model` field in the buffered request JSON before forwarding.

3. **Request optimization** — apply the existing pipeline transforms to proxied
   requests: JSON/prompt compression (`lr-compression`), catalog compression,
   guardrail pre-scan (`lr-guardrails`), secret scanning (`lr-secret-scanner`)
   with block-on-detect. Record `transformed_body` + `transformations_applied`
   on the monitor event (fields already exist on `LlmCall`).

4. **Response rewriting / guardrails** — scan/redact the response before it
   reaches the client (guardrail response scan, secret redaction). Requires
   buffering or streaming-transform of the response (see Architecture).

5. **MCP-via-LLM over proxy** — active mode can inject MCP tool definitions into
   the proxied request and execute tool calls server-side, the same way the
   native `McpViaLlm` path does. This is what unblocks the validation rule that
   currently forbids `mcp_mode == ViaLlm` with a proxy `llm_mode` (relax it to
   allow `ProxyRewrite + ViaLlm`).

6. **Reroute** — instead of forwarding to the origin, hand the (translated)
   request to LocalRouter's own router/provider system (e.g. serve an Anthropic
   request from a different provider). Largest scope; likely a later phase.

---

## Architecture

### The interceptor, extended
`ProxyInterceptor` already has the shape. Active mode is a new implementation,
`ActiveInterceptor`, selected per-connection by the client's `llm_mode`. Passive
stays the default.

```rust
// interceptor.rs — already present, fleshed out for active:
pub enum RequestAction {
    Forward,                          // passive
    Replace(Vec<u8>),                 // rewritten request body (+ maybe headers)
    Reject { status: u16, body: Vec<u8> },   // firewall deny / blocked
    // Reroute(RouterRequest)         // phase 6
}
pub enum ResponseAction { Forward, Replace(Vec<u8>) }
```

`on_request` becomes **async and awaited before forwarding** (it already is
async). For "Ask", it awaits a decision from an approval manager (below), so the
transport must `await ctx.interceptor.on_request(&ex)` and act on the result
*before* opening the upstream connection — a change from today, where the result
is ignored. The transport already has the full request bytes at that point.

### Transport changes (`transport.rs`)
Today `proxy_request` calls `on_request(&base)` and ignores the result. Active:
1. Build the exchange (have the buffered request bytes).
2. `match on_request(&ex).await`:
   - `Reject{status,body}` → return that response, never contact upstream.
   - `Replace(bytes)` → forward `bytes` instead of the original (recompute
     `Content-Length`; keep method/path/headers minus hop-by-hop).
   - `Forward` → unchanged.
3. Response: wrap the tapped body; for `ResponseAction::Replace`, buffer the
   response (bounded) and swap it. Streaming responses that must be rewritten
   need a transform-stream; for v1, response rewriting can buffer non-SSE
   responses and pass SSE through (rewriting SSE is a later refinement).

Because the request body is already fully buffered before forwarding, **request
rewriting needs no new streaming machinery** — only response rewriting does.

### The approval channel (firewall "Ask")
Reuse the pattern from the MCP gateway firewall
(`crates/lr-mcp/src/gateway/firewall.rs` + the sampling/elicitation
pending-approval managers): a `PendingApprovalManager` keyed by a request id, an
SSE/Tauri event to the UI carrying the request preview, and a oneshot the
transport awaits (with a timeout → default-deny). The proxy interceptor holds an
`Arc<dyn ApprovalGate>` (trait in lr-proxy; app wires it to the Tauri event
system), so lr-proxy stays decoupled.

Rule storage: a new `FirewallRules`-style structure on the client (the client
already has `firewall: FirewallRules` for MCP — extend or add an
`llm_firewall`). "Allow always / Deny always" writes a rule; the persistence
pattern is the existing firewall "always allow" flow (see the config-safety
memory: map to the correct config field, don't default to mcp_permissions).

### Rules engine
A small evaluator: `evaluate(request_meta, rules) -> Decision`. Ordered rules,
first match wins, default configurable (Allow / Ask / Deny). Reuses
`AnthropicRequestMeta` (model, has_tools, message_count) plus content scanning.
Structured matching only — **no string-matching the model name for capability
detection** (per the no-string-matching memory); match on catalog data /
explicit rule fields.

### Config + UI
- Client gets an `llm_proxy_policy` (default Ask/Allow) + rules list.
- `ProxyRewrite` becomes selectable in `ClientModeSelector` (drop the disabled
  flag) — but see the ToS gate: only enable rewriting features for API-key
  clients; for subscription clients, expose firewall-deny + observe only.
- A "Firewall" tab in client detail to author rules; a live approval toast/panel
  in the app (reuse the MCP firewall popup component).
- Monitor: proxied events already carry `transformed_body` /
  `transformations_applied` / a `firewall_action` concept (the MCP event has
  one) — show what active mode changed and why.

---

## Phasing

- **P1 — Firewall deny/ask/allow** (rules engine + approval gate + transport
  `Reject`/await). Highest value, ToS-safe (deny/allow). Response untouched.
- **P2 — Model enforcement** (allow-list deny + forced-model rewrite via
  `Replace`). API-key clients only.
- **P3 — Request optimization** (compression / guardrail pre-scan / secret block)
  reusing the native pipeline crates.
- **P4 — Response guardrails/redaction** (buffered non-SSE first, SSE later).
- **P5 — MCP-via-LLM over proxy** (relax validation; inject tools; execute).
- **P6 — Reroute to LocalRouter's router.**

Each phase is independently shippable behind the `ProxyRewrite` mode + per-feature
config.

## Representative files
- `crates/lr-proxy/src/interceptor.rs` — `RequestAction`/`ResponseAction`,
  `ApprovalGate` trait, rules types.
- `crates/lr-proxy/src/transport.rs` — act on `on_request`/`on_response` results.
- `crates/lr-proxy/src/active.rs` — new `ActiveInterceptor` (rules eval, approval,
  transforms), sibling to `passive.rs`.
- `crates/lr-config/src/types.rs` — client `llm_proxy_policy` + rules; relax the
  `ViaLlm + proxy` validation for `ProxyRewrite`.
- Reuse: `lr-mcp` firewall/approval pattern, `lr-compression`, `lr-guardrails`,
  `lr-secret-scanner`, strategy `model_permissions`.
- `src-tauri/src/launcher/proxy.rs` — wire `ActiveInterceptor` + approval gate to
  Tauri events.
- Frontend — enable `ProxyRewrite`, firewall rules UI, approval popup.

## Verification
- Unit: rules evaluation (allow/ask/deny ordering, defaults), model-rewrite of a
  buffered request body, reject-response synthesis.
- E2e (extend `mitm_e2e.rs`): a client whose rule denies a model → assert the
  local upstream is never hit and the client gets the synthesized error; a
  forced-model rule → assert the upstream receives the rewritten `model`.
- Manual: an "Ask" rule pops the approval UI; Allow/Deny routes correctly.

## Mandatory final steps
Save this plan (done), then on implementation: todo list first; plan review; test
coverage; bug hunt (approval timeouts, streaming rewrite races, Content-Length
recompute, rule-eval edge cases); commit per-phase.
