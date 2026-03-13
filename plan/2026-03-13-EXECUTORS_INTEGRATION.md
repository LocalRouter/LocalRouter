# Coding Agents: Executors Integration + Tool Refactoring

## Context

Our `lr-coding-agents` crate was supposed to use BloopAI/vibe-kanban's `executors` crate but was implemented from scratch without it. The result has critical gaps: no `kill_on_drop`, no graduated signal escalation, stdin is `Stdio::null()` (no control protocol), stderr discarded, no shutdown cleanup. Meanwhile, the vibe-kanban executors crate is battle-tested with proper process management, the Claude Code SDK control protocol (stdin/stdout JSON messages for approvals, interrupts, user messages, permission mode changes), and support for session resumption via `--resume`.

This plan integrates the executors crate as a git dependency and simultaneously refactors the tool surface based on 5 user requirements.

---

## Phase 0: Add executors as git dependency

### Dependency Strategy

**Decision: Accept all deps.** The executors crate has heavy transitive deps (`sqlx`, `codex-protocol`, `codex-core` from OpenAI git, `ts-rs`, `schemars`). We accept these as-is since the crate doesn't feature-gate them. Build times will increase but we get a battle-tested process management layer.

**Files:**
- `crates/lr-coding-agents/Cargo.toml` — add git deps:
  ```toml
  executors = { git = "https://github.com/BloopAI/vibe-kanban.git", package = "executors" }
  workspace_utils = { git = "https://github.com/BloopAI/vibe-kanban.git", package = "utils" }
  ```
- `Cargo.lock` — will update automatically
- Both `executors` and `workspace_utils` (aliased as `utils` in their workspace) needed — executors imports types from workspace_utils that appear in its public API (`ApprovalStatus`, `QuestionStatus`, `MsgStore`)

### What we keep vs replace

**Replace with executors:**
- `spawn_agent_process()` in manager.rs → Use `ClaudeCode::spawn()` / `spawn_follow_up()` from executors
- CLI argument building per agent → Executors handles this per-agent type
- Process lifecycle (kill_on_drop, signal handling) → Executors handles this
- Control protocol for Claude Code → Executors' `ProtocolPeer` handles stdin/stdout JSON messages

