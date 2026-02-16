# Firewall Feature - Implementation Plan

Per-client firewall for MCP tools and skills with three-tier permission: **Allow / Ask / Deny**.

---

## Data Model

### Config: `FirewallPolicy` + `FirewallRules`

**File:** `crates/lr-config/src/types.rs`

```rust
#[derive(Default)]
pub enum FirewallPolicy { #[default] Allow, Ask, Deny }

pub struct FirewallRules {
    pub default_policy: FirewallPolicy,           // Fallback (default: Allow)
    pub server_rules: HashMap<String, FirewallPolicy>,     // server_id -> policy
    pub tool_rules: HashMap<String, FirewallPolicy>,       // namespaced tool name -> policy
    pub skill_rules: HashMap<String, FirewallPolicy>,      // skill name -> policy
    pub skill_tool_rules: HashMap<String, FirewallPolicy>, // skill tool name -> policy
}
```

Resolution order (most specific wins):
1. `tool_rules["filesystem__write_file"]` → if present, use it
2. `server_rules[server_uuid]` → if present, use it
3. `default_policy` → fallback

Add `pub firewall: FirewallRules` to `Client` struct with `#[serde(default)]`.

### Session State: `GatewaySession`

**File:** `crates/lr-mcp/src/gateway/session.rs`

Add two fields:
- `pub firewall_rules: FirewallRules` — copied from client config at session creation
- `pub firewall_session_approvals: HashSet<String>` — tools approved for session lifetime ("Accept for session")

---

## Backend: FirewallManager

**New file:** `crates/lr-mcp/src/gateway/firewall.rs`

Follows the existing `ElicitationManager` pattern from `elicitation.rs`:

```rust
pub struct FirewallManager {
    pending: Arc<DashMap<String, FirewallApprovalSession>>,
    default_timeout_secs: u64,
    notification_broadcast: Option<Arc<broadcast::Sender<(String, JsonRpcNotification)>>>,
    app_handle: Option<tauri::AppHandle>,
}
```

**Approval session:**
```rust
pub struct FirewallApprovalSession {
    pub request_id: String,
    pub client_id: String,
    pub client_name: String,
    pub tool_name: String,
    pub server_name: String,
    pub arguments_preview: String,
    pub response_sender: Option<oneshot::Sender<FirewallApprovalResponse>>,
    pub created_at: Instant,
    pub timeout_seconds: u64,
}
```

**Response actions:** `Deny | AllowOnce | AllowSession`

**Flow:**
1. `request_approval()` → creates oneshot channel, stores session, emits Tauri event + SSE notification + triggers tray rebuild, awaits with timeout
2. `submit_response()` → looks up pending by request_id, sends on oneshot channel, triggers tray rebuild
3. On timeout → auto-deny, triggers tray rebuild

Add `firewall_manager: Arc<FirewallManager>` to `McpGateway`.

---

## Backend: Interception Point

**File:** `crates/lr-mcp/src/gateway/gateway_tools.rs` — `handle_tools_call()` (line 184)

Insert firewall check **after** tool name extraction, **before** routing:

```
1. Extract tool_name (existing, line 190-203)
2. Handle search tool (existing, line 206-208)
3. Check if skill tool (existing, line 211-215)
   → If skill: resolve policy via firewall_rules.resolve_skill_tool()
   → Apply firewall (see step 5)
   → Then proceed to handle_skill_tool_call()
4. Look up (server_id, original_name) from session (existing, line 220-231)
5. NEW — Firewall check:
   a. Resolve policy: firewall_rules.resolve_mcp_tool(tool_name, server_id)
   b. Allow → proceed
   c. Deny → return JsonRpcError with "Tool call denied by firewall"
   d. Ask → check session_approvals first
      - If tool in session_approvals → proceed
      - Else → firewall_manager.request_approval() → await response
        - AllowOnce → proceed
        - AllowSession → add to session_approvals, proceed
        - Deny/Timeout → return error
6. Route to server (existing, line 249-252)
```

For skill tools, the same check happens inside `handle_skill_tool_call()` or a wrapper before it in `handle_tools_call()`.

---

## Backend: Tauri Commands

**File:** `src-tauri/src/ui/commands_clients.rs`

