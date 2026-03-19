# Monitor Event Redesign: Combined Events + Session Grouping

## Context

The current monitor emits **31 separate event types** with request/response split into separate events. A single LLM chat completion generates 8-10+ events (LlmRequest, SecretScanRequest, SecretScanResponse, GuardrailRequest, GuardrailResponse, PromptCompression, LlmRequestTransformed, LlmResponse pending, LlmResponse updated, RoutingDecision, FirewallDecision). This is too noisy. The goal is to:

1. **Combine request/response pairs** into single events that update from pending → complete
2. **Add session grouping** so related events (LLM call + guardrail + secret scan + routing) appear as one collapsible group in the UI

## New Event Types (31 → 23)

### Combined events (10 types — emit pending, update to complete/error):

| New Type | Replaces | Category |
|----------|----------|----------|
| `LlmCall` | LlmRequest + LlmRequestTransformed + LlmResponse + LlmError | llm |
| `McpToolCall` | McpToolCall + McpToolResponse | mcp |
| `McpResourceRead` | McpResourceRead + McpResourceResponse | mcp |
| `McpPromptGet` | McpPromptGet + McpPromptResponse | mcp |
| `McpElicitation` | McpElicitationRequest + McpElicitationResponse | mcp |
| `McpSampling` | McpSamplingRequest + McpSamplingResponse | mcp |
| `GuardrailScan` | GuardrailRequest + GuardrailResponse | security |
| `GuardrailResponseScan` | GuardrailResponseCheckRequest + GuardrailResponseCheckResponse | security |
| `SecretScan` | SecretScanRequest + SecretScanResponse | security |
| `RouteLlmClassify` | RouteLlmRequest + RouteLlmResponse | routing |

### Standalone events (13 types — unchanged structure):
RoutingDecision, AuthError, AccessDenied, RateLimitEvent, ValidationError, McpServerEvent, OAuthEvent, InternalError, ModerationEvent, ConnectionError, PromptCompression, FirewallDecision, SseConnection

## Session Grouping

- Add `session_id: Option<String>` to `MonitorEvent` and `MonitorEventSummary`
- One `session_id` per incoming API request (e.g., one `/v1/chat/completions` call)
- All events within that request share the session_id — LlmCall, GuardrailScan, SecretScan, RoutingDecision, PromptCompression, FirewallDecision, and MCP tool calls triggered via MCP-via-LLM
- Standalone error events (AuthError, etc.) have `session_id: None`
- Frontend groups by session_id into collapsible rows with the LlmCall as the header

## Combined Event Data Model

Each combined `MonitorEventData` variant holds request fields (always populated) and response fields (`Option<T>`, filled on completion). Example for `LlmCall`:

```rust
LlmCall {
    // Request (populated at creation)
    endpoint: String, model: String, stream: bool,
    message_count: usize, has_tools: bool, tool_count: usize,
    request_body: serde_json::Value,
    // Transformation (filled via update)
    transformed_body: Option<serde_json::Value>,
    transformations_applied: Option<Vec<String>>,
    // Response (filled on completion)
    provider: Option<String>, status_code: Option<u16>,
    input_tokens: Option<u64>, output_tokens: Option<u64>, total_tokens: Option<u64>,
    cost_usd: Option<f64>, latency_ms: Option<u64>,
    finish_reason: Option<String>, content_preview: Option<String>, streamed: Option<bool>,
    // Error (filled only on error)
    error: Option<String>,
}
```

Same pattern for all other combined types: request fields required, response fields `Option<T>`.

## Emission Pattern

**Before (2 events):**
```
emit_guardrail_request()  → creates GuardrailRequest event
emit_guardrail_response() → creates GuardrailResponse event
```

**After (1 event, 2 calls):**
```
id = emit_guardrail_scan()    → creates GuardrailScan event (Pending)
complete_guardrail_scan(id)   → updates same event to Complete with response data
```

