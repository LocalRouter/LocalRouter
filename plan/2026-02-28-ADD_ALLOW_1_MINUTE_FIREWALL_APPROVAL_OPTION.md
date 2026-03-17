# Add "Allow 1 Minute" Firewall Approval Option

## Context
When a model has "Ask" permission, each HTTP request triggers a separate popup. Clients like Claude Code send multiple requests per user interaction (main response, tool calls, etc.), causing 3+ popups for a single action. Adding a 1-minute time-based approval gives users a lightweight way to suppress popup spam without committing to a full hour.

## Changes

### 1. Add `Allow1Minute` enum variant
**File:** `crates/lr-mcp/src/gateway/firewall.rs`
- Add `Allow1Minute` with `#[serde(rename = "allow_1_minute")]` to `FirewallApprovalAction`

### 2. Add convenience methods to trackers
**File:** `crates/lr-server/src/state.rs`
- `ModelApprovalTracker::add_1_minute_approval()` → calls `add_approval(..., Duration::from_secs(60))`
- `FreeTierApprovalTracker::add_approval(client_id, duration)` → new generic method (currently only has `add_1_hour_approval`), then `add_1_minute_approval()` convenience
- `GuardrailApprovalTracker` already has generic `add_bypass(client_id, duration)` → just add `add_1_minute_bypass()`

### 3. Handle `Allow1Minute` in submit handler
**File:** `src-tauri/src/ui/commands_clients.rs`
- Add `FirewallApprovalAction::Allow1Minute` to the `pending_info` match guard (~line 696)
- Add `FirewallApprovalAction::Allow1Minute` branch after `Allow1Hour` (~line 776), calling the 1-minute tracker methods for each type (guardrail, free-tier, model)

### 4. Add to allow match arms in route handlers
- `crates/lr-server/src/routes/chat.rs`: Add `Allow1Minute` to allow arms at lines 662 and 930. Add specific handling at line 1316 for free-tier (call `add_1_minute_approval`).
- `crates/lr-server/src/routes/completions.rs`: Add `Allow1Minute` to allow arm at line 385.
- `crates/lr-mcp/src/gateway/gateway_tools.rs`: Add `Allow1Minute` to allow arm at line 559.

### 5. Update TypeScript types
**File:** `src/types/tauri-commands.ts`
- Add `'allow_1_minute'` to `FirewallApprovalAction` union type

### 6. Update frontend UI
**File:** `src/components/shared/FirewallApprovalCard.tsx`
- Add `"allow_1_minute"` to `ApprovalAction` type
- Add "Allow for 1 Minute" `DropdownMenuItem` before the "Allow for 1 Hour" item, with the same conditional (`isModelRequest || isGuardrailRequest || isFreeTierFallback`)

## Verification
- `cargo test && cargo clippy` to ensure compilation and no warnings
- `npx tsc --noEmit` to verify TypeScript types
- Manual test: configure a model with "Ask" permission, send a request, verify "Allow for 1 Minute" appears in the dropdown, and subsequent requests within 1 minute skip the popup
