# Monitor Page - Passive MITM Traffic Inspector

## Context

LocalRouter needs a real-time traffic inspector to passively observe all requests/responses flowing through the gateway: LLM calls, MCP operations, guardrail checks, routing decisions, and more. Currently, the backend emits thin events (`llm-request` with just `"chat"`) and logs to access log files. The Monitor page provides a rich, structured, in-memory event stream viewable in the UI.

---

## Architecture: Centralized MonitorEventStore

Rather than emitting large payloads on every Tauri event, we use a **centralized in-memory store** on `AppState`. Routes/middleware push structured events into a bounded `VecDeque`. Lightweight Tauri events (`monitor-event-created`, `monitor-event-updated`) carry only the event ID. The frontend fetches details on-demand via Tauri commands.

This keeps the event bus light and handles streaming accumulation cleanly (create event as Pending, update to Complete when stream finishes).

---

## Event Types

All event types use request/response pairs where applicable. Linked via shared `request_id`.

### LLM Events

| Event Type | Emission Point | Key Data |
|-----------|----------------|----------|
| `LlmRequest` | Route handler entry | endpoint, model, client, messages, tools, stream, params |
| `LlmResponse` | Non-streaming completion / stream finish | provider, model, status, tokens, cost, latency, content preview, finish_reason |
| `LlmError` | Provider error paths | error message, status code, provider, model |

### MCP Events

| Event Type | Emission Point | Key Data |
|-----------|----------------|----------|
| `McpToolCall` | `handle_tools_call` after firewall | tool name, server, arguments, firewall decision |
| `McpToolResponse` | After server response | tool name, response preview, latency, error |
| `McpResourceRead` | `handle_resources_read` | URI, server |
| `McpResourceResponse` | After response | content preview, latency |
| `McpPromptGet` | `handle_prompts_get` | prompt name, server, arguments |
| `McpPromptResponse` | After response | content preview, latency |
| `McpElicitationRequest` | Elicitation handler receives request | schema, message, server, request_id |
| `McpElicitationResponse` | After user responds / timeout | response content, action (submitted/cancelled/timeout), latency |
| `McpSamplingRequest` | Sampling handler receives request | messages, model hint, server, max_tokens, timeout |
| `McpSamplingResponse` | After LLM response / approval | model used, content, tokens, action (approved/rejected), latency |

### Security Events (request/response pairs)

| Event Type | Emission Point | Key Data |
|-----------|----------------|----------|
| `GuardrailRequest` | `run_guardrails_scan` called | direction (request/response), text preview, models used, client |
| `GuardrailResponse` | `run_guardrails_scan` returns | result (pass/flagged), flagged categories, confidence scores, action taken, latency |
| `GuardrailResponseCheckRequest` | Response-side guardrail starts | stream context, text preview |
| `GuardrailResponseCheckResponse` | Response-side guardrail returns | flagged categories, stream_aborted, latency |
| `SecretScanRequest` | `run_secret_scan_check` called | client, text preview, scanner rules count |
| `SecretScanResponse` | `run_secret_scan_check` returns | findings count, findings details, action taken (notify/ask/block), latency |

### Routing Events

| Event Type | Emission Point | Key Data |
|-----------|----------------|----------|
| `RouteLlmRequest` | Before RouteLLM classification call | original model, threshold, client |
| `RouteLlmResponse` | After RouteLLM classification | win_rate, threshold, selected tier (strong/weak), routed model |
| `RoutingDecision` | After final routing determined (auto-router, model firewall, or direct) | routing_type, original model, final model, candidate models (if auto-router), firewall action |

### Other Events

| Event Type | Emission Point | Key Data |
|-----------|----------------|----------|
| `PromptCompression` | After compression completes | original/compressed tokens, reduction %, duration, method |
| `FirewallDecision` | Any firewall approval flow | type (tool/model/auto-router), client, action (allow/deny + duration), item name |
| `SseConnection` | SSE open/close | client, session, action (opened/closed) |

---

