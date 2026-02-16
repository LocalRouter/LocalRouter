# Plan: Static Tray Icon Mode + Appearance Settings Tab

## Context

The tray graph was previously always-on with no option for a static icon. Users who prefer a clean, non-animated tray icon need a way to disable the activity graph while retaining notification overlays (approval requests, health issues, update available). The UI preferences for the tray are currently buried in the Server settings tab and deserve their own dedicated tab.

The `tray_graph_enabled` config field already exists but is forced to `true` everywhere. The `enabled` param in `update_tray_graph_settings` is accepted but ignored. This plan re-activates that infrastructure and adds the UI.

Additionally, the notification overlay priority needs reordering: Approval requests should be highest priority (not health).

## Changes

### 1. Config default change
**File: `crates/lr-config/src/types.rs`**
- Line 2034: Change `tray_graph_enabled` default from `true` to `false`
- Line 394: Update doc comment to explain static vs graph modes
- `serde(default)` on the field means existing configs without the field will get `false` (static) — but existing configs that already have `tray_graph_enabled: true` saved will keep their value

### 2. Respect `enabled` param in update command
**File: `src-tauri/src/ui/commands.rs`**
- Line 1252: Remove `let _ = enabled;` suppression
- Line 1257: Change `config.ui.tray_graph_enabled = true` to `config.ui.tray_graph_enabled = enabled`
- Line 1239: Update doc comment

### 3. Extract shared overlay determination + fix priority order
**File: `src-tauri/src/ui/tray_graph_manager.rs`**
- Extract lines 447-481 into a new public function `determine_overlay(app_handle, dark_mode) -> TrayOverlay`
- **Change priority order** to: Firewall Pending > Health Warning/Error > Update Available > None
- Replace inline logic in `update_tray_graph_impl()` with call to `determine_overlay()`
- Fix `is_enabled()` (line 547): return `self.config.read().tray_graph_enabled` instead of hardcoded `true`

### 4. Static icon mode in graph manager
**File: `src-tauri/src/ui/tray_graph_manager.rs`**
- In `update_tray_graph_impl()`, before data collection: read `tray_graph_enabled` from config
- When `false`: skip bucket shifting and data point collection, pass empty `vec![]` to `generate_graph()`
- `generate_graph(&[], config, overlay, dark_mode)` already renders a clean rounded-rect border with overlay — no bars
- The background task still runs (wakes on health events, firewall changes) so overlays update in real-time
- Hash comparison prevents redundant icon updates when nothing changes

### 5. New Appearance settings tab (frontend)
**File: `src/views/settings/appearance-tab.tsx`** (new)
- Tray Icon Mode selector: "Static Icon" (default) / "Activity Graph"
- Graph Refresh Rate selector (conditionally shown when graph mode selected)
- Description text explaining each mode
- Single save button calling existing `update_tray_graph_settings` command
- Reuse `TrayGraphSettings` and `UpdateTrayGraphSettingsParams` types from `tauri-commands.ts`

### 6. Register Appearance tab
**File: `src/views/settings/index.tsx`**
- Import `AppearanceTab`
- Add `<TabsTrigger value="appearance">Appearance</TabsTrigger>` after Server
- Add `<TabsContent value="appearance"><AppearanceTab /></TabsContent>`

### 7. Remove UI Preferences from Server tab
**File: `src/views/settings/server-tab.tsx`**
- Remove the "UI Preferences" Card (lines 344-386)
- Remove related state (`trayGraphSettings`, `isUpdatingTrayGraph`)
- Remove `loadTrayGraphSettings()`, `updateTrayGraphSettings()`, `calculateTimeWindow()` functions
- Remove unused imports

### 8. Update demo mocks
**File: `website/src/components/demo/mockData.ts`**
- Change `trayGraphSettings.enabled` default to `false`

**File: `website/src/components/demo/TauriMockSetup.ts`**
- Update mock handler for `update_tray_graph_settings` to store both `enabled` and `refreshRateSecs`

## Verification

1. `cargo build` — compile check
2. `cargo test` — existing tests pass
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — TypeScript types check
5. Manual: switch to static icon mode, verify clean icon with overlay on health/firewall/update events
6. Manual: switch to graph mode, verify graph renders with correct overlay
7. Manual: trigger firewall + health simultaneously, verify firewall overlay wins (new priority)
