# Plan: Revamp Clients View to Left-List + Right-Detail-Pane Layout

## Overview
Convert the Clients view from the current full-page list → full-page detail pattern to the split-pane layout used by MCP Servers, Providers, and Skills: a resizable left list panel with a right detail pane.

## Current State
- `index.tsx`: Switches between `<ClientList>` (DataTable) and `<ClientDetail>` (full page with back button)
- `client-list.tsx`: Full DataTable with columns (Name, Client ID, Status, Providers, MCP, Last Used, Actions)
- `client-detail.tsx`: Full-page view with header (back button, name, badge, toggle, delete) + 4 Tabs (Config, Models, MCP, Skills)
- Each tab is a separate component in `tabs/`

## Target State
Match the `ProvidersPanel` pattern: `ResizablePanelGroup` with left list (35%) and right detail pane (65%).

## Changes

### 1. Rewrite `src/views/clients/index.tsx`
- Remove the conditional list/detail switching
- Remove the header with "Create Client" button (moves into left panel)
- Add `ResizablePanelGroup` layout like ProvidersPanel
- Parse `activeSubTab` format: `"clientId"` or `"clientId|tab"` (keep existing format)
- Left panel: search input + "+" button + scrollable client list items
- Right panel: selected client detail with tabs inline, or empty state

### 2. Remove `src/views/clients/client-list.tsx`
- No longer needed — the DataTable is replaced by the simple clickable list items in the left panel (same style as providers-panel)

### 3. Rewrite `src/views/clients/client-detail.tsx`
- Remove the back button and outer layout wrapper (no longer a full page)
- Remove the `onBack` prop
- Keep: header (name, badge, Try It Out, toggle, delete menu), tabs (Config, Models, MCP, Skills)
- Wrap in `ScrollArea` for the right pane
- The 4 tabs remain as `<Tabs>` component inside the right pane — this is the natural way to integrate multiple tabs into the detail pane

### 4. Left panel list items
Each client item shows:
- Client name (bold, truncated)
- Client ID subtitle (truncated, muted)
- Enabled/disabled status dot (green/gray, like the health dot in providers)
- Selected state: `bg-accent`, hover: `hover:bg-muted`

### 5. Dialog management
- Create client wizard: triggered by "+" button in left panel header
- Delete confirmation: stays in the detail pane (via EntityActions or dropdown)
- Toggle enable/disable: stays in detail pane header

### 6. Navigation
- `onTabChange("clients", clientId)` — select a client
- `onTabChange("clients", clientId + "|tab")` — select client + specific tab
- `onTabChange("clients", null)` — deselect (show empty state)

## Files to Modify
1. **`src/views/clients/index.tsx`** — Full rewrite to split-pane layout
2. **`src/views/clients/client-detail.tsx`** — Simplify (remove back button, wrap for pane)
3. **`src/views/clients/client-list.tsx`** — Delete (functionality absorbed into index.tsx)

## Files Unchanged
- `tabs/config-tab.tsx` — No changes
- `tabs/models-tab.tsx` — No changes
- `tabs/mcp-tab.tsx` — No changes
- `tabs/skills-tab.tsx` — No changes
- `components/client/HowToConnect.tsx` — No changes
- `components/wizard/ClientCreationWizard.tsx` — No changes

## Verification
1. `npm run dev` / `cargo tauri dev` — visual check of the new layout
2. Verify left panel: search filters clients, "+" opens creation wizard, clicking selects
3. Verify right panel: shows detail with all 4 tabs working (Config, Models, MCP, Skills)
4. Verify navigation: URL subtab updates on selection, deep links to specific tabs work
5. Verify actions: enable/disable toggle, delete, Try It Out, rotate credentials all work
6. Verify resizable handle works to resize panels
7. Verify empty state shows when no client is selected
