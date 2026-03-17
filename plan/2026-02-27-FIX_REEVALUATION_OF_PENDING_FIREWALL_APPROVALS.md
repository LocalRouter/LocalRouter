# Fix: Re-evaluation of Pending Firewall Approvals

## Context

The re-evaluation in `reevaluate_pending_approvals` (commands_clients.rs) has two problems:
1. **Only triggers from `submit_firewall_approval`** — should also trigger when permissions change from the settings UI (e.g., `set_client_mcp_permission`, `set_client_model_permission`, etc.)
2. **Has a hacky resource-identity matching fallback** — when the client isn't in config or `check_needs_approval` returns `Ask`, it compares `tool_name + server_name + client_id + request type` to the just-submitted request. This is wrong; it should only use `check_needs_approval` with fresh config.

Features 1 (focus-based delay) and 3 (debug multi-popup) are already implemented. This plan only covers the re-evaluation fix.

---

## Approach: Event-Based + Explicit Call

All permission-modifying commands already emit `app.emit("clients-changed", ())`. There's already a listener for this event in `main.rs:666` that handles MCP notification broadcasts. We add a second listener that triggers re-evaluation.

**Two trigger points:**
1. **`clients-changed` event listener** (new, in `main.rs`) — catches all config/permission changes from the settings UI AND from `submit_firewall_approval`'s `AllowPermanent`/`DenyAlways` (which update config and emit the event)
2. **Explicit call in `submit_firewall_approval`** (existing) — catches time-based tracker updates (`Allow1Hour`, `Deny1Hour`, guardrail bypass/denial) that don't modify config and don't emit events

If both fire for the same action (e.g., `AllowPermanent` updates config → emits event → listener calls re-eval, AND the explicit call also runs), the second call harmlessly finds nothing left to resolve.

---

## Changes

### 1. Clean up `reevaluate_pending_approvals` (commands_clients.rs)

Remove the `submitted_info` and `submitted_action` parameters and all resource-identity matching logic. The function becomes:

```rust
fn reevaluate_pending_approvals(
    app: &tauri::AppHandle,
    firewall_manager: &FirewallManager,
    config_manager: &ConfigManager,
    model_approval_tracker: &ModelApprovalTracker,
    guardrail_approval_tracker: &GuardrailApprovalTracker,
    guardrail_denial_tracker: &GuardrailDenialTracker,
)
```

Logic (unchanged from the config-based path that already exists):
- `firewall_manager.list_pending()` → for each pending session
- Look up client in config. If not found → `continue` (popup stays, e.g. debug client)
- Construct `FirewallCheckContext` based on request type
- Call `check_needs_approval(&ctx)`
- `Allow` → `submit_response(AllowOnce)` + close window
- `Deny` → `submit_response(Deny)` + close window
- `Ask` → do nothing (popup stays)

### 2. Update call in `submit_firewall_approval` (commands_clients.rs)

Change the call at ~line 935 to pass only the 6 clean parameters (no `submitted_info`, no `submitted_action`).

### 3. Add `clients-changed` event listener (main.rs)

After the existing `clients-changed` listener at line 666, add a second listener:

```rust
let app_state_for_reeval = app_state.clone();
let config_manager_for_reeval = config_manager.clone();
let app_handle_for_reeval = app.handle().clone();
app.listen("clients-changed", move |_event| {
    crate::ui::commands_clients::reevaluate_pending_approvals(
        &app_handle_for_reeval,
        &app_state_for_reeval.mcp_gateway.firewall_manager,
        &config_manager_for_reeval,
        &app_state_for_reeval.model_approval_tracker,
        &app_state_for_reeval.guardrail_approval_tracker,
        &app_state_for_reeval.guardrail_denial_tracker,
    );
});
```

This requires making `reevaluate_pending_approvals` `pub(crate)`.

### 4. Tray rebuild after re-evaluation (main.rs listener)

After re-evaluation in the event listener, also rebuild the tray menu + notify tray activity (so the question mark overlay updates). The `submit_firewall_approval` path already does this.

---

## Files Modified

| File | Change |
|------|--------|
| `src-tauri/src/ui/commands_clients.rs` | Remove `submitted_info`/`submitted_action` params, remove resource-identity fallback, make `pub(crate)` |
| `src-tauri/src/main.rs` | Add `clients-changed` listener for re-evaluation |

---

## Verification

1. `cargo test && cargo clippy && cargo fmt`
2. Manual testing with debug page:
   - Check "Send multiple" → trigger LLM Model → 3 popups appear
   - Allow Always on first → second (same model) auto-closes → third (different model) stays
   - Allow for 1 Hour on first → second auto-closes → third stays
   - Deny Always on first → second auto-closes (denied) → third stays
   - Change permissions in settings UI while popups are open → matching popups auto-resolve
