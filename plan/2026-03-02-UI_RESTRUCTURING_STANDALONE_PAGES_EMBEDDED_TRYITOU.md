# UI Restructuring: Standalone Pages, Embedded Try-It-Out, Slimmer Settings

## Context

The current UI bundles Coding Agents, Guardrails, and Strong/Weak configuration into the Settings page, with a separate standalone "Try It Out" page for testing. This creates a fragmented experience -- users must jump between Settings and Try It Out to configure and test features. This restructuring promotes these features to standalone pages with their own settings and embedded try-it-out tabs, while organizing the sidebar to reflect logical groupings (Skill and Coding Agents under MCP).

## New Sidebar Structure

```
Dashboard          (⌘1)
Client             (⌘2)
─────────────
LLM Provider       (⌘3)
MCP                (⌘4)
  Skill            (⌘5)  ← indented
  Coding Agents    (⌘6)  ← indented
─────────────
GuardRails         (⌘7)
Strong/Weak        (⌘8)
Marketplace        (⌘9)
─────────────
Settings
Debug              (dev only)
```

---

## Phase 1: Sidebar + Routing Infrastructure

**Goal**: Update sidebar hierarchy and App.tsx routing to support new views.

### Files to modify:
- `src/components/layout/sidebar.tsx` — Add `indent` support to NavItem, reorganize groups, update keyboard shortcuts, add new view IDs (`coding-agents`, `guardrails`, `strong-weak`), remove `try-it-out`
- `src/App.tsx` — Add imports/cases for new views, remove `try-it-out` case (add placeholder components initially)

### Sidebar changes:
- Add `indent?: boolean` to `NavItem` interface, make `shortcut` optional
- When `indent` is true, add extra left padding (`pl-6` or similar) to the nav button
- Four nav groups with separators: `topNavItems` (Client), `resourceNavItems` (LLM Provider, MCP, Skill indented, Coding Agents indented), `featureNavItems` (GuardRails, Strong/Weak, Marketplace), `bottomNavItems` (Settings, Debug)
- Update keyboard shortcut handler to new numbering
- Remove `FlaskConical` import (Try It Out icon)

### View type update:
```typescript
export type View = 'dashboard' | 'clients' | 'resources' | 'mcp-servers' | 'skills'
  | 'coding-agents' | 'guardrails' | 'strong-weak' | 'marketplace' | 'settings' | 'debug'
```

---

## Phase 2: GuardRails Standalone Page

**Goal**: Create a standalone GuardRails page combining model management, settings, and try-it-out.

### Files to create:
- `src/views/guardrails/index.tsx`

### Existing components to reuse:
- `SafetyModelList` and `SafetyModelPicker` from `src/components/guardrails/`
- Try-it-out content from `src/views/try-it-out/guardrails-tab/index.tsx`
- Settings content from `src/views/settings/guardrails-tab.tsx`

### Page structure:
Two-panel layout (like MCP servers page):
- **Left panel**: Safety model list with "+" add button, click to select
- **Right panel with tabs**:
  - **Models** (default) — Selected model detail, SafetyModelList management
  - **Try It Out** — Embedded guardrails testing (mode selector, quick tests, results)
  - **Settings** — Parallel scanning toggle, scan_requests toggle (moved from global settings)

---

## Phase 3: Strong/Weak Standalone Page

**Goal**: Create a standalone Strong/Weak page combining RouteLLM config and try-it-out.

### Files to create:
- `src/views/strong-weak/index.tsx`

### Existing components to reuse:
- `ThresholdSelector` from `src/components/routellm/`
- Settings from `src/views/settings/routellm-tab.tsx`
- Try-it-out from `src/views/try-it-out/routellm-tab/index.tsx`

