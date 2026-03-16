# Hub-and-Spoke UI Refactor for Clients, LLMs, and MCPs

## Context

The Clients, LLM Providers, and MCP Servers pages currently use a split-panel layout (list on left, detail on right via `ResizablePanelGroup`). The Optimize page uses a different, better pattern: a full-screen card list ("hub") that links to dedicated sub-pages ("spokes"), with collapsible sidebar navigation showing sub-items.

**Goal**: Refactor all three pages to match the Optimize hub-and-spoke pattern.

## Key Assumptions

- **Sidebar**: Individual clients/providers/MCPs will appear as dynamic children in collapsible sidebar groups (like Optimize's GuardRails, Secret Scanning, etc.)
- **LLMs tabs**: The Providers/All Models/Compatibility tabs stay on the overview. Only the Providers tab changes from split-panel to card list. Models and Compatibility remain as-is since they're catalog/table views.
- **Client Settings tab**: Stays on the overview page alongside the client list tab.
- **Sidebar item limit**: Show max ~10 items in sidebar with "Show all (N)..." link to prevent overflow.

---

## Implementation

### Phase 1: Sidebar Dynamic Children

**`src/components/layout/sidebar.tsx`**

1. Add new type for dynamic children:
   ```typescript
   interface NavDynamicChild {
     id: string        // entity's unique ID (client_id, instance_name, server id)
     label: string     // display name
     icon?: React.ElementType
   }

   interface NavDynamicCollapsible {
     id: View
     icon: React.ElementType
     label: string
     shortcut?: string
     dynamicChildren: NavDynamicChild[]
   }
   ```

2. Extend `SidebarProps`:
   - Add `activeSubTab: string | null` (needed to highlight selected dynamic child)
   - Change `onViewChange` signature to `(view: View, subTab?: string | null) => void`
   - Add `dynamicGroups?: { clients, providers, mcpServers }` data prop

3. Convert `clients`, `resources`, `mcp-servers` from flat `NavItem`s to `NavDynamicCollapsible`s

4. Add `renderNavDynamicCollapsible()`:
   - Clicking parent → `onViewChange(group.id)` (shows overview)
   - Clicking child → `onViewChange(group.id, child.id)` (shows detail)
   - Active child highlighted when `activeView === group.id && activeSubTab` starts with `child.id`
   - Collapsed sidebar: only parent icon shown (same as Optimize)
   - Expanded sidebar: parent + indented children with truncation
   - Cap displayed items at ~10, show "Show all" link

5. Update auto-expand effect to handle dynamic collapsibles (check `activeSubTab` against children)

**`src/components/layout/app-shell.tsx`**

- Pass `activeSubTab` to Sidebar (already available as prop)
- Transform existing `clients`, `providers`, `mcpServers` state into `NavDynamicChild[]` arrays
- Pass as `dynamicGroups` prop to Sidebar

### Phase 2: Clients Hub-and-Spoke

**`src/views/clients/index.tsx`**

Replace `ResizablePanelGroup` with conditional rendering:

```
if (selectedClientId && selectedClient) {
  // SPOKE: Full-screen ClientDetail with "Back" button
} else {
  // HUB: Full-screen scrollable card list
}
```

Hub view: Scrollable card grid (like Optimize overview cards), each card shows:
- Client name, truncated client_id
- Enabled/disabled badge
- "View" button → navigates to detail

Spoke view: Existing `ClientDetail` component rendered full-screen with an ArrowLeft "Back to Clients" button at the top.

Keep the Client/Settings tabs at the top of the page.

### Phase 3: LLM Providers Hub-and-Spoke

**`src/views/resources/providers-panel.tsx`** (1769 lines)

Replace `ResizablePanelGroup` with conditional rendering:

```
if (selectedId && selectedProvider) {
  // SPOKE: Full-screen provider detail with "Back" button
} else {
  // HUB: Card grid of all providers
}
```

Hub cards show: ProviderIcon, instance name, provider type, health dot + latency, enabled/disabled.

Spoke: Existing detail panel content (tabs: Info, Try It Out, Models, Free Tier, Settings) rendered full-screen.

Keep all dialog state (create/delete dialogs) in same component - they're modals that work in both modes.

**`src/views/resources/index.tsx`** - Minor: Keep Providers/Models/Compatibility tabs. No structural change.

### Phase 4: MCP Servers Hub-and-Spoke

**`src/views/resources/mcp-servers-panel.tsx`** (1368 lines)

Same pattern as providers:

```
if (selectedId && selectedServer) {
  // SPOKE: Full-screen server detail with "Back" button
} else {
  // HUB: Card grid of all MCP servers
}
```

Hub cards show: McpServerIcon, name, transport type, health dot + latency, OAuth status.

**`src/views/mcp-servers/index.tsx`** - Minor wrapper adjustments.

### Phase 5: Sidebar Responsive Behavior

Ensure dynamic children in the sidebar have the same responsive behavior as Optimize sub-items:
- Text truncation via `truncate` class
- Proper indentation matching existing `ml-3 border-l pl-1` pattern
- Collapsed sidebar: only parent icon visible
- Auto-expand when a child is active

---

## Implementation Order

1. Phase 1 (Sidebar) - Foundation, everything depends on this
2. Phase 2 (Clients) - Simplest, good validation of the pattern
3. Phase 4 (MCP Servers) - Medium complexity
4. Phase 3 (LLM Providers) - Most complex (1769 line file)
5. Phase 5 (Responsive) - Final polish

## Files Modified

| File | Change Size |
|------|------------|
| `src/components/layout/sidebar.tsx` | Major |
| `src/components/layout/app-shell.tsx` | Minor |
| `src/views/clients/index.tsx` | Major |
| `src/views/resources/providers-panel.tsx` | Major |
| `src/views/resources/mcp-servers-panel.tsx` | Major |
| `src/views/resources/index.tsx` | Minor |
| `src/views/mcp-servers/index.tsx` | Minor |

## Verification

1. `npx tsc --noEmit` - Type check passes
2. Visual check: Sidebar shows collapsible groups with dynamic children for all three sections
3. Visual check: Overview pages show full-screen card lists
4. Navigation: Click card → detail sub-page renders full-screen
5. Navigation: Back button → returns to overview
6. Navigation: Sidebar child click → correct detail page
7. Sidebar collapse: Dynamic groups collapse to parent icon only
8. Command palette: Existing navigation to clients/providers/MCPs still works
9. Tray menu events (`open-mcp-server`, `open-client-tab`, etc.) still navigate correctly
