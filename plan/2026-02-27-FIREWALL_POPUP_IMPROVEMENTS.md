# Firewall Approval Popup Improvements

## Context

The firewall approval system opens a separate native popup window for each pending request (model access, MCP tool, skill, guardrail). Two UX issues:
1. Popups appear suddenly and the user can accidentally click Allow/Deny before reading.
2. When multiple popups are queued for the same resource, the user must manually respond to each even after a persistent action (e.g., "Allow Always") that covers them all.

Additionally, the permission-check logic is currently duplicated across call sites. This plan unifies it into a single function used by both the original trigger and the re-evaluation.

## Features

### Feature 1: 1-Second Button Delay (Focus-Based)
- Uses Tauri's `onFocusChanged` API so delay starts on window focus, not mount
- Buttons disabled until 1s after window gains focus
- No changes to FirewallApprovalCard.tsx

### Feature 2: Unified `check_needs_approval` Function
- New `FirewallCheckContext` enum and `check_needs_approval` in `access_control.rs`
- Refactor `gateway_tools.rs` and `chat.rs` to use it
- Add `reevaluate_pending_approvals` in `commands_clients.rs` to auto-resolve pending popups

### Feature 3: Debug Multi-Popup Mode
- Add `send_multiple` param to `debug_trigger_firewall_popup`
- Creates 3 sessions: 2 same resource + 1 different
- Checkbox in debug UI

## Files Modified

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/access_control.rs` | Add unified check function + tests |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Refactor to use unified function |
| `crates/lr-server/src/routes/chat.rs` | Refactor to use unified function |
| `src-tauri/src/ui/commands_clients.rs` | Add re-evaluation logic |
| `src/views/firewall-approval.tsx` | Focus-based button delay |
| `src-tauri/src/ui/commands.rs` | Multi-popup debug mode |
| `src/views/debug/index.tsx` | Multi-popup checkbox |
| `src/types/tauri-commands.ts` | Add params type |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock |
