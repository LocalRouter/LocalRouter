# Client Sessions, Debug Mode, and Sticky Bottom Panels

## Context

Clients connect to LocalRouter for LLM requests (stateless HTTP) and MCP sessions (stateful SSE), but there's no unified view showing a client's live activity. Users need to inspect what a client is doing in real time -- which MCP sessions are open, what LLM requests are being made, what firewall decisions are happening -- and want a debugging mode to suppress popups and capture all traffic inline. Additionally, these debug/try-it-out views should persist as sticky bottom panels (Gmail compose-window style) so the user can navigate the app while monitoring.

---

## Phase 1: Backend -- ClientSessionManager

### New crate: `crates/lr-sessions`

A lightweight runtime-only session tracker (no disk persistence).

**Types:**

```rust
pub struct ClientSessionManager {
    debug_modes: DashMap<String, bool>,
    debug_logs: DashMap<String, Arc<Mutex<VecDeque<DebugLogEntry>>>>,
    connection_info: DashMap<String, ClientConnectionInfo>,
    llm_request_counts: DashMap<String, AtomicU64>,
    llm_last_activity: DashMap<String, Instant>,
    debug_buffer_size: usize, // default 1000
}

pub struct DebugLogEntry {
    pub timestamp: DateTime<Utc>,
    pub entry_type: DebugLogEntryType,
    pub client_id: String,
    pub summary: String,
    pub detail: Option<serde_json::Value>,
}

pub enum DebugLogEntryType {
    LlmRequest, LlmResponse,
    McpToolCall, McpToolResult,
    FirewallAutoApproved,   // Would have been popup, debug mode auto-allowed
    FirewallDecision,       // Normal allow/deny
    McpSessionCreated, McpSessionDestroyed,
    CodingAgentStarted, CodingAgentCompleted,
}

pub struct ClientConnectionInfo {
    pub remote_addr: Option<SocketAddr>,
    pub user_agent: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}
```

**Files to create:**
- `crates/lr-sessions/Cargo.toml`
- `crates/lr-sessions/src/lib.rs`

**Wire into AppState:**
- `crates/lr-server/src/state.rs` -- Add `pub client_session_manager: Arc<ClientSessionManager>` to `AppState`

### IP/Port Tracking

- `crates/lr-server/src/lib.rs` -- Change `axum::serve(listener, app)` to use `.into_make_service_with_connect_info::<SocketAddr>()`
- `crates/lr-server/src/middleware/auth_layer.rs` -- Extract `ConnectInfo<SocketAddr>` from request extensions, add `remote_addr: Option<SocketAddr>` to `AuthContext`
- `crates/lr-server/src/routes/chat.rs` -- After auth, call `state.client_session_manager.record_connection(client_id, remote_addr, user_agent)`

### Debug Mode: Firewall Bypass

When `debug_mode` is true for a client, firewall "Ask" decisions auto-approve (AllowOnce) and log a `FirewallAutoApproved` entry instead of showing a popup.

**Integration points** (insert check before `firewall_manager.request_approval()`):
- `crates/lr-mcp/src/gateway/gateway_tools.rs` -- In the `FirewallCheckResult::Ask` branch of tool call handling
- `crates/lr-server/src/routes/chat.rs` -- Model approval, guardrail approval, auto-router approval

**Approach:** `McpGateway` gets a reference to `ClientSessionManager`. Before `request_approval()`, check `session_manager.is_debug_mode(client_id)`. If true, log and return AllowOnce immediately.

### Debug Log Capture Points

- `crates/lr-server/src/routes/chat.rs` -- Log `LlmRequest` (model, message count) and `LlmResponse` (tokens, cost)
- `crates/lr-mcp/src/gateway/gateway_tools.rs` -- Log `McpToolCall` (tool name, args preview) and `McpToolResult`
- Each log entry emits a `"debug-log-entry"` Tauri event for real-time streaming

---

## Phase 2: Tauri Commands

### New file: `src-tauri/src/ui/commands_sessions.rs`

```rust
get_client_session_info(client_id: String) -> ClientSessionOverview
set_client_debug_mode(client_id: String, enabled: bool) -> ()
get_client_debug_mode(client_id: String) -> bool
get_client_debug_log(client_id: String, offset: usize, limit: usize) -> Vec<DebugLogEntryInfo>
clear_client_debug_log(client_id: String) -> ()
```

**`ClientSessionOverview`** aggregates:
- `client_session_manager.get_connection_info()` -- IP, port, user-agent
- `client_session_manager.get_llm_stats()` -- request count, last activity
- `mcp_gateway.get_sessions_for_client()` -- MCP sessions
- `coding_agent_manager.get_sessions_for_client()` -- Coding sessions
- `firewall_manager.list_pending()` filtered by client_id -- Pending approvals
- `debug_mode` flag

```rust
#[derive(Serialize)]
pub struct ClientSessionOverview {
    pub client_id: String,
    pub is_active: bool,
    pub debug_mode: bool,
    pub connection_info: Option<ConnectionInfoDto>,
    pub llm_request_count: u64,
    pub last_llm_activity_secs_ago: Option<u64>,
    pub mcp_sessions: Vec<McpSessionSummary>,
    pub coding_sessions: Vec<CodingSessionSummary>,
    pub context_sessions: Vec<ContextSessionSummary>,
    pub pending_approvals: Vec<PendingApprovalInfo>,
}
```

Register in `src-tauri/src/main.rs`.

---

## Phase 3: Frontend -- Sticky Bottom Panels

### Gmail-style panel system

Panels are fixed at the bottom-right of the main content area. Multiple can be open side-by-side. Each has a title bar (minimize/expand/close), resizable height via top drag handle.

**New files:**
- `src/components/panels/BottomPanelProvider.tsx` -- React context for panel state
- `src/components/panels/BottomPanelContainer.tsx` -- Renders all active panels
- `src/components/panels/BottomPanel.tsx` -- Individual panel chrome (title bar, resize, minimize, close)
- `src/components/panels/DebugSessionPanel.tsx` -- Debug log viewer content
- `src/components/panels/TryItOutPanel.tsx` -- Try-it-out content (future, can reuse existing LlmTab/McpTab)

**Panel state (React Context):**
```typescript
interface PanelState {
  id: string
  type: 'debug-session' | 'try-it-out-llm' | 'try-it-out-mcp'
  title: string
  clientId: string
  minimized: boolean
  height: number  // px
}

interface BottomPanelContextType {
  panels: Map<string, PanelState>
  openPanel: (panel: Omit<PanelState, 'id'>) => string
  closePanel: (id: string) => void
  toggleMinimize: (id: string) => void
  resizePanel: (id: string, height: number) => void
}
```

**Mount in AppShell** (`src/components/layout/app-shell.tsx`):
- Add `<BottomPanelContainer />` after `<main>`, within the flex column
- z-index: z-40 (below modals z-50, below toasts z-9999, above content)
- Wrap app with `<BottomPanelProvider>` in `App.tsx`

**Panel layout:**
- Panels sit in a flex row at `position: fixed; bottom: 0; right: 0`
- Each panel ~400px wide minimum, stacks left-to-right
- Minimized: just title bar (~36px)
- Expanded: default 300px height, draggable top edge to resize
- Smooth slide-in animation from bottom

### DebugSessionPanel content:
- Header: client name, active/inactive badge, "Clear" button
- Filter bar: All | LLM | MCP | Firewall
- Scrollable log list with entries showing: timestamp, type badge, summary
- Click entry to expand and show full detail JSON
- Auto-approved firewall entries highlighted with a distinctive badge
- Auto-scroll to bottom, with "scroll lock" toggle
- Subscribes to `"debug-log-entry"` Tauri event for live streaming

