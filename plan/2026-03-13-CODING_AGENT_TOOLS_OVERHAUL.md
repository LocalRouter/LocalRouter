# Coding Agent Tools Overhaul

## Context

The coding agent MCP tools (`coding_agent_start`, `coding_agent_say`, etc.) work but have several UX and architectural gaps compared to Claude Code's internal Agent tool. This plan addresses 5 improvements inspired by that comparison: honest session lifecycle, configurable tool naming, configurable approval routing, output indexing, and tool consolidation. The result is a cleaner 4-tool API surface (`AgentStart`, `AgentSay`, `AgentStatus`, `AgentList`) with smarter defaults.

**Underlying library constraint**: `command-group` v5.0.1 manages process groups. It supports spawning, signals (SIGINT/SIGTERM/SIGKILL via `UnixChildExt::signal()`), stdin/stdout, and zombie reaping. It does NOT support process resumption — once a process exits, context is lost.

---

## 1. Honest Session Lifecycle — Remove Silent Auto-Resume

**Problem**: `say()` on Done/Error/Interrupted sessions silently spawns a new process. The AI thinks context is preserved, but it's lost.

**Solution**: Return an error on `say()` for terminal sessions. Force explicit new session creation.

### Files to modify
- `crates/lr-coding-agents/src/manager.rs` — `say()` method (lines 207-243)
- `crates/lr-coding-agents/src/types.rs` — Add new error variant

### Changes
1. In `say()`, replace the `Done | Error | Interrupted` arm (lines 207-243) with:
   ```rust
   SessionStatus::Done | SessionStatus::Error | SessionStatus::Interrupted => {
       return Err(CodingAgentError::SessionEnded {
           status: session.status.clone(),
       });
   }
   ```
2. Add `SessionEnded { status: SessionStatus }` variant to `CodingAgentError` with message: `"Session has ended ({status}). Context cannot be preserved — start a new session with AgentStart."`
3. Update existing tests for `say()` that expect auto-resume behavior

---

## 2. Configurable Tool Prefix

**Problem**: Tool names are hardcoded as `coding_agent_*`. Users want customizable prefix, default `Agent`.

**Rule**: If prefix ends with non-alphanumeric char (e.g., `coding_agent_`), suffixes are lowercase (`start`). If prefix ends with alphanumeric (e.g., `Agent`), suffixes are capitalized (`Start`).

### Files to modify
- `crates/lr-config/src/types.rs` — Add `tool_prefix: String` to `CodingAgentsConfig`
- `crates/lr-coding-agents/src/mcp_tools.rs` — Replace hardcoded `TOOL_PREFIX` with dynamic generation
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — Dynamic tool name matching
- `crates/lr-mcp/src/gateway/merger.rs` — Dynamic instruction text
- Frontend types/mocks (after backend is working)

### Changes

**Config** (`crates/lr-config/src/types.rs`):
- Add to `CodingAgentsConfig`:
  ```rust
  #[serde(default = "default_tool_prefix")]
  pub tool_prefix: String,  // default: "Agent"
  ```

**Tool name generation** (`crates/lr-coding-agents/src/mcp_tools.rs`):
- Remove `const TOOL_PREFIX`
- Add helper functions that take prefix from config:
  ```rust
  /// Build a tool name from prefix + suffix, applying casing rules.
  /// Non-alphanumeric ending prefix: "coding_agent_" + "start" = "coding_agent_start"
  /// Alphanumeric ending prefix: "Agent" + "start" = "AgentStart"
  pub fn build_tool_name(prefix: &str, suffix: &str) -> String

  /// Check if a tool name matches any coding agent tool for this prefix.
  pub fn is_coding_agent_tool_with_prefix(tool_name: &str, prefix: &str) -> bool

  /// Extract action suffix from a tool name given the prefix.
  pub fn action_from_tool_with_prefix(tool_name: &str, prefix: &str) -> Option<&str>
  ```
- The 4 suffixes (after consolidation): `start`, `say`, `status`, `list`
- `build_tools_for_agent()` takes `prefix: &str` parameter
- `all_tool_names()` takes `prefix: &str` parameter
- `is_coding_agent_tool()` takes `prefix: &str` parameter

