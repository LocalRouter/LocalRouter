# Strategy UI Deprecation

**Date**: 2026-02-10
**Status**: Applied
**Reversible**: Yes - all changes are commented out, not deleted

## Purpose

Deprecate the shared/multi-client Strategy concept in the UI. Previously, clients could be connected to any strategy, including shared strategies across multiple clients. Now we enforce a 1:1 client-to-strategy relationship. The backend functionality is untouched; only the UI surfaces for managing and selecting strategies independently have been hidden.

## Search Marker

All commented-out code is tagged with:
```
DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
```

To find all changes: `grep -r "DEPRECATED.*Strategy UI hidden" src/`

## Files Changed

### 1. `src/views/settings/index.tsx` - Settings View
- **Commented out**: "Strategies" tab trigger in the settings TabsList
- **Commented out**: `RoutingTab` TabsContent and its import
- **Commented out**: `handleDetailChange` function (only used by routing tab)
- **Updated**: Description text removed "strategies" mention
- **To revert**: Uncomment the import, tab trigger, TabsContent, and `handleDetailChange`. Restore description text.

### 2. `src/views/resources/index.tsx` - Resources / LLM Providers View
- **Commented out**: "Model Strategies" tab trigger
- **Commented out**: `StrategiesPanel` TabsContent and its import
- **Updated**: Description text from "Manage providers and strategies" to "Manage LLM providers"
- **To revert**: Uncomment the import, tab trigger, and TabsContent. Restore description text.

### 3. `src/views/clients/tabs/models-tab.tsx` - Client Models Tab
- **Commented out**: The entire Strategy selector Card (strategy dropdown, "Create Personal Strategy" button, shared strategy warning alert)
- **Commented out**: `isSharedStrategy`, `ownedStrategies` computed values
- **Commented out**: `handleStrategyChange`, `handleCreatePersonalStrategy` functions
- **Commented out**: Unused imports (`Badge`, `Button`, `Select` components, `Alert` components, `Route` and `AlertTriangle` icons)
- **Changed**: Removed `ml-4` nesting indentation on Rate Limits + Model Configuration sections (they were visually nested under the strategy card); now uses `space-y-4` for direct layout
- **To revert**: Uncomment all the above. Restore the `ml-4` wrapper div around Rate Limits and Model Configuration. Restore `onViewChange` destructuring (remove underscore prefix).

### 4. `src/views/try-it-out/llm-tab/index.tsx` - Try-it-out LLM Tab
- **Commented out**: `Strategy` interface
- **Commented out**: Strategy mode state (`strategies`, `selectedStrategy`, `strategyToken`)
- **Commented out**: "Against Strategy" radio button in the mode selector
- **Commented out**: Strategy selector dropdown (shown when mode === "strategy")
- **Commented out**: `list_strategies` call in init `Promise.all`
- **Commented out**: Strategy default selection logic
- **Commented out**: Strategy test client creation `useEffect`
- **Commented out**: `"strategy"` case in `getAuthToken` switch
- **Commented out**: `"strategy"` case in `getModeDescription` switch
- **Commented out**: `Route` icon import
- **Changed**: `TestMode` type from `"client" | "strategy" | "direct"` to `"client" | "direct"` (with commented union member)
- **Updated**: Mode descriptions to say "routing pipeline" instead of "strategy pipeline"
- **To revert**: Uncomment all the above. Restore `TestMode` union. Restore `strategyToken` in `getAuthToken` deps array.

### 5. `src/views/try-it-out/index.tsx` - Try-it-out Index
- **Changed**: `mode` type in `llmInitial` state from `"client" | "strategy" | "direct"` to `"client" | "direct"`
- **To revert**: Uncomment `"strategy"` in the union type.

### 6. `src/components/layout/command-palette.tsx` - Command Palette
- **Commented out**: "Strategies" item in Settings command group
- **Commented out**: Strategies search results group (the whole block that lists strategies by name)
- **Commented out**: `Route` icon import
- **Changed**: `strategies` destructured param to `_strategies` (unused suppression)
- **To revert**: Uncomment all the above. Rename `_strategies` back to `strategies`.

### 7. `src/components/layout/app-shell.tsx` - App Shell
- **Commented out**: `Strategy` interface
- **Commented out**: `strategies` state and `setStrategies`
- **Commented out**: `loadStrategies` function
- **Commented out**: `strategies-changed` event listener
- **Commented out**: `loadStrategies()` call in `loadData`
- **Changed**: `strategies` prop on `CommandPalette` to pass `[]` with deprecation comment
- **To revert**: Uncomment all the above. Restore `strategies={strategies}` prop.

### 8. `src/components/Sidebar.tsx` - Old Sidebar (legacy)
- **Commented out**: `Strategy` interface
- **Commented out**: `strategies` state
- **Commented out**: `loadStrategies()` call and function
- **Commented out**: `strategies-changed` event listener and cleanup
- **Commented out**: `{ id: 'routing', label: 'Strategies' }` from `mainTabs` array
- **Commented out**: Strategies sub-tabs expansion section (the expandable list of strategy names)
- **To revert**: Uncomment all the above.

## Files NOT Changed (still reference strategies internally)

These files still exist and reference strategies but are no longer reachable from the UI:

- `src/views/settings/routing-tab.tsx` - Full strategy management UI (import commented out)
- `src/views/resources/strategies-panel.tsx` - Strategy panel with list/detail view (import commented out)
- `src/components/strategy/` - Strategy configuration components (still used by models-tab for the actual model config)
- `src/components/strategies/` - Strategy editor components (RateLimitEditor still used)
- `src/types/tauri-commands.ts` - Strategy TypeScript types (still needed by models-tab)

## Backend (No Changes)

The Rust backend is completely untouched. All strategy-related Tauri commands still exist and function:
- `list_strategies`
- `create_strategy`
- `update_strategy`
- `delete_strategy`
- `assign_client_strategy`
- `create_test_client_for_strategy`

## How to Fully Revert

1. Search for `DEPRECATED.*Strategy UI hidden` across `src/`
2. Uncomment all marked sections
3. Restore original import lines
4. In `models-tab.tsx`, restore the `ml-4` wrapper div nesting
5. In `command-palette.tsx`, rename `_strategies` back to `strategies`
6. In `models-tab.tsx`, rename `_onViewChange` back to `onViewChange`
7. In `app-shell.tsx`, restore `strategies={strategies}` prop
8. Run `npx tsc --noEmit` to verify