## Backend Implementation

### New crate: `crates/lr-monitor/`

Follows existing crate pattern (`lr-monitoring`, `lr-guardrails`).

**Files:**
- `crates/lr-monitor/Cargo.toml`
- `crates/lr-monitor/src/lib.rs` — public API
- `crates/lr-monitor/src/types.rs` — `MonitorEvent`, `MonitorEventType`, `MonitorEventData` enum, `EventStatus`
- `crates/lr-monitor/src/store.rs` — `MonitorEventStore`: `VecDeque`-based ring buffer with `parking_lot::RwLock`, push/update/list/get/clear, Tauri event emission on push/update
- `crates/lr-monitor/src/summary.rs` — `MonitorEventSummary` generation (one-line summary from event data for list view)

**MonitorEventStore key design:**
- Default max capacity: 1000 events
- FIFO eviction when at capacity
- `push()` — insert event, emit `monitor-event-created` with `{id, event_type, summary}`
- `update()` — modify existing event (streaming accumulation), emit `monitor-event-updated` with `{id}`
- `list()` — paginated summaries with optional filter (event type, client, status, text search)
- `get()` — full event detail by ID
- AtomicU64 sequence counter for stable ordering

**MonitorEvent stores the full request/response JSON** (as `serde_json::Value`) for detail rendering, with a truncation limit on large content fields (e.g., response content capped at 10KB). This enables the detailed view the user wants (seeing messages, tools, parameters).

### Wire into AppState

**File: `crates/lr-server/src/state.rs`**
- Add `pub monitor_store: Arc<lr_monitor::MonitorEventStore>` to `AppState`
- Initialize in builder, set `app_handle` on store when `set_app_handle()` is called

### Instrument route handlers

**`crates/lr-server/src/routes/chat.rs`:**
- After line 61 (`emit_event("llm-request", "chat")`): Push `LlmRequest` with full request JSON, model, client_id, stream flag, message count, tools count
- Before `run_secret_scan_check` (~line 287): Push `SecretScanRequest`; after return: Push `SecretScanResponse` with findings/action
- Before `run_guardrails_scan` spawns (~line 290): Push `GuardrailRequest`; after join handle returns: Push `GuardrailResponse` with result/categories/action
- Before RouteLLM classification: Push `RouteLlmRequest` with original model and threshold; after classification returns: Push `RouteLlmResponse` with win_rate and tier
- After final routing determined (auto-router, model firewall, or direct — wherever final model is set): Push `RoutingDecision` with routing_type, original model, final model
- After prompt compression: Push `PromptCompression`
- Non-streaming response (`handle_non_streaming`): Push `LlmResponse` with response body
- Streaming response: Push `LlmResponse` with `status: Pending` at stream start. In the post-stream accounting `tokio::spawn` block (~line 2200-2320), call `monitor_store.update(id)` to fill in final tokens/cost/latency/content_preview and set `status: Complete`
- Error paths: Push `LlmError`

**`crates/lr-server/src/routes/completions.rs`**, **`embeddings.rs`**, **`audio.rs`**, **`images.rs`**, **`moderations.rs`:**
- Same pattern: `LlmRequest` at entry, `LlmResponse`/`LlmError` at completion

### MCP gateway instrumentation

The MCP gateway (`crates/lr-mcp/`) doesn't have access to `AppState`. Use the existing callback pattern (`on_context_saved` at `gateway.rs:66`):

**File: `crates/lr-mcp/src/gateway/gateway.rs`:**
- Add `pub(crate) on_monitor_event: parking_lot::RwLock<Option<Arc<dyn Fn(lr_monitor::MonitorEvent) + Send + Sync>>>`
- Add setter: `pub fn set_on_monitor_event<F: Fn(lr_monitor::MonitorEvent) + Send + Sync + 'static>(&self, callback: F)`
- Wire up in AppState construction to call `monitor_store.push()`

**`crates/lr-mcp/src/gateway/gateway_tools.rs`:**
- In `handle_tools_call` after firewall check (~line 228): Emit `McpToolCall`
- After server response (~line 269): Emit `McpToolResponse`