| Command | Signature | Purpose |
|---------|-----------|---------|
| `get_client_firewall_rules` | `(client_id) -> FirewallRules` | Fetch rules for UI display |
| `set_client_firewall_rule` | `(client_id, rule_type, key, policy)` | Set one rule (rule_type: "server"/"tool"/"skill"/"skill_tool") |
| `set_client_default_firewall_policy` | `(client_id, policy)` | Set default policy |
| `submit_firewall_approval` | `(request_id, action)` | User responds to approval popup |
| `list_pending_firewall_approvals` | `() -> Vec<PendingApprovalInfo>` | Show pending requests in UI |

---

## Backend: Passing FirewallRules to Gateway

**File:** `crates/lr-server/src/routes/mcp.rs` — `mcp_gateway_handler()`

Currently passes `allowed_servers`, `deferred_loading`, `roots`, `skills_access` to `handle_request_with_skills()`. Add `client.firewall.clone()` as a new parameter. The gateway stores it in the session.

**File:** `crates/lr-mcp/src/gateway/gateway.rs` — `handle_request_with_skills()`

Accept `firewall_rules: FirewallRules` parameter, pass to session creation/update.

---

## Notification System: Tauri Popup Window + System Tray

### Tauri Popup Window

When a tool call hits "Ask" policy, the `FirewallManager` creates a **small always-on-top Tauri `WebviewWindow`** (~400x280px):

```rust
// In FirewallManager::request_approval()
let popup = WebviewWindowBuilder::new(
    &app_handle,
    format!("firewall-approval-{}", request_id),
    WebviewUrl::App("firewall-approval".into()),  // React route
)
.title("Tool Approval Required")
.inner_size(400.0, 280.0)
.resizable(false)
.always_on_top(true)
.center()
.build()?;
```

The popup shows a React component with:
- Client name + tool name (human-readable, de-namespaced)
- Server/skill name
- Arguments preview (truncated JSON)
- Three buttons: **Deny** | **Allow Once** | **Allow for Session**

Multiple concurrent approvals: each gets its own popup window (stacked/offset). The popup closes itself after user action or timeout.

**New frontend route:** `/firewall-approval` — a minimal React page that:
1. Reads the `request_id` from the window label
2. Fetches pending approval details via `invoke("get_firewall_approval_details", { requestId })`
3. Renders the approval UI
4. Calls `invoke("submit_firewall_approval", { requestId, action })` on user choice
5. Closes the window via `getCurrentWebviewWindow().close()`

### System Tray Integration

**Files:** `src-tauri/src/ui/tray.rs`, `tray_menu.rs`, `tray_graph.rs`

#### Tray Icon Overlay

Add a new `TrayOverlay` variant for pending firewall approvals:

```rust
pub enum TrayOverlay {
    None,
    Warning(Rgba<u8>),
    UpdateAvailable,
    FirewallPending,  // NEW — green question mark
}
```

Render as a green (`#22c55e`) question mark glyph in the top-left cutout area (same position as existing health/update overlays). Priority order: Health Warning > Firewall Pending > Update Available.

The overlay appears when `FirewallManager.pending` is non-empty and clears when all approvals are resolved.

#### Tray Menu — Pending Approvals Section

In `build_tray_menu()` (`tray_menu.rs`), add a new section at the top (below health issues, above clients):

```
❓ Approve: "write_file" for Client A     → click opens/focuses popup
❓ Approve: "run_script" for Client B     → click opens/focuses popup
─────────────────────────────────────
Clients (HEADER)
  ...
```

Each menu item uses ID `firewall_approve_<request_id>`. On click, the handler either:
- Focuses the existing popup window for that request_id, or
- Creates the popup if it was dismissed/closed

#### Tray Rebuild Triggers

Call `rebuild_tray_menu()` when:
- New firewall approval request is created
- Approval is resolved (user action or timeout)

Store pending approvals in shared state accessible to `build_tray_menu()`:

```rust
pub struct FirewallNotificationState {
    pending: Arc<DashMap<String, PendingApprovalInfo>>,
}
```

Managed via `app.manage(firewall_notification_state)` so `build_tray_menu()` can read it.

### Event Flow