**Virtual server** (`crates/lr-mcp/src/gateway/virtual_coding_agents.rs`):
- Store prefix in `CodingAgentSessionState`
- `owns_tool()` uses dynamic prefix matching
- `check_permissions()` uses `build_tool_name(prefix, "start")` instead of hardcoded `"coding_agent_start"`
- `is_tool_indexable()` uses dynamic matching
- `build_instructions()` generates tool names dynamically from prefix

**Config propagation**:
- `CodingAgentManager` already holds `config: CodingAgentsConfig` — expose `config.tool_prefix`
- Pass prefix through `build_coding_agent_tools()` and all name-related functions

---

## 3. Configurable Approval Mode (Allow / Ask / Elicitation)

**Problem**: Approvals currently use a polling pattern (`status` → `respond`). Need configurable routing with 3 modes.

**Modes**:
- **Allow**: Auto-approve all tool/plan approvals, auto-respond to questions with empty answer. Show warning in UI.
- **Ask**: Show LocalRouter popup (new popup type). Dynamic fields based on question.
- **Elicitation** (default): Forward to client via MCP elicitation. Falls back to Ask if unsupported.

**Key outcome**: The `respond` tool is removed entirely. Down from 6 tools to 5 (and to 4 after combining say+interrupt in section 5).

### Files to modify

**Backend — Approval routing**:
- `crates/lr-config/src/types.rs` — New `CodingAgentApprovalMode` enum + config field
- `crates/lr-coding-agents/src/manager.rs` — Integrate approval mode into question handling
- `crates/lr-coding-agents/src/approval.rs` — Add auto-approve logic for Allow mode
- `crates/lr-coding-agents/src/mcp_tools.rs` — Remove `respond` tool, remove `handle_respond()`

**Backend — Popup (Ask mode)**:
- `src-tauri/src/ui/commands.rs` — Add `get_coding_agent_question_details` + `submit_coding_agent_question_response` commands
- `src-tauri/src/ui/commands.rs` — Add to `debug_trigger_firewall_popup` match for `"coding_agent_question"` type

**Backend — Elicitation mode**:
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — Pass elicitation manager reference
- `crates/lr-coding-agents/src/approval.rs` — Add elicitation forwarding with Ask fallback

**Frontend — Popup**:
- New file: `src/views/coding-agent-question.tsx` — Popup with dynamic fields (model on existing `src/views/elicitation-form.tsx`, 272 lines)
- `src/views/debug/index.tsx` — Add sample popup button for `"coding_agent_question"` type
- `src/types/tauri-commands.ts` — Add types for question details/response

**Frontend — Configuration UI**:
- `src/views/settings/coding-agents-tab.tsx` — Add ApprovalMode selector (Allow/Ask/Elicitation)
- Add warning banner when Allow is selected: "Autonomous mode: the agent will execute all actions without confirmation. This is powerful but dangerous."
- Add sample popup button next to the selector (FlaskConical icon, reuse `SamplePopupButton` component from `src/components/shared/SamplePopupButton.tsx`)

### Changes