**`crates/lr-mcp/src/gateway/gateway_resources.rs`:**
- Emit `McpResourceRead` / `McpResourceResponse`

**`crates/lr-mcp/src/gateway/gateway_prompts.rs`:**
- Emit `McpPromptGet` / `McpPromptResponse`

**Elicitation/Sampling handlers in `gateway.rs`:**
- When elicitation request received: Emit `McpElicitationRequest`; after user responds/timeout: Emit `McpElicitationResponse`
- When sampling request received: Emit `McpSamplingRequest`; after LLM response/approval: Emit `McpSamplingResponse`

### Tauri commands

**New file: `src-tauri/src/ui/commands_monitor.rs`**

Commands:
- `get_monitor_events(offset, limit, filter)` → `MonitorEventListResponse`
- `get_monitor_event_detail(event_id)` → `Option<MonitorEvent>`
- `clear_monitor_events()` → `()`
- `get_monitor_stats()` → `MonitorStats`
- `set_monitor_max_capacity(capacity)` → `()`

Register in `src-tauri/src/main.rs` invoke_handler and `src-tauri/src/ui/mod.rs`.

---

## Frontend Implementation

### Navigation registration

**`src/components/layout/sidebar.tsx`:**
- Add `'monitor'` to `View` type union (line 49)
- Add nav item **above** Clients (line 720), using `Activity` icon from lucide-react:
  ```tsx
  {renderNavItem({ id: 'monitor', icon: Activity, label: 'Monitor' })}
  ```

**`src/App.tsx`:**
- Import `MonitorView`, add case in `renderView()` switch

### Component structure

```
src/views/monitor/
  index.tsx                 — Main layout (resizable panels + Try-it-out)
  event-list.tsx            — Scrollable event list table
  event-detail.tsx          — Detail panel dispatcher
  event-filters.tsx         — Filter bar (type, client, status, search)
  try-it-out-panel.tsx      — Collapsible side panel
  hooks/
    useMonitorEvents.ts     — Event subscription + state management
  detail-renderers/
    llm-request-detail.tsx  — Messages, tools, parameters
    llm-response-detail.tsx — Tokens, cost, content preview
    guardrail-detail.tsx    — Request: text preview, models; Response: categories, findings, confidence, action
    mcp-tool-detail.tsx     — Tool call args + response
    mcp-resource-detail.tsx
    mcp-prompt-detail.tsx
    mcp-elicitation-detail.tsx — Request: schema/message; Response: user action/content
    mcp-sampling-detail.tsx    — Request: messages/model hint; Response: LLM result/action
    secret-scan-detail.tsx     — Request: client/rules; Response: findings/action
    routing-detail.tsx         — RouteLLM req/resp + final routing decision
    compression-detail.tsx     — Token savings, method
    firewall-detail.tsx        — Decision type, action, item
    generic-detail.tsx         — Fallback JSON view
```

### Layout

```
+--------------------------------------------+------------------+
| [Filter bar] [Clear] [Stats: 142/1000]     | Try It Out  [>]  |
+--------------------------------------------+------------------+
| Event List (ResizablePanel - top)           | [Client: ...]    |
|  Timestamp | Type | Client | Summary | ms  | [LLM | MCP]     |
|  12:34:01  | LLM Req | Cursor | gpt-4o    | ...              |
+--------------------------------------------+                  |
| Detail Panel (ResizablePanel - bottom)      |                  |
|  LLM Chat Request                           |                  |
|  Model: openai/gpt-4o | Stream: true        |                  |
|  Messages: 12 | Tools: 5                    |                  |
|  [Messages] [Parameters] [Raw JSON]         |                  |
+--------------------------------------------+------------------+
```

Uses `ResizablePanelGroup` from `@/components/ui/resizable` (already exists, wraps `react-resizable-panels`).

