# Fix Pending Monitor Events: RAII Guard Pattern

## Context

Combined monitor events (those using the emit-then-complete pattern) get stuck in `Pending` state forever when requests fail validation, access checks, rate limiting, or other pre-flight checks. The root cause: `emit_llm_call()` is called early in each route handler, but many early `return Err(...)` paths and `?` operators exit without calling `complete_llm_call` or `complete_llm_call_error`.

**Affected routes and uncovered early-return count:**
- `chat.rs`: ~14 paths (validation, auto-router, model access, firewall, rate limits, secret scan, compression, RouteLLM, guardrails)
- `completions.rs`: ~7 paths
- `moderations.rs`: ~6 paths
- `embeddings.rs`: ~5 paths
- `images.rs`: ~3 paths
- `audio.rs`: ~3+ paths (3 sub-handlers)

**Total: ~38 code paths leaving LlmCall events permanently Pending.**

GuardrailScan, SecretScan, and McpToolCall events are already properly completed on all paths.

## Approach: RAII Guard + Stale Event Sweep

Use Rust's `Drop` trait to guarantee event completion. A guard struct wraps the event ID; if not explicitly completed, `Drop` automatically marks the event as `Error`. A background sweep catches edge cases (leaked guards in spawned tasks, panics).

Key enabler: `MonitorEventStore::update()` is fully synchronous (`parking_lot::RwLock`), so `Drop` can safely call it.

---

## Step 1: Create `MonitorEventGuard` in `crates/lr-monitor/src/guard.rs` (NEW)

Generic guard for any combined event type:

```rust
pub struct MonitorEventGuard {
    store: Arc<MonitorEventStore>,
    event_id: String,
    event_type: MonitorEventType,
    created_at: std::time::Instant,
    completed: bool,
}
```

- `event_id(&self) -> &str` - getter
- `defuse(mut self) -> String` - marks completed, returns event_id (consumes guard)
- `Drop` impl: if `!self.completed`, calls `store.update()` to set `EventStatus::Error` with message like `"Request terminated without completion"` and computed `duration_ms`

Add `mod guard; pub use guard::MonitorEventGuard;` to `crates/lr-monitor/src/lib.rs`.

Constructor takes `Arc<MonitorEventStore>` — the store is already `Arc`-wrapped in `AppState`.

## Step 2: Create `LlmCallGuard` in `crates/lr-server/src/routes/monitor_helpers.rs`

Thin wrapper around `MonitorEventGuard` with LLM-specific completion methods:

```rust
pub struct LlmCallGuard {
    inner: MonitorEventGuard,
}
```

Methods:
- `event_id(&self) -> &str`
- `complete(self, state, provider, model, status_code, ...)` - calls existing `complete_llm_call()`, then `inner.defuse()`
- `complete_error(self, state, provider, model, status_code, error_msg)` - calls existing `complete_llm_call_error()`, then `inner.defuse()`
- `into_event_id(self) -> String` - for streaming handlers that move the ID into a spawned task; defuses guard

Change `emit_llm_call()` return type from `String` to `LlmCallGuard`.

## Step 3: Update `AppState` to expose `Arc<MonitorEventStore>`

- File: `crates/lr-server/src/state.rs`
- `monitor_store` field is likely already `Arc<MonitorEventStore>` — verify and pass the Arc clone to the guard constructor in `emit_llm_call`.

## Step 4: Update route handlers

For each route handler, replace `llm_event_id: String` with `llm_guard: LlmCallGuard`:

### Non-streaming paths
```rust
// Before:
let llm_event_id = emit_llm_call(...);
// ... early returns leave it Pending ...
complete_llm_call(&state, &llm_event_id, ...);

// After:
let llm_guard = emit_llm_call(...);
// ... early returns trigger Drop → Error automatically ...
// At provider error:
llm_guard.complete_error(state, provider, model, 502, &error_msg);
// At success:
llm_guard.complete(state, provider, model, 200, ...);
```

### Streaming paths (chat.rs, completions.rs)
The event ID is moved into a `tokio::spawn` closure:
```rust
let llm_event_id = llm_guard.into_event_id(); // defuses guard
// Spawned task manages completion manually (already has success + error paths)
tokio::spawn(async move {
    // ... existing code using llm_event_id ...
    complete_llm_call(&state, &llm_event_id, ...);
});
```

### Files to update:
1. **`crates/lr-server/src/routes/chat.rs`** - Main handler + `handle_streaming`, `handle_non_streaming`, `handle_streaming_parallel`, `handle_non_streaming_parallel`, `handle_mcp_via_llm`, `build_non_streaming_response`
2. **`crates/lr-server/src/routes/completions.rs`** - Main handler + streaming/non-streaming sub-handlers
3. **`crates/lr-server/src/routes/audio.rs`** - 3 sub-handlers (transcription, translation, speech)
4. **`crates/lr-server/src/routes/embeddings.rs`** - Single handler
5. **`crates/lr-server/src/routes/images.rs`** - Single handler
6. **`crates/lr-server/src/routes/moderations.rs`** - Single handler

### Key pattern for passing guard through sub-functions:
Where `llm_event_id` is passed to sub-functions like `handle_streaming(...)`, pass `llm_guard` instead (or `llm_guard.into_event_id()` if the sub-function manages completion).

## Step 5: Add `sweep_stale_pending()` safety net

In `crates/lr-monitor/src/store.rs`:

```rust
pub fn sweep_stale_pending(&self, max_age: Duration) -> usize {
    let cutoff = Utc::now() - max_age;
    let mut events = self.events.write();
    let mut count = 0;
    for event in events.iter_mut() {
        if event.status == EventStatus::Pending && event.timestamp < cutoff {
            event.status = EventStatus::Error;
            event.duration_ms = Some(max_age.num_milliseconds() as u64);
            // Set error in event data where applicable
            count += 1;
        }
    }
    count
}
```

Wire into server startup as a periodic background task (every 60s, max_age 5 minutes). Location: wherever the Axum server/background tasks are initialized.

## Step 6: Tests

1. **Unit test for `MonitorEventGuard` Drop** - Create guard, drop without completing, verify event is `Error`
2. **Unit test for `MonitorEventGuard` defuse** - Create guard, defuse, verify event stays `Pending` (not auto-errored)
3. **Unit test for `sweep_stale_pending`** - Push old Pending events, sweep, verify they become `Error`
4. **Existing tests** - Run `cargo test` to verify no regressions

## Step 7: Mandatory final steps

1. **Plan review** - Review plan against implementation for missed items
2. **Test coverage review** - Ensure all new code has tests
3. **Bug hunt** - Re-read implementation looking for: guards accidentally dropped too early in streaming paths, missing `into_event_id()` calls, race conditions in sweep

## Verification

1. `cargo test` - all tests pass
2. `cargo clippy` - no warnings
3. Manual test: send a request with an invalid model → verify the LlmCall event shows as `Error` (not `Pending`) in the monitor UI
4. Manual test: send a valid streaming request → verify the event completes as `Complete` normally
5. Check monitor event list for any remaining `Pending` events after a failed request