### Page structure:
Single page (no sidebar list — there's only one RouteLLM instance):
- **Status/download section** — Model status, download button, progress bar
- **Tabs** (once model is available):
  - **Try It Out** — ThresholdSelector with test functionality
  - **Settings** — Auto-unload timeout, resource info (from current RouteLLMTab in settings)

---

## Phase 4: Coding Agents Standalone Page

**Goal**: Un-deprecate and enhance the coding agents page with tabs for agents, sessions, try-it-out, and settings.

### Files to modify:
- `src/views/coding-agents/index.tsx` — Major rewrite, remove `@deprecated`

### Files to create:
- `src/views/coding-agents/agents-tab.tsx` — Agent list + detail (extracted from current view)
- `src/views/coding-agents/sessions-tab.tsx` — Session monitoring
- `src/views/coding-agents/try-it-out-tab.tsx` — MCP try-it-out for coding agent tools
- `src/views/coding-agents/settings-tab.tsx` — Concurrency + agent config (from settings)

### Existing components to reuse:
- Current agent list/detail UI from `src/views/coding-agents/index.tsx`
- Settings from `src/views/settings/coding-agents-tab.tsx`
- `McpTab` from `src/views/try-it-out/mcp-tab/` (for the try-it-out tab)

### Tab structure:
- **Agents** (default) — Two-panel: agent list (left) + detail with install status (right)
- **Sessions** — List of active/recent sessions with status badges, display text, working directory. Click for detail (recent output, pending questions). "End Session" button per session.
- **Try It Out** — Embedded `McpTab` showing coding agent MCP tools
- **Settings** — Max concurrent sessions, agent detection list (moved from global settings)

### Backend changes needed for Sessions tab:
- `src-tauri/src/ui/commands_coding_agents.rs` — Add `get_coding_session_detail` command that returns recent output, pending questions, cost/turn info for a session
- `crates/lr-coding-agents/src/manager.rs` — Add method to expose session detail (output buffer, pending question, cost/turns)
- `src/types/tauri-commands.ts` — Add `CodingSessionDetail` type
- Consider emitting `coding-session-updated` event for real-time monitoring

---

## Phase 5: Embed Try-It-Out in Existing Pages

**Goal**: Add try-it-out tabs to MCP, Skills, and LLM Provider pages.

### MCP Servers page (`src/views/mcp-servers/index.tsx`):
- Wrap existing content in a tab layout: **Servers** (default) | **Try It Out**
- Try It Out tab embeds `McpTab` from `src/views/try-it-out/mcp-tab/`

### Skills page (`src/views/skills/index.tsx`):
- Add a **Try It Out** tab to existing skill detail
- Embeds `McpTab` pre-configured in "direct" mode for the selected skill

### LLM Provider page (`src/views/resources/index.tsx`):
- Add a **Try It Out** tab alongside "Providers" and "All Models"
- Embeds `LlmTab` from `src/views/try-it-out/llm-tab/`

---

## Phase 6: Update Client Detail Navigation

**Goal**: Update the "Try It Out" dropdown in client detail to navigate to embedded try-it-out tabs.

### File to modify:
- `src/views/clients/client-detail.tsx`

### Changes:
Update the dropdown navigation targets:
- "LLM" → `onViewChange("resources", "try-it-out/init/client/${client.client_id}")`
- "MCP & Skills" → `onViewChange("mcp-servers", "try-it-out/init/client/${client.client_id}")`
- "GuardRails" → `onViewChange("guardrails", "try-it-out/init/client/${client.client_id}")`

Each target page will need to handle init path parsing in its try-it-out tab. Extract `parseInitPath` from `src/views/try-it-out/index.tsx` into a shared utility (`src/utils/navigation.ts` or similar).

---

## Phase 7: Slim Down Settings + Delete Try It Out

**Goal**: Remove migrated tabs from Settings, delete the standalone Try It Out page.

### Settings (`src/views/settings/index.tsx`):
- Remove tabs: `coding-agents`, `guardrails`, `routellm`
- Remaining tabs: **Server** | **Appearance** | **Logs** | **Updates**
- Remove imports: `CodingAgentsSettingsTab`, `GuardrailsTab`, `RouteLLMTab`

### Delete/deprecate:
- `src/views/try-it-out/index.tsx` — Delete main view
- `src/views/try-it-out/routellm-tab/` — Delete (merged into strong-weak page)
- `src/views/try-it-out/guardrails-tab/` — Delete (merged into guardrails page)
- Keep `src/views/try-it-out/llm-tab/` and `src/views/try-it-out/mcp-tab/` as shared components (or move to `src/components/try-it-out/`)

### App.tsx:
- Remove `TryItOutView` import and switch case
- Add event listeners: `open-coding-agents-page`, `open-guardrails-page` if needed

### Demo mock (`website/src/components/demo/TauriMockSetup.ts`):
- Add mock handlers for any new Tauri commands
- Remove try-it-out navigation if present

---

## Phase 8: Polish

- Move `src/views/try-it-out/llm-tab/` and `src/views/try-it-out/mcp-tab/` to `src/components/try-it-out/` since they're now shared components
- Update any remaining cross-references to the old try-it-out page
- Update website docs if they reference the Try It Out page
- Run `npx tsc --noEmit` to verify types

---

## Verification

1. `npx tsc --noEmit` — TypeScript compilation
2. `cargo test && cargo clippy` — Backend (if backend changes in Phase 4)
3. Manual testing:
   - Sidebar: all items navigate correctly, indentation renders, keyboard shortcuts work
   - GuardRails page: model list, add/remove models, try-it-out testing, settings persist
   - Strong/Weak page: download flow, threshold testing, settings
   - Coding Agents page: agent list, session monitoring, try-it-out tab, settings
   - MCP/Skills/LLM Provider pages: try-it-out tabs work
   - Client detail dropdown navigates to embedded try-it-out
   - Settings page only shows Server, Appearance, Logs, Updates
   - Try It Out sidebar item is gone
