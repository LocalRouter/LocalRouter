# MCP-via-LLM History-Based Session Matching & Reconstruction

## Context

Two related bugs in MCP-via-LLM mode:

**Bug 1 — Session collision**: `get_or_create_session` uses naive "first non-expired session per client_id". Two apps sharing the same client always get the same session, causing: history contamination, shared firewall approvals, broken memory transcripts, pending execution collisions.

**Bug 2 — Lost MCP context across turns**: The orchestrator injects MCP tool calls/results into the message history during the agentic loop, but on the next turn the client only sends the "visible" messages (no injected MCP interactions). The orchestrator takes the client's messages as-is without reconstructing the full history. The LLM loses all MCP tool context from previous turns, breaking multi-turn agentic conversations.

**Example of Bug 2:**
```
Turn 1 stored full_messages: [sys, sys_instr, user1, asst_tools, tool_result, asst_final]
Turn 2 client sends:         [sys, user1, asst_final, user2]
Turn 2 LLM sees:             [sys, user1, asst_final, user2]  ← missing MCP context!
Turn 2 LLM should see:       [sys, sys_instr, user1, asst_tools, tool_result, asst_final, user2]
```

The fix uses **normalized hash matching** for both session selection AND history reconstruction: find the right session based on message similarity, then splice hidden MCP messages back into the request.

## Implementation Plan

### Step 1: Add `unicode-normalization` dependency

- **`Cargo.toml`** (workspace): Add `unicode-normalization = "0.1"` to `[workspace.dependencies]`
- **`crates/lr-mcp-via-llm/Cargo.toml`**: Add `unicode-normalization = { workspace = true }`

### Step 2: Add normalization, hashing, and scoring functions to `session.rs`

**File**: `crates/lr-mcp-via-llm/src/session.rs`

```rust
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use unicode_normalization::UnicodeNormalization;

/// Normalize text for fuzzy-resilient hash comparison.
/// Trim whitespace, collapse interior whitespace, Unicode NFC normalization.
fn normalize_for_hash(text: &str) -> String {
    text.trim()
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compute a normalized hash for a single message.
pub fn compute_message_hash(role: &str, content: &ChatMessageContent) -> u64 {
    let text = content.as_text();
    let normalized = normalize_for_hash(&text);
    let mut hasher = DefaultHasher::new();
    role.hash(&mut hasher);
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// Compute normalized hashes for a slice of ChatMessages.
pub fn compute_message_hashes(messages: &[ChatMessage]) -> Vec<u64> {
    messages.iter().map(|m| compute_message_hash(&m.role, &m.content)).collect()
}

/// Score how well stored hashes match incoming hashes (0.0..=1.0).
/// Handles prefix match (continuation) and suffix-anchored match (client dropped old messages).
pub fn score_session_match(stored: &[u64], incoming: &[u64]) -> f64 { ... }
```

### Step 3: Add `client_message_hashes` to `McpViaLlmSession`

**File**: `crates/lr-mcp-via-llm/src/session.rs`

```rust
pub struct McpViaLlmSession {
    // ... existing fields ...
    /// Hashes of client-visible messages from the last request (before any injection).
    /// Used for session matching on subsequent requests.
    pub client_message_hashes: Vec<u64>,
}
```

Initialize to empty vec in `new()`.

### Step 4: Add history reconstruction function to `session.rs`

**File**: `crates/lr-mcp-via-llm/src/session.rs`

This is the core new functionality: given a session's stored full history (including hidden MCP interactions) and the client's incoming messages, reconstruct the complete history for the next LLM call.

```rust
/// Reconstruct the full conversation history by splicing hidden MCP messages
/// back into the incoming request messages.
///
/// The session's `full_messages` contains the complete history from previous turns
/// including MCP tool calls/results that the client never saw. The client's incoming
/// messages contain only the "visible" subset plus new messages for this turn.
///
/// Algorithm:
/// 1. Find the anchor: hash the last message in full_messages (the final assistant
///    response returned to the client) and find it in incoming
/// 2. Replace everything up to the anchor with full_messages (which includes hidden
///    MCP interactions)
/// 3. Strip previously-injected server instructions (they'll be re-injected fresh)
/// 4. Use the client's current system message (in case it changed)
/// 5. Append new messages from incoming after the anchor
pub fn reconstruct_history(
    full_messages: &[ChatMessage],
    incoming: &[ChatMessage],
    gateway_instructions: Option<&str>,
) -> Vec<ChatMessage> {
    if full_messages.is_empty() {
        return incoming.to_vec();
    }

    // Hash the last message in full_messages (the anchor)
    let anchor = full_messages.last().unwrap();
    let anchor_hash = compute_message_hash(&anchor.role, &anchor.content);

    // Find anchor in incoming (search from end for robustness)
    let anchor_pos = incoming.iter().rposition(|m|
        compute_message_hash(&m.role, &m.content) == anchor_hash
    );

    let Some(pos) = anchor_pos else {
        // Can't find anchor — can't reconstruct, use incoming as-is
        return incoming.to_vec();
    };

    let mut result = Vec::with_capacity(full_messages.len() + incoming.len() - pos);

    // Take full session history (includes hidden MCP messages)
    result.extend_from_slice(full_messages);

    // Append new messages from incoming (after the anchor)
    if pos + 1 < incoming.len() {
        result.extend_from_slice(&incoming[pos + 1..]);
    }

    // Strip previously-injected server instructions (will be re-injected fresh)
    if let Some(instructions) = gateway_instructions {
        result.retain(|m| {
            !(m.role == "system" && m.content.as_text() == instructions)
        });
    }

    // Use client's current system message (handles changes between turns)
    if let Some(client_sys) = incoming.first().filter(|m| m.role == "system") {
        if let Some(first_sys) = result.iter_mut().find(|m| m.role == "system") {
            *first_sys = client_sys.clone();
        }
    }

    result
}
```