The Try-it-out panel:
- Collapsed by default (width 0 with overflow hidden)
- Toggle button in header
- When expanded: ~480px wide, shows client selector dropdown + LLM/MCP tabs
- Reuses `LlmTab` and `McpTab` from `src/views/try-it-out/` with `initialMode="client"`, `hideModeSwitcher`
- Does NOT auto-connect — user must select a client first

### Event list columns
- **Time** — relative ("2s ago") with absolute tooltip
- **Type** — icon + color-coded badge (blue=LLM, green=MCP, orange=Security, purple=Routing)
- **Client** — name with ServiceIcon
- **Summary** — model name for LLM, tool name for MCP, "N findings" for security
- **Status** — badge (Pending spinner, Complete check, Error x)
- **Duration** — ms

Newest first. Click to select → populates detail panel.

### useMonitorEvents hook

```typescript
function useMonitorEvents(filter?: MonitorEventFilter) {
  // Initial load via get_monitor_events command
  // Listen to 'monitor-event-created' → prepend to list
  // Listen to 'monitor-event-updated' → refresh that event in list + detail if selected
  // Returns: events, selectedEvent, selectEvent, clearEvents, stats
}
```

### Detail renderers

Each renderer receives the full `MonitorEvent.data` and renders type-specific UI:

- **LLM Request**: Model, endpoint, stream flag, message count by role, token estimate, tools list, collapsible full messages view (rendered as chat bubbles), collapsible parameters panel, collapsible raw JSON
- **LLM Response**: Provider, model, status, token usage bar, cost, latency, finish reason, content preview (expandable), streaming badge
- **MCP Tool Call/Response**: Tool name, server, arguments (formatted JSON), firewall decision badge; response: content, latency, error
- **MCP Elicitation Request/Response**: Request: schema fields, message; Response: user action (submitted/cancelled/timeout), submitted content, latency
- **MCP Sampling Request/Response**: Request: messages, model hints, max_tokens; Response: model used, content, tokens, action (approved/rejected), latency
- **Guardrail Request/Response**: Request: direction, text preview, models used; Response: result badge, category list with confidence bars, flagged categories, action taken, latency
- **Secret Scan Request/Response**: Request: client, rules count; Response: findings with details, action taken (notify/ask/block), latency
- **RouteLLM Request/Response**: Request: original model, threshold; Response: win rate visualization, tier selected, routed model
- **Routing Decision**: Routing type (auto-router/model-firewall/direct), original vs final model, candidate models list
- **Prompt Compression**: Original/compressed tokens, reduction % bar, duration, method
- **Firewall Decision**: Type badge (tool/model/auto-router), client, action + duration, item name
- **SSE Connection**: Client, session, action (opened/closed), timestamp
- **Generic**: Formatted JSON fallback for any unrecognized type

### TypeScript types

**`src/types/tauri-commands.ts`:**
- Add `MonitorEventType`, `EventStatus`, `MonitorEventSummary`, `MonitorEvent`, `MonitorEventListResponse`, `MonitorStats`, `MonitorEventFilter`
- Add command params: `GetMonitorEventsParams`, `GetMonitorEventDetailParams`, `SetMonitorMaxCapacityParams`

### Demo mocks

**`website/src/components/demo/TauriMockSetup.ts`:**
- Add handlers for `get_monitor_events`, `get_monitor_event_detail`, `clear_monitor_events`, `get_monitor_stats`

**`website/src/components/demo/mockData.ts`:**
- Add sample monitor events array

---

## Implementation Phases

### Phase 1: Backend core (`crates/lr-monitor/`)
1. Create crate with types, store, summary generation
2. Add `MonitorEventStore` to `AppState`
3. Create `commands_monitor.rs` with Tauri commands
4. Register commands in `main.rs`

### Phase 2: Instrument LLM routes
5. Instrument `chat.rs` (most complex: streaming, guardrails, RouteLLM, secret scan, compression)
6. Instrument `completions.rs`, `embeddings.rs`, `audio.rs`, `images.rs`, `moderations.rs`

