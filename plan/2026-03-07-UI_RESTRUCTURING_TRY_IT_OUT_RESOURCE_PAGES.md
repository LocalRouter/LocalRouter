# UI Restructuring: Try It Out & Resource Pages

## Context

Currently, each resource view (Clients, Providers, MCP Servers, Skills, Guardrails) has a top-level "Try It Out" tab with mode selectors (Against Client / All / Direct / Specific). The user wants to:
- Move Try It Out into individual resource detail views (forced to that specific resource)
- Keep a global Try It Out tab at each view level (forced to "All" mode)
- Refactor Guardrails into a list/detail layout
- Revamp Strong/Weak to always show 3 tabs regardless of download state
- Add a Settings tab to the Client list page

## Requirements Checklist

1. **Client Settings Tab**: Add "Settings" top-level tab to Client list page with Server Status, MCP Client Connection, Network Settings (no Resource Limits)
2. **Client Try It Out**: Remove global Try It Out from Client list page; add it inside ClientDetail after Connect tab (LLM + MCP sub-tabs, forced to this client, no client selector)
3. **Provider Try It Out**: Add Try It Out tab to individual provider detail (after Info), forced to this provider. Global Resources Try It Out forced to "All models" mode
4. **Guardrails List/Detail**: Refactor into left-pane list (with search + add) / right-pane detail like Clients/Providers. Individual model gets Try It Out tab (specific model). Global Try It Out forced to "All models"
5. **Strong/Weak Revamp**: Always 3 tabs (Model, Try It Out, Settings) regardless of download state. Model tab: resource requirements + download status/button + Open Folder. Try It Out: ThresholdSelector (disabled if not downloaded). Settings: Memory Management only
6. **MCP Servers Try It Out**: Add to individual server detail, forced to specific server, no mode selector, deferred loading toggle added. Global forced to "All MCPs & Skills"
7. **Skills Try It Out**: Add to individual skill detail, forced to specific skill, deferred loading toggle (default off). Global forced to "All MCPs & Skills"
8. **Coding Agents**: No changes

---

## Implementation Plan

### Phase 1: Component Props (foundation for all other changes)

#### 1A. `LlmTab` - Add "all" mode and selector hiding
**File**: `src/views/try-it-out/llm-tab/index.tsx`

- Add `"all"` to `TestMode` type (alongside `"client"` and `"direct"`)
- Add prop `hideProviderSelector?: boolean` — hides provider dropdown when in "direct" mode (for embedding in provider detail)
- In "all" mode: use internal test token, fetch ALL provider models (no provider filter), show a single combined model dropdown
- When `hideModeSwitcher` is true, don't render mode radio buttons
- When `hideProviderSelector` is true and mode is "direct", don't render provider dropdown (use `initialProvider` value)

#### 1B. `McpTab` - Add deferred loading in direct mode + hide target selector
**File**: `src/views/try-it-out/mcp-tab/index.tsx`

- Add prop `hideDirectTargetSelector?: boolean` — hides target dropdown in "direct" mode
- Add prop `showDeferredInDirect?: boolean` — shows deferred loading toggle in "direct" mode
- Update deferred loading toggle visibility: show when `mode === "client"` or `mode === "all"` or `(mode === "direct" && showDeferredInDirect)`
- Update `connectionConfig` to pass deferred loading value in direct mode when `showDeferredInDirect`

#### 1C. `GuardrailsTab` - Add forced mode and model pre-selection
**File**: `src/views/try-it-out/guardrails-tab/index.tsx`

- Add props: `forcedMode?: TestMode`, `hideModeSwitcher?: boolean`, `forcedModelId?: string`
- When `forcedMode` set, use as initial mode and don't allow changing
- When `hideModeSwitcher`, hide mode radio group
- When `forcedModelId` set and mode is "specific_model", pre-select that model and hide model dropdown

#### 1D. `ServerTab` - Add `hideResourceLimits` prop
**File**: `src/views/settings/server-tab.tsx`

- Add prop `hideResourceLimits?: boolean`
- When true, don't render the Resource Limits card

#### 1E. `ThresholdSelector` - Add `disabled` prop
**File**: `src/components/routellm/ThresholdSelector.tsx`

- Add `disabled?: boolean` prop
- Disable slider, preset buttons, test input/buttons when true

---

### Phase 2: Client Changes (R1, R2)