**Key design decisions:**
- Anchor on the last message in `full_messages` (the final assistant response the client received)
- Strip injected server instructions by matching against `gateway_instructions` content
- Replace the system message with the client's current one (handles system prompt changes)
- Fall back to raw incoming if anchor not found (new conversation or too much divergence)

### Step 5: Refactor `get_or_create_session` in `manager.rs`

**File**: `crates/lr-mcp-via-llm/src/manager.rs`

Change signature to accept optional messages for matching:
```rust
pub(crate) fn get_or_create_session(
    &self,
    client_id: &str,
    incoming_messages: Option<&[ChatMessage]>,
) -> Arc<RwLock<McpViaLlmSession>>
```

Algorithm:
1. Clean expired sessions for this client_id
2. If `None` (preview) or empty → return first available or create new
3. Compute `incoming_hashes`, score each session, return best match >= 0.5
4. No match → create new session

### Step 6: Update `handle_request` and `handle_streaming_request`

**File**: `crates/lr-mcp-via-llm/src/manager.rs`

In both methods, after session matching but before calling the orchestrator:

```rust
// 1. Match session
let session = self.get_or_create_session(&client.id, Some(&request.messages));

// 2. Store client hashes for future matching (BEFORE reconstruction)
let incoming_hashes = compute_message_hashes(&request.messages);
session.write().client_message_hashes = incoming_hashes;

// 3. Check for pending mixed execution (has its own reconstruction)
if let Some((pending, results)) = self.take_pending_if_matching(&client.id, &request) {
    // ... resume flow (unchanged, has own history reconstruction) ...
}

// 4. Reconstruct history: inject hidden MCP messages from previous turns
{
    let s = session.read();
    if !s.history.full_messages.is_empty() {
        let reconstructed = reconstruct_history(
            &s.history.full_messages,
            &request.messages,
            s.gateway_instructions.as_deref(),
        );
        drop(s);
        request.messages = reconstructed;
    }
}

// 5. Call orchestrator (inject_server_instructions happens inside)
```

Also update:
- `list_tools_for_preview` (line 212): pass `None`
- `get_gateway_session_key` (line 129): leave as-is

### Step 7: Update `pending_executions` to be session-aware

Add `gateway_session_key: String` to `PendingMixedExecution` in `session.rs`.

Update construction in:
- `orchestrator.rs` (~line 590): Read `session.read().gateway_session_key.clone()`
- `orchestrator_stream.rs` (~line 679): Same

### Step 8: Update `take_pending_if_matching` and resume flow

When `take_pending_if_matching` finds a match, extract `pending.gateway_session_key` to look up the correct session via `find_session_by_gateway_key` instead of relying on the session from `get_or_create_session`.

### Step 9: Add `find_session_by_gateway_key` helper

**File**: `crates/lr-mcp-via-llm/src/manager.rs`

```rust
fn find_session_by_gateway_key(
    &self, client_id: &str, gateway_key: &str,
) -> Option<Arc<RwLock<McpViaLlmSession>>> {
    self.sessions_by_client.get(client_id).and_then(|sessions| {
        sessions.iter()
            .find(|s| s.read().gateway_session_key == gateway_key)
            .cloned()
    })
}
```

### Step 10: Write tests

**File**: `crates/lr-mcp-via-llm/src/tests.rs`

- `normalize_for_hash` — whitespace trim/collapse, Unicode NFC
- `compute_message_hashes` — deterministic, role-sensitive
- `score_session_match` — prefix (1.0), suffix-anchored, no match (0.0), empty
- `reconstruct_history` — full reconstruction with hidden MCP messages, server instruction stripping, system message update, no-anchor fallback, empty full_messages
- `get_or_create_session` with messages — match, no match, multiple sessions
- Pending execution with `gateway_session_key`

### Step 11: Plan review, test coverage review, bug hunt

### Step 12: Commit

## Key Files

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `unicode-normalization` workspace dep |
| `crates/lr-mcp-via-llm/Cargo.toml` | Add `unicode-normalization` dep |
| `crates/lr-mcp-via-llm/src/session.rs` | Hashing, scoring, `reconstruct_history`, `client_message_hashes` field, `gateway_session_key` on `PendingMixedExecution` |
| `crates/lr-mcp-via-llm/src/manager.rs` | History-based `get_or_create_session`, reconstruction call, `find_session_by_gateway_key`, updated resume flow |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Pass `gateway_session_key` to `PendingMixedExecution` |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Pass `gateway_session_key` to `PendingMixedExecution` |
| `crates/lr-mcp-via-llm/src/tests.rs` | Comprehensive test coverage |

## Existing Code to Reuse

- `ChatMessageContent::as_text()` in `lr-providers/src/lib.rs:1234`
- `inject_server_instructions()` in `orchestrator.rs:1502` — already inserts separate system message, reconstruction strips old one so it gets re-injected fresh
- `resume_after_mixed()` in `orchestrator.rs:835` — has its own reconstruction for mixed tool case, not changed

## Verification

1. `cargo test -p lr-mcp-via-llm` — all tests pass
2. `cargo clippy && cargo fmt`
3. Manual test:
   - `cargo tauri dev` with MCP-via-LLM client
   - Turn 1: Send message → observe MCP tool calls in logs → get response
   - Turn 2: Send follow-up → verify logs show reconstructed history includes previous MCP tool calls
   - Parallel: Open second chat with same client → verify separate session created (different `mcp-via-llm-` keys in logs)