---

## Phase 4: Frontend -- Sessions Tab

### New file: `src/views/clients/tabs/sessions-tab.tsx`

**Add to client-detail.tsx** after the Connect tab, as its own group:
```tsx
<div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
  <TabsTrigger value="sessions">Sessions</TabsTrigger>
</div>
```

**Content:**
1. **Overview card**: Status badge (active/inactive), IP:port, user-agent, first/last seen, debug mode toggle (Switch), "Open Debug Panel" button
2. **LLM Activity section**: Request count, last activity time, recent models used
3. **MCP Sessions section**: Card list showing session ID, duration, initialized/failed servers, tool count, firewall approvals/denials
4. **Coding Agent Sessions section**: Card list with agent type, status, working directory, duration
5. **Context Sessions section**: If active, show context management state

**Data flow:** Polls `get_client_session_info` every 5s, also listens to `"client-session-changed"` event.

**Debug mode toggle:** Calls `set_client_debug_mode`. On enable, auto-opens a DebugSessionPanel via `BottomPanelContext`.

---

## Phase 5: TypeScript Types and Mock

### `src/types/tauri-commands.ts`
Add all new types: `ClientSessionOverview`, `ConnectionInfoDto`, `McpSessionSummary`, `CodingSessionSummary`, `ContextSessionSummary`, `DebugLogEntryInfo`, and all `*Params` interfaces.

### `website/src/components/demo/TauriMockSetup.ts`
Add mock handlers returning realistic demo data for all 5 new commands.

---

## Verification

1. **Backend**: `cargo test -p lr-sessions` -- Unit tests for ring buffer, debug mode toggle, connection info tracking
2. **Type check**: `npx tsc --noEmit`
3. **Lint**: `cargo clippy && cargo fmt`
4. **Manual testing**:
   - Enable debug mode for a client via Sessions tab
   - Verify sticky debug panel opens at bottom
   - Connect a client (e.g., via curl or MCP client)
   - Verify IP/port appears in session overview
   - Make LLM requests -- verify they appear in debug log in real-time
   - Make MCP tool calls with "Ask" permission -- verify they auto-approve and appear as "Auto-Approved" entries in debug log (no popup)
   - Navigate to other pages -- verify debug panel persists
   - Open a second "Try It Out" panel -- verify both display side-by-side
   - Minimize/expand/close panels
   - Disable debug mode -- verify popups resume for subsequent requests

---

## Key Files to Modify

| File | Change |
|------|--------|
| `crates/lr-sessions/` (new) | ClientSessionManager, DebugLogEntry, ConnectionInfo |
| `crates/lr-server/src/state.rs` | Add ClientSessionManager to AppState, extend AuthContext |
| `crates/lr-server/src/lib.rs` | `into_make_service_with_connect_info::<SocketAddr>()` |
| `crates/lr-server/src/middleware/auth_layer.rs` | Extract ConnectInfo, add remote_addr |
| `crates/lr-server/src/routes/chat.rs` | Debug log capture, debug-mode firewall bypass |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Debug-mode firewall bypass for MCP tools |
| `crates/lr-mcp/src/gateway/gateway.rs` | Thread ClientSessionManager into gateway |
| `src-tauri/src/ui/commands_sessions.rs` (new) | 5 Tauri commands |
| `src-tauri/src/main.rs` | Register new commands |
| `src/components/panels/` (new) | BottomPanelProvider, Container, Panel, DebugSessionPanel |
| `src/components/layout/app-shell.tsx` | Mount BottomPanelContainer |
| `src/App.tsx` | Wrap with BottomPanelProvider |
| `src/views/clients/client-detail.tsx` | Add Sessions tab |
| `src/views/clients/tabs/sessions-tab.tsx` (new) | Sessions tab content |
| `src/types/tauri-commands.ts` | New types |
| `website/src/components/demo/TauriMockSetup.ts` | Mock handlers |
