# Plan: Add blocking wait to `coding_agent_status`

## Context

When an AI client uses the coding agent tools, after calling `coding_agent_start` or `coding_agent_say`, it must poll `coding_agent_status` in a loop to wait for the agent to finish or need input. This wastes tokens, adds latency, and clutters the conversation. A blocking wait mode would let the AI make a single call that returns only when the agent needs attention.

## Approach: Add `wait` parameter to existing `coding_agent_status`

Rather than adding a 7th tool, enhance `coding_agent_status` with an optional `wait: boolean` parameter. When `true`, the tool blocks until the session leaves the `Active` state (i.e., transitions to `done`, `awaiting_input`, `error`, or `interrupted`), then returns the same `StatusResponse`.

An optional `timeoutSeconds` parameter (default: 300s) prevents indefinite blocking. On timeout, it returns current status with `active` state (so the AI knows it timed out and can re-wait or take action).

**Why this over a new tool:**
- No new tool to register/discover
- Same response shape - AI already knows how to handle it
- Tool description update is sufficient for AI to learn the new mode
- Simpler implementation

**Key insight:** The broadcast channel (`change_tx` / `subscribe_changes()`) already exists in `CodingAgentManager` and `notify_changed()` fires on every state transition. This is currently unused by any consumer.

## Files to modify

### 1. `crates/lr-coding-agents/src/manager.rs`
Add a new public method:

```rust
pub async fn wait_for_non_active(
    &self,
    session_id: &str,
    client_id: &str,
    timeout: Duration,
    output_lines: Option<usize>,
) -> Result<StatusResponse, CodingAgentError>
```

Logic:
- Check current status immediately — if already non-active, return it
- Subscribe to `change_tx` via `subscribe_changes()`
- Loop: `tokio::select!` between broadcast recv and timeout
  - On broadcast: re-check session status, return if non-active
  - On timeout: return current status as-is (status will be `active`)
- After the wait loop exits, return `StatusResponse` (same as regular `status()`)

### 2. `crates/lr-coding-agents/src/mcp_tools.rs`
**Tool schema** (in `build_tools_for_agent`, the `coding_agent_status` entry):
- Add `wait` boolean property: `"description": "If true, blocks until the session needs attention (done, awaiting_input, error, interrupted) instead of returning immediately."`
- Add `timeoutSeconds` number property: `"description": "Max seconds to wait when wait=true (default: 300, max: 600)"`
- Update tool description to mention blocking mode

**Handler** (in `handle_status`):
- Parse `wait` and `timeoutSeconds` from args
- If `wait == true`: call `manager.wait_for_non_active()` instead of `manager.status()`
- Otherwise: existing behavior unchanged

### 3. `crates/lr-coding-agents/src/gateway/virtual_coding_agents.rs`
No changes needed — tool dispatch already goes through `handle_coding_agent_tool_call` which routes to `handle_status`.

### 4. MCP gateway instructions (if applicable)
Update the coding agents section in the system instructions to mention:
> Use `wait: true` with `coding_agent_status` to block until the agent needs attention, instead of polling in a loop.

## Edge cases

- **Session already non-active at call time**: Return immediately (no wait)
- **Timeout**: Return current status — AI sees `active` and knows it timed out
- **Session deleted during wait**: Return `SessionNotFound` error
- **Multiple broadcasts while active**: Each wakes the loop, re-checks, continues waiting if still active
- **Broadcast channel lag**: Use `tokio::sync::broadcast` with buffer of 16 — if receiver falls behind, `RecvError::Lagged` is handled by just re-checking status

## Verification

1. `cargo test -p lr-coding-agents` — unit tests pass
2. `cargo clippy -p lr-coding-agents` — no warnings
3. Manual test: start a coding agent session, call `coding_agent_status` with `wait: true`, verify it blocks until the agent finishes or asks a question
4. Verify non-wait mode still works identically (no regression)
