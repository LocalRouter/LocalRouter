# Incremental Model Loading

**Date**: 2026-03-07
**Status**: In Progress

## Problem
With many providers configured, model loading blocks the UI for seconds because:
1. Providers are fetched **sequentially** (N providers × latency = total wait)
2. No cached data shown while loading
3. No loading states in model selection components

## Solution

### Phase 1: Parallelize Backend
- Change `list_all_models()` in `registry.rs` from sequential loop to `futures::future::join_all()`
- All providers fetched concurrently → latency = max(provider latencies) instead of sum

### Phase 2: Incremental Loading
- Add `get_cached_models` Tauri command → returns cached models instantly (no network)
- Add `refresh_models_incremental` Tauri command → spawns parallel background per-provider fetches
- Emits `models-provider-loaded` event per provider as each completes
- Emits `models-changed` when all done (backward compat)

### Phase 3: Frontend Updates
- `app-shell.tsx`: Show cached models instantly, trigger incremental refresh, merge per-provider events
- `Sidebar.tsx`: Use cached models on mount, listen to events for updates
- Add loading states to safety/guardrails model selection components

## Files Modified
- `crates/lr-providers/src/registry.rs` - Parallelize + new methods
- `src-tauri/src/ui/commands_providers.rs` - New Tauri commands
- `src-tauri/src/main.rs` - Register commands
- `src/components/layout/app-shell.tsx` - Incremental loading
- `src/components/Sidebar.tsx` - Cached + event-driven
- `src/types/tauri-commands.ts` - New types
- `website/src/components/demo/TauriMockSetup.ts` - Mock handlers
- Safety/guardrails model selection components - Loading states
