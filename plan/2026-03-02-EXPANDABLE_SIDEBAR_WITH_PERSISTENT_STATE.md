# Expandable Sidebar with Persistent State

## Context
The sidebar currently shows only icons (48px wide) with tooltip labels on hover. The user wants the ability to expand the sidebar to show section names alongside icons, with a toggle to collapse/expand, persisted in config.

## Approach

### 1. Backend: Add `sidebar_expanded` to UiConfig
**File: `src-tauri/src/config/types.rs`**
- Add `sidebar_expanded: bool` field to `UiConfig` struct (default: `false`)
- Update `Default for UiConfig` impl

### 2. Backend: Add Tauri commands
**File: `src-tauri/src/ui/commands.rs`**
- `get_sidebar_expanded()` → returns `bool`
- `set_sidebar_expanded(expanded: bool)` → updates config and saves

Follow the existing `get_tray_graph_settings`/`update_tray_graph_settings` pattern.

### 3. Register commands
**File: `src-tauri/src/main.rs`**
- Add both commands to `invoke_handler`

### 4. TypeScript types
**File: `src/types/tauri-commands.ts`**
- Add `SetSidebarExpandedParams` interface

### 5. Demo mock
**File: `website/src/components/demo/TauriMockSetup.ts`**
- Add mock handlers for `get_sidebar_expanded` and `set_sidebar_expanded`

### 6. Frontend: Sidebar component
**File: `src/components/layout/sidebar.tsx`**
- Add `expanded` state, load from config on mount
- Toggle button (chevron icon) at bottom of sidebar
- When expanded (~180px wide):
  - Nav items show icon + label text inline
  - Logo area shows "LocalRouter" text
  - Tooltips disabled (labels visible)
  - Shortcuts shown as subtle kbd badges
- When collapsed (48px, current behavior):
  - Icons only with tooltips (unchanged)
- Smooth width transition with CSS

### 7. App Shell layout
**File: `src/components/layout/app-shell.tsx`**
- No changes needed - sidebar width is self-contained via flex, main content uses `flex-1`

## Files to Modify
1. `src-tauri/src/config/types.rs` - UiConfig struct + default
2. `src-tauri/src/ui/commands.rs` - Two new Tauri commands
3. `src-tauri/src/main.rs` - Register commands
4. `src/types/tauri-commands.ts` - TypeScript params type
5. `website/src/components/demo/TauriMockSetup.ts` - Mock handlers
6. `src/components/layout/sidebar.tsx` - Expand/collapse UI

## Verification
1. `cargo test && cargo clippy && cargo fmt` - Rust checks
2. `npx tsc --noEmit` - TypeScript type checks
3. `cargo tauri dev` - Visual verification: toggle sidebar, reload app (state persists)
