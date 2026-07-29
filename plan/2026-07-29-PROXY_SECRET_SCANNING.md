# Secret Scanning in Proxy Mode

**Date**: 2026-07-29
**Status**: Implemented

## Problem

Sending `Is this my AWS key AKIAIOSFODNN7EXAMPLE` through a client in **proxy
mode** with secret scanning set to **Notify** produced no notification and no
monitor event. Three independent defects, each sufficient on its own:

1. **The proxy never scanned at all.** `run_secret_scan_check` was called only
   from the gateway pipeline (`lr-server/src/routes/pipeline.rs`). The proxy's
   firewall (`AppFirewall::evaluate` in `src-tauri/src/launcher/proxy.rs`)
   checked access control, allowed models, and rate limits — but never
   secrets. Proxy-mode traffic bypassed secret scanning entirely.

2. **Text extraction did not understand proxied request shapes.**
   `lr_guardrails::text_extractor::extract_request_text` only walked
   `messages[]` and `prompt`. Codex (OpenAI Responses) sends `input[]` +
   `instructions`, so **zero** text was extracted and nothing could ever
   match. Anthropic's top-level `system` field was likewise invisible.

3. **Nothing listened for the Notify event.** The backend emitted
   `secret-scan-notify`, but no frontend listener existed, so Notify was
   silent even on the gateway path.

The scanner engine itself was fine: it detects `AKIAIOSFODNN7EXAMPLE` via
`aws-access-key-id` at entropy 3.68 (threshold 3.0). Verified directly.

## Design

### One shared scan service, two callers

`run_secret_scan_check` was tied to `&ChatCompletionRequest`, which the proxy
never has. Extracted the logic into a format-agnostic entry point that takes
the raw request JSON:

```rust
pub enum SecretScanOutcome { Allow, Deny(String) }

pub async fn scan_request_for_secrets(
    state: &AppState, client_id: &str, model: &str, body: &Value,
) -> SecretScanOutcome
```

Both paths now share the scanner, the per-client action override, the
time-based bypass, the monitor events, and the approval popup:

- **Gateway**: `run_secret_scan_check` is a thin wrapper serializing the
  chat request and mapping `Deny` → HTTP 403.
- **Proxy**: `AppFirewall::evaluate` calls it with the provider-native body
  and maps `Deny` → `RequestAction::reject_json(403, …)`.

`AppFirewall` gained an `AppState` (already available at `ProxyService::new`,
which is constructed from `server_manager.get_state()`).

The scan runs **before** the model-approval ask so a leaked secret is still
caught when the model itself is already approved.

Because the proxy firewall hook is also what the WebSocket relay calls per
message, this covers Codex's WebSocket transport with no extra work.

### Format-agnostic text extraction

`extract_request_text` now additionally handles:

- OpenAI Responses: `input` (string or item array, `input_text`/`output_text`
  parts, `function_call` arguments, `function_call_output` output) and
  `instructions`
- Anthropic Messages: top-level `system` (string or content blocks)

System-prompt text from every format is labelled with a leading `system` so
the scanner's existing "skip system messages" rule applies uniformly —
system prompts are full of tool documentation that trips entropy rules.

### Notify actually notifies

`App.tsx` now listens for `secret-scan-notify` and raises a toast naming the
rules that fired, following the existing `guardrail-response-flagged` pattern.

### Fail closed on Ask

If the approval popup cannot be shown, the request is now **denied** rather
than returning a 500 that a caller might treat as transient. A request
carrying a detected secret must never go out because the UI failed.

## Behavior after the fix

| Action | Gateway | Proxy (HTTP + WebSocket) |
|---|---|---|
| Off | no scan | no scan |
| Notify | monitor event + toast, request proceeds | same |
| Ask | popup; deny → 403 | popup; deny → 403 to the client |

## Files

- `crates/lr-server/src/routes/pipeline.rs` — shared service + tests
- `crates/lr-guardrails/src/text_extractor.rs` — provider-native shapes
- `src-tauri/src/launcher/proxy.rs` — proxy firewall wiring
- `src-tauri/src/main.rs` — pass `AppState` to `ProxyService`
- `src/App.tsx` — Notify toast listener

## Verification

Unit tests lock the reported scenario end to end (extractor + engine) for the
Codex Responses body, the WebSocket `response.create` message, and the
Anthropic Messages body; plus system-prompt skipping and clean-traffic
negatives across formats. Full workspace clippy/fmt/test at CI parity.

Not verified against live Codex + a real ChatGPT account.