```
Tool call hits "Ask" policy
  ↓
FirewallManager::request_approval()
  ├─ Store in pending DashMap
  ├─ Update FirewallNotificationState (for tray)
  ├─ Emit Tauri event "firewall-approval-request"
  ├─ Create popup WebviewWindow (always-on-top)
  ├─ Rebuild tray menu (adds approval item + green ? overlay)
  ├─ Send SSE notification to client (for elicitation-capable clients)
  └─ tokio::time::timeout(120s, oneshot_rx).await
        ↓
User clicks button in popup (or tray menu item → popup)
  ↓
invoke("submit_firewall_approval", { requestId, action })
  ↓
FirewallManager::submit_response()
  ├─ Send on oneshot channel
  ├─ Remove from pending + notification state
  ├─ Close popup window
  ├─ Rebuild tray menu
  └─ Emit "firewall-approval-resolved" event
```

---

## Frontend: Firewall Configuration UI

### Separate "Firewall" Tab on Client Detail

**New file:** `src/views/clients/tabs/firewall-tab.tsx`

A dedicated tab (alongside Connect, Models, MCP, Skills, Settings) showing all firewall rules for the client in one place.

### FirewallPolicySelector Component

**New file:** `src/components/firewall/FirewallPolicySelector.tsx`

Three-state segmented control: `[Allow] [Ask] [Deny]`
- Allow: green (`#10b981`)
- Ask: amber (`#f59e0b`)
- Deny: red (`#ef4444`)

### Firewall Tab Layout

```
Default Policy: [Allow] [Ask] [Deny]
"When no specific rule matches, tool calls will be: {policy}"

MCP Server Tools:
  ▸ filesystem                    [Allow] [Ask] [Deny]
    ├─ filesystem__read_file      [Allow] [Ask] [Deny]
    ├─ filesystem__write_file     [Allow] [Ask] [Deny]
    └─ filesystem__delete_file    [Allow] [Ask] [Deny]
  ▸ github                        [Allow] [Ask] [Deny]
    └─ (tools listed when expanded)

Skills:
  ▸ deploy                        [Allow] [Ask] [Deny]
    ├─ skill_deploy_get_info      [Allow] [Ask] [Deny]
    └─ skill_deploy_run_script    [Allow] [Ask] [Deny]
```

Each server/skill row is expandable (chevron) to show individual tools. Server/skill-level policy acts as bulk setter. Individual tool overrides take precedence and show as different from parent.

### New Tauri Commands for Tool/Skill Enumeration

- `list_mcp_server_tools(server_id) -> Vec<ToolInfo>` — tools for a specific server
- `list_skill_tools(skill_name) -> Vec<ToolInfo>` — tools for a specific skill

### Firewall Approval Popup Page

**New file:** `src/views/firewall-approval.tsx`

Minimal React page rendered in the popup `WebviewWindow`. Shows:
- Tool name (human-readable)
- Server/skill name
- Client name
- Arguments preview (truncated JSON, collapsible)
- Three buttons: **Deny** | **Allow Once** | **Allow for Session**
- Timeout countdown indicator

---

## Frontend: Connection Graph Changes

**File:** `src/components/connection-graph/utils/buildGraph.ts`

New edge styles for firewalled connections (client → MCP server / skill):

| Condition | Style |
|-----------|-------|
| All tools allowed (or no rules) | Current solid green/amber |
| Any tool set to Ask | Dashed amber stroke, `strokeDasharray: '4,4'` |
| All tools denied (server-level Deny) | Dashed red stroke, `strokeDasharray: '5,5'` |
| Mixed (some deny, some allow) | Dashed amber (indicates partial firewall) |

Include `firewall` in the `ClientInfo` returned by `list_clients` command so the graph builder can compute edge styles.

---

## Monitoring

**File:** `crates/lr-monitoring/src/mcp_logger.rs`

Add optional field to `McpAccessLogEntry`:
```rust
pub firewall_action: Option<String>, // "allowed", "denied", "asked:allowed", "asked:denied", "asked:timeout"
```

---

## Implementation Order

