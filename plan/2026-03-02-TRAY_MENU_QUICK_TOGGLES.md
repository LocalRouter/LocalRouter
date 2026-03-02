# Tray Menu Refactor — Replace Strategy Selector with Quick Toggles + Dead Code Cleanup

**Date**: 2026-03-02
**Status**: Implemented

## Summary

Replaced the obsolete "Model strategy" selector in the system tray with quick-toggle items for rate limits, free tier mode, and weak model routing. Added a Settings shortcut per client. Introduced `enabled` field on `StrategyRateLimit`. Cleaned up dead strategy-switching code.

## Changes

### New Per-Client Tray Menu Structure

```
Client Name (disabled header)
● Enabled / ○ Disabled
⧉ Copy Client ID (OAuth)
⧉ Copy API Key / Client Secret
⚙ Settings                           ← opens client in UI
────────────────
[if rate_limits non-empty:]
Rate Limits (disabled header)
  ✓  100 requests / hr               ← toggle enabled field
     $5.00 / day                      ← disabled rate limit
  ✓  Free Tier Mode                   ← toggle free_tier_only
[if auto_config.enabled AND routellm has weak_models:]
  ✓  Weak Model Routing               ← toggle routellm_config.enabled
────────────────
MCP Allowlist ...
Skills Allowlist ...
Coding Agents ...
```

### Files Modified

**Step 1: `enabled` field on StrategyRateLimit**
- `crates/lr-config/src/types.rs` — Added `enabled: bool` with `default_true`
- `src-tauri/src/config/types.rs` — Mirror
- `src/types/tauri-commands.ts` — Added `enabled?: boolean`
- `crates/lr-config/src/validation.rs` — Updated 5 test constructors

**Step 2: Rate limit enforcement**
- `crates/lr-router/src/lib.rs` — Skip disabled limits
- `src-tauri/src/router/mod.rs` — Mirror

**Step 3: Tray menu building**
- `src-tauri/src/ui/tray_menu.rs` — Replaced strategy selector with quick toggles, added `format_rate_limit` helper, added Settings item, added 4 new handlers, removed `handle_set_client_strategy`

**Step 4: Event routing**
- `src-tauri/src/ui/tray.rs` — Updated imports and event routing for new handlers

**Step 5: Dead code cleanup**
- `src-tauri/src/ui/commands_clients.rs` — Removed `assign_client_strategy`
- `src-tauri/src/main.rs` — Removed registration
- `crates/lr-clients/src/manager.rs` — Removed `set_client_strategy`
- `src-tauri/src/clients/mod.rs` — Removed `set_client_strategy`
- `crates/lr-config/src/lib.rs` — Removed `assign_client_strategy`
- `src-tauri/src/config/mod.rs` — Removed `assign_client_strategy`
- `src/types/tauri-commands.ts` — Removed `AssignClientStrategyParams`
- `website/src/components/demo/TauriMockSetup.ts` — Removed mock

**Step 6: Website demo**
- `website/src/components/demo/MacOSTrayMenu.tsx` — Replaced strategy selector with quick toggles
- `website/src/components/demo/mockData.ts` — Added sample rate_limits, weak_models, free_tier_only

**Step 7: Website text fix**
- `website/src/pages/Home.tsx` — Added " theme" after WinXP link
