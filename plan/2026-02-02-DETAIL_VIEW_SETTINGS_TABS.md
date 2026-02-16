# Plan: Multi-tab detail views with Settings tabs

## Overview

Restructure all four detail views (Clients, Providers, MCP Servers, Skills) to use a consistent multi-tab pattern: an **info tab** first, then a **Settings tab** at the end. Move enable/disable, edit, and delete into Settings. Remove the three-dot menu and header toggle.

---

## 1. Client Detail View

**File:** `src/views/clients/client-detail.tsx`

**Current tabs:** Config | Models | MCP | Skills
**New tabs:** Connect | Models | MCP | Skills | Settings

Changes:
- Rename "Config" tab → "Connect" and remove the name editing Card from `config-tab.tsx` (keep only HowToConnect credentials)
- Remove the enable/disable `Switch` and three-dot `DropdownMenu` from the header (lines 180-204)
- Remove the `Badge` from the header
- Add new "Settings" tab at the end
- Move the delete AlertDialog into the settings tab

**New file:** `src/views/clients/tabs/settings-tab.tsx`
- Props: `client`, `onUpdate`, `onDelete`
- Sections: Client Name card (moved from config-tab), Enable/Disable card with Switch, Danger Zone card with Delete button + inline AlertDialog

**Modified file:** `src/views/clients/tabs/config-tab.tsx`
- Remove the "Client Name" Card section (lines ~117-140)
- Remove name state/handler (`handleNameChange`, debounce logic) — move to settings-tab

---

## 2. Providers Panel

**File:** `src/views/resources/providers-panel.tsx`

**Current:** Single detail view with header badge + EntityActions three-dot menu
**New tabs:** Info | Settings

Changes in detail panel (lines ~380-565):
- Remove `EntityActions` component and `Badge` from header
- Wrap existing content (Health Status card, Models list card) in an "Info" tab
- Add "Settings" tab with: Inline edit form (from existing edit dialog), Enable/Disable switch, Danger Zone with Delete + confirmation
- Remove the separate edit dialog — embed the provider config form directly in Settings tab
- Keep "Try It Out" button in the header (it stays outside tabs)

---

## 3. MCP Servers Panel

**File:** `src/views/resources/mcp-servers-panel.tsx`

**Current:** Single detail view with header badge + EntityActions three-dot menu
**New tabs:** Info | Settings

Changes in detail panel (lines ~700-990):
- Remove `EntityActions` component and `Badge` from header
- Wrap existing content (Health Status, Connection Details, Transport Config, OAuth Status cards) in an "Info" tab
- Add "Settings" tab with: Inline edit form (from existing edit modal), Enable/Disable switch, Danger Zone with Delete + confirmation
- Remove the separate edit modal — embed the server config form directly in Settings tab
- Keep "Try It Out" button in the header

---

## 4. Skills View

**File:** `src/views/skills/index.tsx`

**Current:** Single detail view with inline Switch toggle
**New tabs:** Info | Settings

Changes in detail panel (lines ~234-493):
- Remove the `Switch` toggle and enabled/disabled text from header
- Wrap existing content (Details card, Files card, Source path) in an "Info" tab
- Add "Settings" tab with: Enable/Disable switch only
- No edit/delete for skills (they're filesystem-discovered)
- Keep "Try It Out" button in the header

---

## Files to create
- `src/views/clients/tabs/settings-tab.tsx`

## Files to modify
- `src/views/clients/client-detail.tsx` — restructure tabs, remove header controls
- `src/views/clients/tabs/config-tab.tsx` — remove name editing section
- `src/views/resources/providers-panel.tsx` — add tabs, inline edit in Settings, remove EntityActions
- `src/views/resources/mcp-servers-panel.tsx` — add tabs, inline edit in Settings, remove EntityActions
- `src/views/skills/index.tsx` — add tabs, move toggle to Settings

## Verification
- Run `npm run dev` / `cargo tauri dev` and check each detail view
- Client: Verify "Connect" tab shows credentials, "Settings" shows name + enable + delete
- Provider: Verify "Info" tab shows health + models, "Settings" shows inline edit form + enable + delete
- MCP Server: Verify "Info" tab shows health + connection + transport, "Settings" shows inline edit form + enable + delete
- Skills: Verify "Info" tab shows details + files, "Settings" shows enable switch
- Verify three-dot menus and header toggles are fully removed