### Phase 1: Data Model + Deny/Allow (no async hold)
1. Add `FirewallPolicy`, `FirewallRules` to `crates/lr-config/src/types.rs`
2. Add `firewall` field to `Client` with `#[serde(default)]`
3. Add `firewall_rules` + `firewall_session_approvals` to `GatewaySession`
4. Create `crates/lr-mcp/src/gateway/firewall.rs` with resolution logic + `FirewallManager` skeleton
5. Wire `firewall_rules` through `mcp_gateway_handler` → `handle_request_with_skills` → session
6. Insert firewall check in `handle_tools_call()` for Allow/Deny only
7. Add Tauri commands for get/set rules
8. Unit tests for rule resolution

### Phase 2: Ask Flow (hold-and-wait + notifications)
1. Implement `FirewallManager.request_approval()` + `submit_response()` with oneshot + timeout (auto-deny)
2. Create popup `WebviewWindow` spawning logic in `FirewallManager`
3. Add `FirewallNotificationState` managed state for tray
4. Add tray overlay variant `FirewallPending` (green `?`) in `tray_graph.rs`
5. Add pending approvals section to `build_tray_menu()` in `tray_menu.rs`
6. Add tray menu click handler to focus/create popup in `tray.rs`
7. Add `submit_firewall_approval` + `get_firewall_approval_details` Tauri commands
8. Wire Ask path in `handle_tools_call()` with session approval caching
9. Add SSE notification path for MCP clients with elicitation support

### Phase 3: Frontend — Firewall Tab + Approval Popup
1. Create `FirewallPolicySelector` component
2. Create `firewall-tab.tsx` with server/skill tree + per-tool policy controls
3. Add "Firewall" tab to client detail view
4. Add `list_mcp_server_tools` / `list_skill_tools` commands for tool enumeration
5. Create `firewall-approval.tsx` popup page
6. Register popup route in Tauri window config
7. Include `firewall` in `list_clients` response for graph

### Phase 4: Connection Graph + Polish
1. Update `buildGraph.ts` edge styles for firewalled connections
2. Add monitoring log field
3. Handle edge cases: tool list changes while approval pending, config changes during active session
4. Test full end-to-end flow

---

## Verification

1. **Unit tests:** `FirewallRules::resolve_mcp_tool()` and `resolve_skill_tool()` with all precedence combinations
2. **Integration test:** Tool call with Deny policy returns JSON-RPC error
3. **Integration test:** Tool call with Ask policy holds, then resolves on approval
4. **Manual test:** Configure firewall in Firewall tab, observe graph edge changes
5. **Manual test:** Trigger Ask flow → popup appears, approve → tool executes
6. **Manual test:** "Allow for Session" persists for subsequent calls in same session
7. **Manual test:** Tray icon shows green `?`, menu shows pending approvals, click focuses popup
8. **Manual test:** Multiple concurrent approvals → multiple popups, tray shows all
9. **Manual test:** Timeout → auto-deny, popup closes, tray clears
10. **Run:** `cargo test && cargo clippy && cargo fmt`

---

## Critical Files

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | Add FirewallPolicy, FirewallRules, Client.firewall field |
| `crates/lr-mcp/src/gateway/firewall.rs` | **New** — FirewallManager, approval types, popup spawning |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Insert firewall check in handle_tools_call() |
| `crates/lr-mcp/src/gateway/gateway.rs` | Accept + pass firewall_rules to session |
| `crates/lr-mcp/src/gateway/session.rs` | Add firewall_rules + session_approvals fields |
| `crates/lr-server/src/routes/mcp.rs` | Pass client.firewall to gateway |
| `src-tauri/src/ui/commands_clients.rs` | Tauri commands for firewall CRUD + approval |
| `src-tauri/src/ui/tray.rs` | Tray menu click handler for approval items |
| `src-tauri/src/ui/tray_menu.rs` | Pending approvals section in menu |
| `src-tauri/src/ui/tray_graph.rs` | Green `?` overlay variant |
| `src/components/firewall/FirewallPolicySelector.tsx` | **New** — three-state segmented control |
| `src/views/clients/tabs/firewall-tab.tsx` | **New** — dedicated firewall config tab |
| `src/views/firewall-approval.tsx` | **New** — popup window approval page |
| `src/components/connection-graph/utils/buildGraph.ts` | Firewall-aware edge styles |