### Phase 3: Instrument MCP gateway
7. Add `on_monitor_event` callback to `McpGateway` (follows `on_context_saved` pattern)
8. Instrument `gateway_tools.rs`, `gateway_resources.rs`, `gateway_prompts.rs`
9. Instrument elicitation and sampling handlers

### Phase 4: Frontend core
10. Add `'monitor'` to View type, sidebar nav, App.tsx routing
11. Create `MonitorView` layout with resizable panels
12. Create `useMonitorEvents` hook
13. Create event list + filter bar
14. Create detail panel dispatcher

### Phase 5: Detail renderers
15. LLM request/response renderers
16. MCP tool/resource/prompt renderers
17. Guardrail, secret scan, routing renderers
18. Generic fallback

### Phase 6: Try-it-out panel
19. Collapsible side panel with client selector
20. Integrate `LlmTab` and `McpTab`

### Phase 7: Testing + mocks
21. Backend unit tests for store (ring buffer, eviction, update, filter)
22. Integration tests (route emission)
23. Frontend TypeScript types
24. Demo mocks for website

---

## Key Files to Modify

| File | Change |
|------|--------|
| `crates/lr-monitor/` (new) | Entire crate |
| `crates/lr-server/src/state.rs` | Add `monitor_store` to AppState |
| `crates/lr-server/src/routes/chat.rs` | Instrument all phases |
| `crates/lr-server/src/routes/completions.rs` | LlmRequest/LlmResponse |
| `crates/lr-server/src/routes/embeddings.rs` | LlmRequest/LlmResponse |
| `crates/lr-server/src/routes/audio.rs` | LlmRequest/LlmResponse |
| `crates/lr-server/src/routes/images.rs` | LlmRequest/LlmResponse |
| `crates/lr-server/src/routes/moderations.rs` | LlmRequest/LlmResponse |
| `crates/lr-mcp/src/gateway/gateway.rs` | Add monitor callback |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | McpToolCall/McpToolResponse |
| `crates/lr-mcp/src/gateway/gateway_resources.rs` | McpResourceRead/Response |
| `crates/lr-mcp/src/gateway/gateway_prompts.rs` | McpPromptGet/Response |
| `src-tauri/src/ui/commands_monitor.rs` (new) | Tauri commands |
| `src-tauri/src/ui/mod.rs` | Register module |
| `src-tauri/src/main.rs` | Register commands |
| `src/components/layout/sidebar.tsx` | View type + nav item |
| `src/App.tsx` | Route case |
| `src/views/monitor/` (new) | All frontend components |
| `src/types/tauri-commands.ts` | TypeScript types |
| `website/src/components/demo/TauriMockSetup.ts` | Mock handlers |
| `website/src/components/demo/mockData.ts` | Sample events |

## Reusable Components/Patterns
- `ResizablePanelGroup/ResizablePanel/ResizableHandle` from `@/components/ui/resizable`
- `useTauriListener` from `src/hooks/useTauriListener.ts`
- `LlmTab` from `src/views/try-it-out/llm-tab/index.tsx`
- `McpTab` from `src/views/try-it-out/mcp-tab/index.tsx`
- `on_context_saved` callback pattern from `crates/lr-mcp/src/gateway/gateway.rs:66`
- `renderNavItem` pattern from sidebar.tsx for adding nav entry

## Verification

1. **Backend**: `cargo test -p lr-monitor` — ring buffer, eviction, update, filter
2. **Integration**: `cargo test` — ensure route instrumentation doesn't break existing tests
3. **Compile check**: `cargo clippy && cargo build`
4. **Frontend types**: `npx tsc --noEmit`
5. **Manual test**: Start dev server, make requests via Try-it-out, verify events appear in Monitor list with correct details
6. **Streaming test**: Send a streaming chat request, verify single event goes Pending → Complete
7. **MCP test**: Call an MCP tool, verify McpToolCall + McpToolResponse appear
8. **Capacity test**: Set low capacity (10), send many requests, verify old events evicted