#### 2A. Client list page - Remove Try It Out, add Settings
**File**: `src/views/clients/index.tsx`

- Remove the "Try It Out" top-level tab and all its related state/parsing
- Add "Settings" top-level tab that renders `<ServerTab hideResourceLimits />`
- Top-level tabs become: "Client" | "Settings"

#### 2B. Client detail - Add Try It Out tab
**File**: `src/views/clients/client-detail.tsx`

- Add "Try It Out" tab trigger after "Connect" (visible unless `client_mode === "mcp_only"` for LLM sub-tab)
- Tab content: two inner sub-tabs "LLM Provider" and "MCP"
  - LLM: `<LlmTab initialMode="client" initialClientId={client.client_id} hideModeSwitcher hideProviderSelector />`
  - MCP: `<McpTab initialMode="client" initialClientId={client.client_id} hideModeSwitcher />`
- No client selection dropdowns shown

---

### Phase 3: Provider Changes (R3)

#### 3A. Provider detail - Add Try It Out tab
**File**: `src/views/resources/providers-panel.tsx`

- Add "Try It Out" tab trigger after "Info" in provider detail tabs
- Tab content: `<LlmTab initialMode="direct" initialProvider={selectedProvider.instance_name} hideModeSwitcher hideProviderSelector />`
- Remove the existing "Try It Out" button from provider detail header (or change it to switch to the new tab)

#### 3B. Resources global Try It Out - Force "All" mode
**File**: `src/views/resources/index.tsx`

- Change `<LlmTab>` to use `initialMode="all"` and `hideModeSwitcher`
- Remove `tryItOutInitProps` spread (no longer needed since provider-specific testing is now in provider detail)

---

### Phase 4: Guardrails Refactoring (R4)

#### 4A. Create GuardrailsPanel component
**New file**: `src/views/guardrails/guardrails-panel.tsx`

- `ResizablePanelGroup` with list (35%) and detail (65%)
- **List panel**: Search input + "+" button (opens `SafetyModelPicker` in dialog), scrollable model list with name/provider/status
- **Detail panel** (when model selected): Inner tabs "Info" | "Try It Out"
  - Info: model configuration details (provider, model name, type, thresholds, categories)
  - Try It Out: `<GuardrailsTab forcedMode="specific_model" forcedModelId={selectedModel.id} hideModeSwitcher />`

#### 4B. Rewrite GuardrailsView
**File**: `src/views/guardrails/index.tsx`

- Keep header (title, EXPERIMENTAL badge, description)
- Top-level tabs: "Models" | "Try It Out" | "Settings"
  - Models: render `<GuardrailsPanel>` (list/detail)
  - Try It Out: `<GuardrailsTab forcedMode="all_models" hideModeSwitcher />`
  - Settings: Parallel Scanning toggle (unchanged)
- Reuse existing config loading, event listeners, model add/remove logic

---

### Phase 5: Strong/Weak Revamp (R5)

#### 5A. Restructure Strong/Weak view
**File**: `src/views/strong-weak/index.tsx`

- Always show 3 tabs: "Model" | "Try It Out" | "Settings" (regardless of download state)
- **Model tab**:
  - Status badge (Not Downloaded / Downloading / Model unloaded / Loading / Model loaded)
  - Download button (when not downloaded and not downloading)
  - Open Folder button
  - Download progress card (when downloading)
  - Resource Requirements card (moved FROM Settings): Cold Start, Disk Space, Latency, Memory
- **Try It Out tab**:
  - `<ThresholdSelector disabled={!isReady} showTryItOut />`
  - Info message when not downloaded: "Download the model first to use Try It Out"
- **Settings tab**:
  - Memory Management card only (Auto-Unload After Idle selector + Save)
  - Remove Resource Requirements from here
- Default tab: "model" instead of "try-it-out"

---

### Phase 6: MCP Servers & Skills (R6, R7)

#### 6A. MCP Server detail - Add Try It Out tab
**File**: `src/views/resources/mcp-servers-panel.tsx`

- Add "Try It Out" tab trigger in server detail tabs (only when server is enabled)
- Tab content:
  ```
  <McpTab
    initialMode="direct"
    initialDirectTarget={`server:${selectedServer.id}`}
    hideModeSwitcher
    hideDirectTargetSelector
    showDeferredInDirect
  />
  ```
- Change existing "Try It Out" header button to switch to this local tab