All helper pairs in `monitor_helpers.rs` merge into emit + complete function pairs. The `push()` method gains `session_id: Option<String>` parameter.

## MCP Gateway Changes

The MCP gateway callback currently fires-and-forgets (`emit_monitor_event` returns nothing). Need to:
1. Change callback signature to return event ID: `Fn(...) -> String`
2. Add update callback: `Fn(&str, updater) -> bool`
3. Wire both through `McpServerManager` and server state setup

## Frontend Changes

### Event list (`event-list.tsx`)
- Group events by `session_id` (memoized)
- Single-event sessions and `session_id: None` events render as standalone rows
- Multi-event sessions render as collapsible groups:
  - Header row = LlmCall (or first event) with expand chevron
  - Child rows indented underneath
  - Collapsed by default, shows child count badge

### Event detail (`event-detail.tsx`)
- Merged detail components (e.g., one `LlmCallDetail` replaces 4 separate components)
- Shows request section always, response section when complete, error section when errored
- Reduces from ~24 type-specific renderers to ~16

### Filters (`event-filters.tsx`)
- Updated type groups with new type names
- Add session_id filter support

### Hook (`useMonitorEvents.ts`)
- Session grouping logic (group by `session_id`, sort groups by earliest timestamp)
- Types updated

### TypeScript types (`tauri-commands.ts`)
- New `MonitorEventType` union (23 types)
- `session_id` field on `MonitorEventSummary` and `MonitorEvent`

### Demo mock (`TauriMockSetup.ts`)
- Update mock data for new event types

## Implementation Order

### Phase 1: Backend data model
1. `crates/lr-monitor/src/types.rs` — New enum variants, merged data shapes, session_id field, updated filter
2. `crates/lr-monitor/src/summary.rs` — Summary generation for new types (status-aware: pending shows request info, complete shows response info)
3. `crates/lr-monitor/src/store.rs` — Add session_id to push(), update filter matching, update tests

### Phase 2: Emission sites
4. `crates/lr-server/src/routes/monitor_helpers.rs` — Rewrite all helpers as emit+complete pairs with session_id parameter
5. `crates/lr-server/src/routes/chat.rs` — Thread session_id, use new helpers, update all ~40 emit calls
6. `crates/lr-mcp/src/gateway/gateway.rs` — Update callback signature to return event ID, add update callback
7. `crates/lr-mcp/src/gateway/gateway_tools.rs`, `gateway_resources.rs`, `gateway_prompts.rs` — Merge request/response emission
8. `crates/lr-mcp/src/manager.rs` — Update for new callback signatures
9. `crates/lr-mcp-via-llm/src/manager.rs` — Thread session_id from parent LLM call
10. `crates/lr-server/src/middleware/auth_layer.rs` — Update event type name (structure unchanged)
11. `crates/lr-server/src/routes/oauth.rs` — Update event type name
12. `crates/lr-server/src/state.rs` — Wire new callbacks

### Phase 3: Frontend
13. `src/types/tauri-commands.ts` — New TypeScript types
14. `src/views/monitor/event-list.tsx` — Session grouping UI
15. `src/views/monitor/event-detail.tsx` — Merged detail components
16. `src/views/monitor/event-filters.tsx` — Updated filter groups
17. `src/views/monitor/hooks/useMonitorEvents.ts` — Grouping logic
18. `website/src/components/demo/TauriMockSetup.ts` — Updated mocks

### Phase 4: Verify
19. `cargo test && cargo clippy && cargo fmt`
20. `npx tsc --noEmit`
21. Plan review, test coverage review, bug hunt

## Key Risks
- **chat.rs complexity** (~4300 lines, multiple streaming paths) — threading session_id through all paths requires care
- **MCP callback signature change** — ripples through McpServerManager, gateway, and server state wiring
- **All-at-once change** — enum variants shared across all crates, cannot be done incrementally
- **No persistence concern** — monitor store is in-memory ring buffer, cleared on restart
