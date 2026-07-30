# Secret Scan Notify → Existing Popup Window (remove toast)

## Problem

Commit `3fc50ce4` fixed secret scanning for proxy traffic and surfaced the
previously-silent **Notify** action — but surfaced it as an in-app sonner
toast in `App.tsx`. Toasts render inside the main window only, so with the
app in the tray Notify is still invisible, and it is inconsistent with the
rest of the approval/notification UX.

The correct surface already exists: the standalone `firewall-approval-*`
popup window (always-on-top, works with the main window closed) already has
a dedicated secret-scan mode (`is_secret_scan_request` +
`SecretScanApprovalDetails`) used by the **Ask** action. Nothing was ever
deleted — Notify simply was never wired to it.

## Change

Route the Notify action through the existing popup-window infrastructure as
a **notify-only** (non-blocking, informational) variant. Remove the toast.

### Backend

1. `crates/lr-mcp/src/gateway/firewall.rs`
   - `FirewallApprovalSession` + `PendingApprovalInfo`: add
     `is_notify_only: bool`.
   - New `notify_secret_scan(client_id, client_name, model_name, details,
     arguments_preview)`: creates a session with `response_sender: None`,
     `is_secret_scan_request: true`, `is_notify_only: true`, broadcasts the
     same `firewall/approvalRequired` notification on `_firewall` (with
     `is_notify_only: true` in params), returns immediately (no oneshot
     wait). **Dedupe**: if a pending notify-only session already exists for
     the same client, skip creating another (prevents window flood when an
     agent retries with the same secret).
   - `dismiss` path: `cancel_request` already just removes the session —
     reuse it.
2. `crates/lr-server/src/routes/pipeline.rs`
   - Extract a helper building `(SecretScanApprovalDetails, preview)` from
     `ScanResult` (currently built inline in `handle_secret_scan_approval`).
   - Notify arm of `scan_request_for_secrets`: replace
     `state.emit_event("secret-scan-notify", …)` with
     `firewall_manager.notify_secret_scan(…)`. Monitor event emission
     unchanged.
3. `src-tauri/src/main.rs`
   - Firewall popup listener: window title "Secret Scan Notification" when
     `is_notify_only` is set in the notification params (else
     "Approval Required"). No other listener changes — the same listener
     opens the window.
4. `src-tauri/src/ui/commands_clients.rs`
   - `get_firewall_approval_details`: include `is_notify_only`.
   - New `dismiss_firewall_notification(request_id)` command →
     `firewall_manager.cancel_request` + tray rebuild. Register in
     `main.rs` invoke handler.
5. Debug: `debug_trigger_firewall_popup` gets a `secret_scan_notify`
   variant so the popup can be exercised manually.

### Frontend

6. `src/views/firewall-approval.tsx`
   - `ApprovalDetails` type: add `is_notify_only?: boolean`.
   - When notify-only: same secret-scan findings card, but header copy
     "request was allowed" and a single **Dismiss** button calling
     `dismiss_firewall_notification` then closing the window; also dismiss
     on window close (X) so the session doesn't linger in `pending`.
7. `src/App.tsx`: delete the `secret-scan-notify` toast listener.
8. `src/types/tauri-commands.ts` + `website/src/components/demo/TauriMockSetup.ts`:
   params type + mock for `dismiss_firewall_notification`; extend the
   `get_firewall_approval_details` mock with `is_notify_only`.

### Tests

- `firewall.rs`: `notify_secret_scan` returns immediately, session visible
  in `list_pending` with `is_notify_only`, per-client dedupe, dismissal via
  `cancel_request` removes it, expiry via `cleanup_expired` reaps it.

## Mandatory final steps

1. **Plan review** — re-read this plan vs implementation, close gaps.
2. **Test coverage review** — cover new/changed paths.
3. **Bug hunt** — fresh-eyes pass over the diff.
4. **Commit** — only files modified by this work.