#### 6B. MCP Servers global Try It Out - Force "All" mode
**File**: `src/views/mcp-servers/index.tsx`

- Change `<McpTab>` to use `initialMode="all"` and `hideModeSwitcher`
- Remove `tryItOutInitProps` spread

#### 6C. Skill detail - Add Try It Out tab
**File**: `src/views/skills/index.tsx`

- Add "Try It Out" tab trigger in skill detail tabs (only when skill is enabled)
- Tab content:
  ```
  <McpTab
    initialMode="direct"
    initialDirectTarget={`skill:${selectedSkill.name}`}
    hideModeSwitcher
    hideDirectTargetSelector
    showDeferredInDirect
  />
  ```
- Change existing "Try It Out" header button to switch to this local tab

#### 6D. Skills global Try It Out - Force "All" mode
**File**: `src/views/skills/index.tsx`

- Change global `<McpTab>` to use `initialMode="all"` and `hideModeSwitcher`

---

### Phase 7: Cleanup

- Update navigation links that target old try-it-out paths (e.g., `clients/tabs/guardrails-tab.tsx` links to `guardrails/try-it-out/init/client/...`)
- Update `website/src/components/demo/TauriMockSetup.ts` mock data if needed for guardrails list/detail

---

## Files Summary

| File | Action | Phase |
|------|--------|-------|
| `src/views/try-it-out/llm-tab/index.tsx` | Add "all" mode, `hideProviderSelector` | 1A |
| `src/views/try-it-out/mcp-tab/index.tsx` | Add `hideDirectTargetSelector`, `showDeferredInDirect` | 1B |
| `src/views/try-it-out/guardrails-tab/index.tsx` | Add `forcedMode`, `hideModeSwitcher`, `forcedModelId` | 1C |
| `src/views/settings/server-tab.tsx` | Add `hideResourceLimits` | 1D |
| `src/components/routellm/ThresholdSelector.tsx` | Add `disabled` | 1E |
| `src/views/clients/index.tsx` | Remove Try It Out tab, add Settings tab | 2A |
| `src/views/clients/client-detail.tsx` | Add Try It Out tab with LLM + MCP | 2B |
| `src/views/resources/providers-panel.tsx` | Add Try It Out tab to provider detail | 3A |
| `src/views/resources/index.tsx` | Force "all" mode for global Try It Out | 3B |
| `src/views/guardrails/guardrails-panel.tsx` | **NEW** - List/detail panel | 4A |
| `src/views/guardrails/index.tsx` | Rewrite with list/detail layout | 4B |
| `src/views/strong-weak/index.tsx` | Restructure to always-3-tabs layout | 5A |
| `src/views/resources/mcp-servers-panel.tsx` | Add Try It Out tab to server detail | 6A |
| `src/views/mcp-servers/index.tsx` | Force "all" mode for global Try It Out | 6B |
| `src/views/skills/index.tsx` | Add Try It Out to skill detail + force "all" global | 6C/6D |

---

## Verification

1. **Client Settings tab**: Open Clients view → "Settings" tab shows Server Status, MCP Connection, Network (no Resource Limits)
2. **Client Try It Out**: Select a client → "Try It Out" tab after Connect → LLM and MCP sub-tabs with no client selector
3. **Provider Try It Out**: Select a provider → "Try It Out" tab after Info → shows models for that provider only
4. **Resources global Try It Out**: Resources → Try It Out tab → shows all models, no mode selector
5. **Guardrails list/detail**: Guardrails sidebar → left list with search/add, right detail with Info + Try It Out tabs
6. **Guardrails global Try It Out**: Guardrails → Try It Out tab → "All Models" mode, no mode selector
7. **Strong/Weak**: Always shows Model/Try It Out/Settings tabs. Model tab has download + resource requirements. Try It Out disabled when not downloaded. Settings has Memory Management only.
8. **MCP Server detail**: Select server → Try It Out tab → specific server mode, deferred loading toggle available
9. **MCP Servers global**: MCP Servers → Try It Out → "All MCPs & Skills" mode
10. **Skill detail**: Select skill → Try It Out tab → specific skill mode, deferred loading toggle (default off)
11. **Skills global**: Skills → Try It Out → "All MCPs & Skills" mode
12. **Coding Agents**: Unchanged
13. Run `npx tsc --noEmit` to verify types compile
