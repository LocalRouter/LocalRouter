# Plan: Add Sample Popup Buttons to Relevant Pages

## Context

The debug page (`src/views/debug/index.tsx`) has 5 firewall approval popup types with a "send multiple" checkbox. We want to distribute each popup type as a single "Sample Popup" button (with FlaskConical lab icon) onto its relevant page, so users can preview what the firewall approval popup looks like without navigating to the debug page.

## Popup Type → Page Mapping

| Popup Type | Page | Location on Page |
|---|---|---|
| `mcp_tool` | **MCP Servers** | Page header, next to subtitle |
| `llm_model` | **LLM Providers** (Resources) | Page header, next to subtitle |
| `skill` | **Skills** | Page header, next to subtitle |
| `marketplace` | **Marketplace** | Page header, next to subtitle |
| `free_tier_fallback` | **Clients → Models tab → Free-Tier Mode card** | Inside the Free-Tier Mode card, when free-tier is enabled |
| *(Future)* Auto Router | **Strong/Weak** | TBD — user will add this later |

## Implementation

### 1. Create shared `SamplePopupButton` component

**New file:** `src/components/shared/SamplePopupButton.tsx`

```tsx
// Reusable button that triggers a single firewall approval popup
// Props: popupType (one of the 5 types), optional size/variant overrides
// Calls: invoke("debug_trigger_firewall_popup", { popupType, sendMultiple: false })
// Icon: FlaskConical, Label: "Sample Popup"
```

### 2. Add button to each page

#### `src/views/mcp-servers/index.tsx` — MCP Tool popup
- Add `SamplePopupButton` with `popupType="mcp_tool"` in the page header area (next to the subtitle `<p>` tag)

#### `src/views/resources/index.tsx` — LLM Model popup
- Add `SamplePopupButton` with `popupType="llm_model"` in the page header area

#### `src/views/skills/index.tsx` — Skill popup
- Add `SamplePopupButton` with `popupType="skill"` in the page header area

#### `src/views/marketplace/index.tsx` — Marketplace popup
- Add `SamplePopupButton` with `popupType="marketplace"` in the page header area

#### `src/views/clients/tabs/models-tab.tsx` — Free-Tier Fallback popup
- Add `SamplePopupButton` with `popupType="free_tier_fallback"` inside the Free-Tier Mode card (`~line 348`), visible only when `free_tier_only` is enabled
- Place it near the "Paid Fallback" section as a contextual demo

#### *(Future)* `src/views/strong-weak/index.tsx` — Auto Router popup
- Will be added once the Auto Router popup type is implemented

### Button Style
- `variant="outline"`, `size="sm"`
- Icon: `FlaskConical` (lucide-react)
- Label: "Sample Popup"
- Consistent with existing FlaskConical "Try It Out" button pattern used across the app

## Files Modified
1. `src/components/shared/SamplePopupButton.tsx` (new)
2. `src/views/mcp-servers/index.tsx`
3. `src/views/resources/index.tsx`
4. `src/views/skills/index.tsx`
5. `src/views/marketplace/index.tsx`
6. `src/views/clients/tabs/models-tab.tsx`

## Verification
1. Run `cargo tauri dev`
2. Navigate to each page (MCP Servers, LLM Providers, Skills, Marketplace) and verify the "Sample Popup" button is visible in the header
3. Click each button → verify a single firewall approval popup opens with the correct type
4. Navigate to Clients → select a client → Models tab → enable Free-Tier Mode → verify the "Sample Popup" button appears
5. Run `npx tsc --noEmit` to verify types