**Keep (our layer on top):**
- `CodingAgentManager` — session lifecycle, DashMap storage, broadcast notifications
- `CodingSession` — output buffering, status tracking, pending questions
- `GatewayApprovalRouter` — MCP approval routing (we plug into executors' `ExecutorApprovalService` trait)
- `virtual_coding_agents.rs` — VirtualMcpServer integration
- `mcp_tools.rs` — tool definitions and dispatch (heavily refactored per points 1-5)

### Integration Architecture

```
┌──────────────────────────────────────────────────┐
│  MCP Client (Claude Code, Cursor, etc.)          │
└─────────────────┬────────────────────────────────┘
                  │ MCP tool calls
┌─────────────────▼────────────────────────────────┐
│  Gateway → virtual_coding_agents.rs              │
│  (VirtualMcpServer, permissions, indexing)        │
└─────────────────┬────────────────────────────────┘
                  │
┌─────────────────▼────────────────────────────────┐
│  lr-coding-agents (our crate)                    │
│  ├── mcp_tools.rs   — tool defs + dispatch       │
│  ├── manager.rs     — session lifecycle          │
│  ├── types.rs       — session/response types     │
│  └── approval.rs    — approval routing           │
└─────────────────┬────────────────────────────────┘
                  │ spawn / spawn_follow_up
┌─────────────────▼────────────────────────────────┐
│  executors crate (vibe-kanban)                   │
│  ├── ClaudeCode executor (control protocol)      │
│  ├── Gemini, Codex, Amp, etc.                    │
│  └── command-group process management            │
└──────────────────────────────────────────────────┘
```

The key integration point: we implement `ExecutorApprovalService` (from executors) to bridge approvals from the control protocol into our existing popup/elicitation system.

---

## Phase 1: Session resumption — `say()` on Done/Error sessions

### How executors handles this

The `StandardCodingAgentExecutor` trait has:
```rust
async fn spawn_follow_up(
    &self,
    current_dir: &Path,
    prompt: &str,
    session_id: &str,           // Claude Code's internal session ID
    reset_to_message_id: Option<&str>,
    env: &ExecutionEnv,
) -> Result<SpawnedChild, ExecutorError>;
```

For Claude Code, this translates to `claude --resume <session_id> -p <prompt>`. The `--resume` flag tells Claude Code to load its conversation history from disk and continue the session. **Context is preserved** — this is a true resume, not a new session.

### Current problem

Our `say()` on Done/Error sessions calls `spawn_agent_process()` which starts a brand new process with `-p <message>` — all context is lost. The user sees a fresh session while thinking it's a continuation.

### Solution

1. **Capture Claude Code's session ID from output**: Claude Code emits a `{"type":"system","session_id":"..."}` message early in its stream-json output. Parse this in the output reader and store it on `CodingSession`.
2. **Use `spawn_follow_up()` for resumption**: When `say()` is called on a Done/Error/Interrupted session, use the executors' `spawn_follow_up()` with the captured session_id instead of `spawn()`.
3. **Preserve session state**: Keep the same `CodingSession` entry — just replace the process handle and reset status to Active.

**New field on `CodingSession`:**
```rust
/// The underlying agent's session ID (e.g., Claude Code's conversation ID).
/// Used for `--resume` on follow-up messages.
pub agent_session_id: Option<String>,
```

**Modified `say()` flow:**
```
say() on Active/AwaitingInput → send stdin message via ProtocolPeer (NEW — currently broken)
say() on Done/Error/Interrupted:
  if agent_session_id.is_some() → executor.spawn_follow_up(session_id, prompt)
  else → executor.spawn(prompt) (fallback — context lost, but works)
```

**Files:**
- `crates/lr-coding-agents/src/types.rs` — add `agent_session_id` field
- `crates/lr-coding-agents/src/manager.rs` — modify `say()`, parse session_id from output, use executors

---

## Phase 2: Configurable tool naming

### Requirements
- Tool prefix configurable (default: `Agent` instead of `coding_agent_`)
- If prefix ends with non-alphanumeric → suffixes stay lowercase: `Agent_start`, `Agent_status`
- If prefix ends with alphanumeric → suffixes capitalized: `AgentStart`, `AgentStatus`

### Implementation

**New config field** in `CodingAgentsConfig`:
```rust
/// Tool name prefix. Default: "Agent"
/// If ends with non-alphanumeric char, suffixes are lowercase (e.g., "agent_start")
/// If ends with alphanumeric char, suffixes are PascalCase (e.g., "AgentStart")
pub tool_prefix: String,  // default: "Agent"
```

**Tool name generation** in `mcp_tools.rs`:
```rust
fn tool_name(prefix: &str, suffix: &str) -> String {
    match prefix.chars().last() {
        Some(c) if c.is_alphanumeric() => {
            // PascalCase suffix: "Agent" + "Start" = "AgentStart"
            let mut capitalized = suffix.to_string();
            if let Some(first) = capitalized.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            format!("{}{}", prefix, capitalized)
        }
        _ => {
            // Lowercase suffix: "agent_" + "start" = "agent_start"
            format!("{}{}", prefix, suffix)
        }
    }
}
```

**Impact:** `is_coding_agent_tool()`, `action_from_tool()`, `all_tool_names()`, `build_tools_for_agent()` all need to accept the prefix as a parameter instead of using the hardcoded `TOOL_PREFIX` constant.

The `VirtualMcpServer` `owns_tool()` and `all_tool_names()` need access to the configured prefix — pass it through `CodingAgentSessionState`.

**Files:**
- `crates/lr-config/src/types.rs` — add `tool_prefix` to `CodingAgentsConfig`
- `crates/lr-coding-agents/src/mcp_tools.rs` — parameterize all tool name functions
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — pass prefix through state
- Config migration (next version)

---

## Phase 3: Configurable approvals — Allow/Ask/Elicitation

### Requirements
- Three modes: **Allow**, **Ask**, **Elicitation** (default)
- **Allow**: Auto-approve all tool requests + auto-answer questions with empty response. Show warning. Check if executors supports auto-approve natively (YES — `ClaudeCode` has `dangerously_skip_permissions` and `NoopExecutorApprovalService` that auto-approves).
- **Ask**: Show our firewall-style popup with dynamic fields based on the questions. New popup type alongside existing firewall/sampling/elicitation popups.
- **Elicitation**: Forward approval requests to the MCP client via the gateway's existing `ElicitationManager`. Falls back to Ask if client doesn't support elicitation.
- Removes the `respond` tool entirely (approvals handled via popup or elicitation instead of manual tool call).

### How executors handles approvals

The `ExecutorApprovalService` trait:
```rust
#[async_trait]
pub trait ExecutorApprovalService: Send + Sync {
    async fn create_tool_approval(&self, tool_name: &str) -> Result<String, Error>;
    async fn create_question_approval(&self, tool_name: &str, question_count: usize) -> Result<String, Error>;
    async fn wait_tool_approval(&self, approval_id: &str, cancel: CancellationToken) -> Result<ApprovalStatus, Error>;
    async fn wait_question_answer(&self, approval_id: &str, cancel: CancellationToken) -> Result<QuestionStatus, Error>;
}
```

The `ClaudeAgentClient` routes `CanUseTool` control requests through this service. When approvals is `None`, it auto-approves (`NoopExecutorApprovalService`).

### Integration

We implement `ExecutorApprovalService` with three strategies:

```rust
pub enum ApprovalMode {
    Allow,        // NoopExecutorApprovalService — auto-approve everything
    Ask,          // Route to our firewall popup system
    Elicitation,  // Route to ElicitationManager → MCP client
}
```

**Allow mode:**
- Use executors' built-in `NoopExecutorApprovalService` (returns `ApprovalStatus::Approved` for everything)
- Additionally set `dangerously_skip_permissions: true` on the Claude Code executor so the agent itself doesn't even ask
- Show warning in UI config

**Ask mode:**
- **Separate popup** (not reusing firewall popup — cleaner separation, custom UI for dynamic question fields)
- New `AskPopupApprovalService` implementing `ExecutorApprovalService`
- `create_tool_approval()` → creates approval session in a new `CodingAgentApprovalManager` (same pattern as `FirewallApprovalManager`)
- Broadcasts `"coding_agent/approvalRequired"` notification with tool_name, input preview, question fields
- New dedicated Tauri popup window + React component for coding agent approvals
- Dynamic fields: for `AskUserQuestion` → render the questions as form fields; for tool approvals → show tool name + input + allow/deny
- Add sample popup button to debug menu alongside existing popup types
- `wait_tool_approval()` → blocks on oneshot channel until popup responded

**Elicitation mode (default):**
- New `ElicitationApprovalService` implementing `ExecutorApprovalService`
- `create_tool_approval()` → wraps approval as an elicitation request via `ElicitationManager`
- Schema: `{ approve: boolean, reason?: string }` for tool approvals
- Schema: `{ answers: { [question]: string } }` for questions
- **Fallback**: If client responds with elicitation not supported (or timeout), automatically falls back to Ask mode popup
- `wait_tool_approval()` → `elicitation_manager.request_input()`, on failure → `ask_popup_service.wait_tool_approval()`

### Tool changes
- Remove `coding_agent_respond` tool entirely
- Remove `respond` from suffixes, remove handler
- Remove `PendingQuestion` from `StatusResponse` (no longer surfaced via polling)
- Keep `pending_question` internal for the approval service bridge but don't expose via MCP

### New config
```rust
pub enum CodingAgentApprovalMode {
    Allow,       // Auto-approve (dangerous)
    Ask,         // Our popup
    Elicitation, // MCP elicitation (default, falls back to Ask)
}
```

Add to `CodingAgentsConfig`:
```rust
pub approval_mode: CodingAgentApprovalMode, // default: Elicitation
```

**Files:**
- `crates/lr-coding-agents/src/approval.rs` — rewrite: implement `ExecutorApprovalService` for Ask/Elicitation
- `crates/lr-coding-agents/src/types.rs` — remove respond-related types, add `agent_session_id`
- `crates/lr-coding-agents/src/mcp_tools.rs` — remove respond tool, remove from suffixes
- `crates/lr-config/src/types.rs` — add `CodingAgentApprovalMode`, add to config
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — pass approval service references
- New: popup type in `crates/lr-mcp/src/gateway/` (or extend firewall) for Ask mode
- `src-tauri/src/ui/commands.rs` — new Tauri commands for coding agent approval popup + debug trigger
- Frontend: new popup component + debug menu button
- Config migration

---

## Phase 4: Output indexing for status command

**Decision: Rely on gateway auto-indexing.** The gateway's `maybe_compress_response()` (`gateway_tools.rs:717-806`) already indexes tool outputs exceeding the client's `response_threshold_bytes` into FTS5 and replaces them with a summary + IndexSearch hint. `coding_agent_status` is already marked `is_tool_indexable: true`. No custom indexing logic needed.

**Only change:** Adjust `deferrable_tools()` in `virtual_coding_agents.rs` to exclude `status` and `say` from catalog compression deferral — these are primary interaction tools that should always be visible. Only defer `start` and `list`.

**Files:**
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — adjust `deferrable_tools()` to only defer start/list

---

## Phase 5: Combine Say + Interrupt into one tool

### Requirements
- Merge `say` and `interrupt` into a single tool
- `say(sessionId, message, interrupt?)` where:
  - No message + no interrupt = error
  - Message only = send message (queue it)
  - Interrupt only (no message) = just stop
  - Message + interrupt = interrupt current work, then send new message
- Check what executors actually supports underneath

### How executors handles this

**Claude Code control protocol** (from `protocol.rs`):
```rust
// ProtocolPeer methods:
pub async fn send_user_message(&self, content: String)  // Sends via stdin
pub async fn interrupt(&self)                            // Sends {"type":"control_request","request":{"subtype":"interrupt"}}
```

The `ProtocolPeer.read_loop()` handles cancellation:
```rust
_ = cancel.cancelled(), if !interrupt_sent => {
    interrupt_sent = true;
    self.interrupt().await;  // Sends interrupt via control protocol
    // Continue loop to read Claude's response
}
```

So interrupts are graceful — they send a control message and wait for the agent to respond, rather than killing the process.

**Supported combinations:**
1. **Message only** (say): `protocol_peer.send_user_message(message)` — works on Active sessions. For Done/Error sessions, uses `spawn_follow_up()`.
2. **Interrupt only** (stop): `cancel.cancel()` → triggers `protocol_peer.interrupt()` → waits for graceful shutdown. If agent doesn't respond, process is killed via `start_kill()`.
3. **Interrupt + message**: Interrupt first via cancel token, wait for agent to stop, then `spawn_follow_up()` with the new message. This is the safest approach — can't inject a message mid-interruption.
4. **Queue message while active**: Via `send_user_message()` on the ProtocolPeer's stdin. The agent will process it after finishing current work.

### Tool design

Remove separate `interrupt` tool. Modify `say` tool:

```json
{
  "name": "AgentSay",
  "description": "Send a message to an agent session. Can optionally interrupt current work first.",
  "inputSchema": {
    "properties": {
      "sessionId": { "type": "string" },
      "message": { "type": "string", "description": "Message to send. If session is done/error, resumes with context." },
      "interrupt": { "type": "boolean", "description": "If true, interrupts current work before sending message. If true with no message, just stops the agent." }
    },
    "required": ["sessionId"]
  }
}
```

**Logic:**
```
say(sessionId, message=None, interrupt=false) → error("provide message or interrupt")
say(sessionId, message="...", interrupt=false):
  Active → send_user_message via stdin (queued)
  Done/Error/Interrupted → spawn_follow_up (resume)
say(sessionId, message=None, interrupt=true):
  Active → cancel + interrupt → status becomes Interrupted
  Done/Error/Interrupted → no-op (already stopped)
say(sessionId, message="...", interrupt=true):
  Active → cancel + interrupt → wait for stop → spawn_follow_up with message
  Done/Error/Interrupted → spawn_follow_up with message (already stopped)
```

### Updated tool list (4 tools instead of 6)

1. `AgentStart` — start a new session
2. `AgentSay` — send message / interrupt / resume (combined say+interrupt)
3. `AgentStatus` — get status + output (with wait support)
4. `AgentList` — list sessions

Removed: `AgentRespond` (Phase 3), `AgentInterrupt` (merged into Say)

**Files:**
- `crates/lr-coding-agents/src/mcp_tools.rs` — rewrite tool defs, remove respond+interrupt, add interrupt flag to say
- `crates/lr-coding-agents/src/manager.rs` — new `say_with_interrupt()` that handles all combinations
- `crates/lr-coding-agents/src/types.rs` — remove `InterruptResponse`, `RespondResponse`; update `SayResponse`

---

## Implementation Order

1. **Phase 0**: Add executors dependency, get it compiling
2. **Phase 1**: Wire up `spawn()` / `spawn_follow_up()` via executors, fix stdin for active sessions
3. **Phase 5**: Merge say+interrupt (tool surface change, simpler to do before approval refactor)
4. **Phase 2**: Configurable tool prefix
5. **Phase 3**: Approval modes (Allow/Ask/Elicitation) — most complex, depends on phases 0-1
6. **Phase 4**: Output indexing (mostly gateway-level, smallest change)

Followed by: config migration, frontend updates, demo mocks, tests

## Files to modify (summary)

### Backend — lr-coding-agents crate
- `Cargo.toml` — add executors + workspace_utils git deps
- `src/lib.rs` — re-exports
- `src/types.rs` — add `agent_session_id`, remove respond/interrupt response types, add approval mode
- `src/manager.rs` — major rewrite: use executors for spawn, implement combined say+interrupt, capture agent session IDs
- `src/mcp_tools.rs` — major rewrite: 4 tools instead of 6, configurable prefix, remove respond
- `src/approval.rs` — major rewrite: implement `ExecutorApprovalService` for Allow/Ask/Elicitation modes

### Backend — lr-config
- `src/types.rs` — add `tool_prefix`, `CodingAgentApprovalMode` to config, config migration

### Backend — lr-mcp gateway
- `src/gateway/virtual_coding_agents.rs` — pass prefix, pass approval service, update deferrable_tools
- `src/gateway/virtual_server.rs` — no change (trait already has `is_tool_indexable`)

### Backend — src-tauri
- `src/ui/commands.rs` — new Tauri commands for coding agent approval popup + debug trigger

### Frontend
- New popup component for coding agent approvals
- Debug menu: add coding agent approval sample button
- Config UI: approval mode selector with Allow warning

### Config migration
- Next version bump with new fields + defaults

## Verification

1. `cargo test` — all existing tests pass
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — TypeScript types valid
4. Manual test: start a Claude Code session via MCP, verify it uses the control protocol
5. Manual test: `say` on a Done session resumes with `--resume`
6. Manual test: interrupt + message on Active session
7. Manual test: approval popup appears in Ask mode
8. Manual test: elicitation forwarded in Elicitation mode
9. Manual test: tool prefix change updates tool names
10. Verify: large status output gets indexed via gateway compression
