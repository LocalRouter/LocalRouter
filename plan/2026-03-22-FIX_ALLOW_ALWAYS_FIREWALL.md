# Fix "Allow Always" Firewall Approval Not Persisting

## Context

When users select "Allow Always" in firewall approval popups, the permission doesn't persist — they get prompted again on the next request. The user reports seeing "allow once" in logs despite clicking "Allow Always" in the dropdown.

Two root causes identified:

1. **Dropdown click-through bug (primary)**: When clicking "Allow Always" in the Radix dropdown, the portal is removed on `pointerup`, causing the subsequent browser `click` event to fall through to the "Allow Once" button underneath. Both `handleAction("allow_permanent")` AND `handleAction("allow_once")` fire. Since `setSubmitting(true)` is a batched React state update, it doesn't block the second call. If `allow_once` wins the IPC race to the backend, it consumes the pending request without updating config, and `allow_permanent` fails because the request is already gone.
2. **Backend race condition (secondary)**: Even when only `allow_permanent` fires, `submit_response()` unblocks the gateway BEFORE `config_manager.update()` persists the permission. A fast MCP client can send the next request before `ClientManager` is synced.

## Changes

### 1. Fix dropdown click-through with ref guard

**File**: `src/views/firewall-approval.tsx`

Add a `useRef` guard to `handleAction` to prevent double-invocation. React state (`submitting`) is batched and doesn't take effect immediately, but a ref update is synchronous.

```tsx
// Add at component top:
const submittingRef = useRef(false)

// Modify handleAction (line 430):
const handleAction = async (action: ApprovalAction) => {
    if (!details || submittingRef.current) return
    submittingRef.current = true
    setSubmitting(true)
    // ... rest unchanged
    // In catch block, also reset: submittingRef.current = false
}
```

This ensures that when the click-through fires `handleAction("allow_once")`, the ref guard blocks it because the first call (`allow_permanent`) already set it to `true`.

**Note**: The same click-through bug exists on the Deny side (`deny_always` → `deny`), but it's less harmful since `deny` doesn't update config. The ref guard fixes both sides.

### 2. Fix backend race condition — update config BEFORE submitting response

**File**: `src-tauri/src/ui/commands_clients.rs` — `submit_firewall_approval()`

Current flow (lines 1266-1646):
1. `submit_response()` → unblocks gateway → MCP client gets response → may send next request
2. Config update → `config_manager.update()` → `sync_clients()` → `save()` → `emit("clients-changed")`

New flow:
1. **Phase 1 (pre-submit)**: For persistent actions (`AllowPermanent`, `DenyAlways`, `BlockCategories`, `AllowCategories`, `DisableClient`), run `config_manager.update()` BEFORE `submit_response()`. This synchronously updates in-memory config AND syncs `ClientManager` (via the `sync_clients` callback registered in `main.rs:217-219`). Also move time-based tracker updates (`Allow1Minute`, `Allow1Hour`, `AllowSession`, `DenySession`) to pre-submit.
2. **Phase 2 (submit)**: Call `submit_response()` — gateway unblocks. By now `ClientManager` has the updated permissions.
3. **Phase 3 (post-submit)**: `config_manager.save()`, `emit("clients-changed")`, `reevaluate_pending_approvals()`, tray rebuild.

If config update fails in Phase 1, log error but still proceed with `submit_response()` so the current request is allowed.

Refactor approach: inline the `config_manager.update()` closures from the existing helpers (`update_permission_for_allow_permanent`, `update_model_permission_for_allow_permanent`, etc.) directly in the pre-submit phase. Keep `save()` + `emit()` in Phase 3. Use a `needs_save: bool` flag to track whether disk persistence is needed after submit.

### 3. Improve diagnostic logging

**File**: `src-tauri/src/ui/commands_clients.rs`

- After config update succeeds (Phase 1): `"AllowPermanent: config updated for client={}, key={}. ClientManager synced before response."`
- When `pending_info` is None: enhance existing warning with context
- In `reevaluate_pending_approvals` (line 1798): change log prefix from `"Auto-resolving"` to `"Re-evaluation: auto-resolving"` to distinguish from user actions

## Files to Modify

| File | Change |
|------|--------|
| `src/views/firewall-approval.tsx` | Add `useRef` guard to prevent double-invocation of `handleAction` |
| `src-tauri/src/ui/commands_clients.rs` | Reorder config update before submit_response; add logging |

## Verification

1. `cargo test && cargo clippy` — ensure no regressions
2. `npx tsc --noEmit` — verify TypeScript types
3. Manual test: Connect MCP client with `Ask` permissions, trigger tool call, click "Allow Always" from dropdown, verify:
   - Logs show only `AllowPermanent` (no phantom `AllowOnce`)
   - Config is updated (check settings.yaml)
   - No second popup on next tool call
4. Verify "Allow Once" main button still works correctly
5. Check logs show config update confirmation before response submission

## Mandatory Final Steps

1. **Plan Review**: Review plan against implementation for missed changes
2. **Test Coverage Review**: Review code coverage for modified paths
3. **Bug Hunt**: Re-read implementation looking for edge cases