**Config enum** (`crates/lr-config/src/types.rs`):
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentApprovalMode {
    Allow,       // Auto-approve everything
    Ask,         // Show LocalRouter popup
    Elicitation, // Forward via MCP elicitation (default)
}
```
Add to `CodingAgentsConfig`:
```rust
#[serde(default)] // default = Elicitation
pub approval_mode: CodingAgentApprovalMode,
```

**Approval routing logic** (`crates/lr-coding-agents/src/approval.rs`):
- In `GatewayApprovalRouter`, add method `auto_resolve_if_allowed()`:
  - If mode is Allow: immediately send `ApprovalResponse::Approved` on the oneshot, return true
  - Otherwise return false (caller proceeds with Ask/Elicitation)
- For Elicitation mode: create an `ElicitationRequest` from the `PendingQuestion` and forward to `ElicitationManager` (existing in `crates/lr-mcp/src/gateway/elicitation.rs`). On timeout or if client doesn't support elicitation, fall back to Ask mode popup.

**Popup** (`src/views/coding-agent-question.tsx`):
- Model on existing `src/views/elicitation-form.tsx` (272 lines)
- Tauri commands: `get_coding_agent_question_details`, `submit_coding_agent_question_response`
- Dynamic field rendering based on `QuestionItem.options`
- For `ToolApproval`: Show tool name + details, Allow/Deny buttons
- For `PlanApproval`: Show plan summary, Approve/Reject buttons
- For `Question`: Render options as radio buttons or text input

---

## 4. Output Indexing in Status Tool

**Problem**: Large status outputs consume AI context window. Should be indexed and summarized.

**Solution**: When `AgentStatus` output exceeds a threshold, index it via the session's `ContentStore` and return a summary with pointer to `IndexSearch`/`IndexRead`.

### Files to modify
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — `handle_tool_call()` post-processing, `is_tool_indexable()` change, add `content_store` to session state

### Changes

**Mark status as NOT generically indexable** (`virtual_coding_agents.rs`):
```rust
fn is_tool_indexable(&self, tool_name: &str) -> bool {
    // Status does its own indexing; list output is useful for generic indexing
    // Use dynamic name matching with prefix
    if is_status_tool(tool_name, prefix) { return false; }
    if is_list_tool(tool_name, prefix) { return true; }
    false
}
```

**Add ContentStore to session state** (`virtual_coding_agents.rs`):
```rust
pub struct CodingAgentSessionState {
    pub permission: lr_config::PermissionState,
    pub agent_type: Option<lr_config::CodingAgentType>,
    pub prefix: String,
    pub content_store: Option<Arc<ContentStore>>,  // NEW — shared with ContextMode
    pub search_tool_name: String,                   // NEW — e.g. "IndexSearch"
    pub response_threshold_bytes: usize,            // NEW — reuse from context mgmt config
}
```

**Wire up ContentStore**: In the gateway session creation pipeline (after all virtual server states are created), copy the `Arc<ContentStore>` reference and config from `ContextModeSessionState` into `CodingAgentSessionState`. This is a linking step in the gateway.

**Post-process status responses** (`virtual_coding_agents.rs::handle_tool_call()`):
After getting the status response from `handle_coding_agent_tool_call()`, check if `recent_output` text size exceeds `response_threshold_bytes`. If so:
1. Join `recent_output` lines into full text
2. Call `store.index(&source_label, &full_text)` — reuse the `compress_client_tool_response` pattern from `crates/lr-mcp/src/gateway/context_mode.rs:627-676`
3. Replace `recent_output` in the response with a summary: `"[Output compressed — {N} bytes indexed as {source}]\n\n{preview}\n\nUse {search_tool}(queries=[...], source=\"{source}\") to retrieve sections."`
4. Return the modified response

**Source label format**: `"coding_agent:{session_id}:{run_id}"` — allows targeted IndexSearch.

---

## 5. Combine Say + Interrupt into One Tool

**Problem**: `say` and `interrupt` are separate tools. They should be one tool with an `interrupt` flag.

**Underlying support**: `command-group` supports `signal(Signal::SIGINT)` on Unix via `UnixChildExt` trait. Currently uses `start_kill()` (SIGKILL) which is too harsh. We should use SIGINT for graceful interrupt.

### Behavior matrix

| interrupt | message | Behavior |
|-----------|---------|----------|
| false (default) | required | Send message (error if session is active with no stdin, error if session ended) |
| true | none | Graceful stop: send SIGINT, set status=Interrupted |
| true | present | Graceful interrupt: send SIGINT, wait for exit, return error (session ended, start new) |

Note: "interrupt with message" gracefully stops the current work. Since we can't resume context (command-group limitation), the response tells the user the session ended and they need to start a new one. This is honest — the AI knows context is lost.

### Files to modify
- `crates/lr-coding-agents/src/mcp_tools.rs` — Merge interrupt into say tool, remove interrupt tool
- `crates/lr-coding-agents/src/manager.rs` — Change `interrupt()` to use SIGINT instead of SIGKILL, add `say_with_interrupt()` method
- `crates/lr-coding-agents/src/types.rs` — Update `SayResponse` to cover interrupt case

### Changes

**Tool schema** (mcp_tools.rs):
```rust
// AgentSay — merged say + interrupt
McpTool {
    name: build_tool_name(prefix, "say"),
    description: "Send a message or interrupt a running session",
    input_schema: json!({
        "type": "object",
        "properties": {
            "sessionId": { "type": "string" },
            "message": { "type": "string", "description": "Message to send (required unless interrupting)" },
            "interrupt": { "type": "boolean", "description": "If true, gracefully interrupt the running session. Default: false" }
        },
        "required": ["sessionId"]
    }),
}
```

**Graceful interrupt** (manager.rs):
```rust
pub async fn interrupt(&self, session_id: &str, client_id: &str) -> Result<InterruptResponse, CodingAgentError> {
    // ... existing ownership check ...

    if let Some(ref process) = session.process {
        // Graceful: SIGINT first (Unix only), fallback to SIGKILL
        #[cfg(unix)]
        {
            use command_group::UnixChildExt;
            let _ = process.child.signal(nix::sys::signal::Signal::SIGINT);
        }
        #[cfg(not(unix))]
        {
            let _ = process.child.start_kill();
        }
        process.cancel.cancel();
    }

    session.status = SessionStatus::Interrupted;
    // ...
}
```

**Remove standalone interrupt tool**: Remove from `TOOL_SUFFIXES`, `build_tools_for_agent()`, `handle_coding_agent_tool_call()` match, `handle_interrupt()`.

**Handle merged say+interrupt** in `handle_say()`:
```rust
let interrupt = args["interrupt"].as_bool().unwrap_or(false);
if interrupt {
    // Call interrupt first
    manager.interrupt(session_id, client_id).await?;
    // If message provided, inform that session ended (don't auto-resume)
    if message.is_some() {
        return Err("Session interrupted. Context is lost — start a new session to continue.".into());
    }
    // No message: return interrupt response
    return Ok(Some(serde_json::to_value(InterruptResponse { ... }).unwrap()));
}
// Otherwise, normal say behavior...
```

---

## Final Tool Surface (4 tools)

With default prefix `Agent`:

| Tool | Description |
|------|-------------|
| `AgentStart` | Start a new coding agent session |
| `AgentSay` | Send message or interrupt a session |
| `AgentStatus` | Get status + output (with auto-indexing) |
| `AgentList` | List sessions |

---

## Files Modified (Summary)

| File | Changes |
|------|---------|
| `crates/lr-config/src/types.rs` | Add `tool_prefix`, `CodingAgentApprovalMode`, config fields |
| `crates/lr-coding-agents/src/mcp_tools.rs` | Dynamic prefix, remove respond+interrupt, merge say+interrupt |
| `crates/lr-coding-agents/src/manager.rs` | Remove auto-resume in say(), SIGINT in interrupt(), approval integration |
| `crates/lr-coding-agents/src/types.rs` | New error variant, updated response types |
| `crates/lr-coding-agents/src/approval.rs` | Auto-approve (Allow), elicitation forwarding |
| `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` | Dynamic names, output indexing, ContentStore wiring |
| `crates/lr-mcp/src/gateway/merger.rs` | Dynamic instruction text |
| `src-tauri/src/ui/commands.rs` | Coding agent question popup commands |
| `src/views/coding-agent-question.tsx` | New popup for Ask mode (model on elicitation-form.tsx) |
| `src/views/debug/index.tsx` | Sample popup button |
| `src/views/settings/coding-agents-tab.tsx` | Approval mode config + warning |
| `src/types/tauri-commands.ts` | New types |
| `website/src/components/demo/TauriMockSetup.ts` | Updated mocks |

---

## Verification

1. **Unit tests**: `cargo test -p lr-coding-agents` — all existing tests updated + new tests for:
   - `say()` returns error on terminal sessions
   - Tool name generation with various prefixes (`Agent` → `AgentStart`, `coding_agent_` → `coding_agent_start`, `MyBot_` → `MyBot_start`)
   - Auto-approve in Allow mode
   - SIGINT interrupt behavior

2. **Clippy + format**: `cargo clippy && cargo fmt`

3. **TypeScript types**: `npx tsc --noEmit`

4. **Manual testing**:
   - Start a session → verify tool names match configured prefix
   - Complete session → call say → verify error returned
   - Test Allow mode: questions auto-resolved
   - Test Ask mode: popup appears with correct fields
   - Test Elicitation mode: question forwarded to client
   - Verify large status output is indexed and summarized
   - Test interrupt (say with interrupt=true) → verify SIGINT sent
   - Test interrupt with message → verify error about session ended

5. **E2E**: `src-tauri/tests/coding_agents_e2e_test.rs` — update for new tool names and behaviors
