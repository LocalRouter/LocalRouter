# Plan: Move Strong/Weak Try It Out to Dedicated Tab

## Context

The Strong/Weak routing feature currently has inline "Try it out" sections embedded in both the global settings page (`settings/routellm-tab.tsx`) and the client/strategy model configuration (`StrategyModelConfiguration.tsx`). This makes the feature harder to discover and clutters configuration pages. Moving it to a dedicated tab in the Try It Out view centralizes testing, matches existing patterns (LLM tab, MCP tab), and lets users see confidence scores they can then apply in client configuration.

## Changes

### 1. Create `src/views/try-it-out/routellm-tab/index.tsx` (NEW)

New tab component following the pattern of `llm-tab/` and `mcp-tab/`. Contains:
- **Status banner**: RouteLLM state display with download button (reuses same status/download/event-listener pattern from `settings/routellm-tab.tsx`)
- **ThresholdSelector** with `showTryItOut={true}`: slider, presets, quick examples, custom input, test history
- **No client/direct mode selection** — just threshold + test

No props needed (standalone, no mode selection).

### 2. Modify `src/views/try-it-out/index.tsx`

- Import `RouteLLMTryItOutTab` from `./routellm-tab`
- Add third `TabsTrigger value="routellm"` labeled "Strong/Weak" after the MCP tab
- Add third `TabsContent` rendering `<RouteLLMTryItOutTab />`
- Update subtitle to mention Strong/Weak routing

### 3. Modify `src/components/routellm/ThresholdSelector.tsx`

- In test history items, add the raw confidence score as a number (e.g., `0.72`) alongside the existing percentage display
- Format: show `item.score.toFixed(2)` as a monospace number so users can copy the exact threshold value for their client configuration

### 4. Modify `src/views/settings/routellm-tab.tsx`

- Add `onTabChange` prop: `onTabChange?: (view: string, subTab?: string | null) => void`
- Replace the "Try it out" Card (lines 287-301) with a link button: `"Open in Try It Out"` that navigates to `try-it-out/routellm`
- Remove `testThreshold` state and `ThresholdSelector` import (no longer needed in this file)

### 5. Modify `src/views/settings/index.tsx`

- Pass `onTabChange` to `<RouteLLMTab onTabChange={onTabChange} />` (line 85)

### 6. Modify `src/components/strategy/StrategyModelConfiguration.tsx`

- Add optional `onTabChange` prop to `StrategyModelConfigurationProps`
- In the Weak Model card, remove `showTryItOut` from `<ThresholdSelector>` (keep slider + presets for configuration)
- Add a "Try It Out" link button below the ThresholdSelector that navigates to `try-it-out/routellm`

### 7. Thread `onTabChange` to StrategyModelConfiguration call sites

- **`src/views/resources/strategies-panel.tsx`** (line 351): pass `onTabChange` prop — the strategies panel receives it from `ResourcesView`
- **`src/views/clients/tabs/models-tab.tsx`** (line 287): pass `onViewChange` as `onTabChange` — already has `onViewChange` prop (currently unused/deprecated, un-deprecate it)

### 8. Verify call sites pass `onViewChange` to models-tab

- Check `src/views/clients/client-detail.tsx` passes `onViewChange` to `ClientModelsTab` (it already does based on the existing prop)

## Files to modify
- `src/views/try-it-out/routellm-tab/index.tsx` (NEW)
- `src/views/try-it-out/index.tsx`
- `src/components/routellm/ThresholdSelector.tsx`
- `src/views/settings/routellm-tab.tsx`
- `src/views/settings/index.tsx`
- `src/components/strategy/StrategyModelConfiguration.tsx`
- `src/views/resources/strategies-panel.tsx`
- `src/views/clients/tabs/models-tab.tsx`

## Verification

1. `npx tsc --noEmit` — types compile
2. `cargo tauri dev` — app runs, navigate to Try It Out > Strong/Weak tab
3. Test: download model if needed, adjust threshold, run test prompts, verify confidence score shows as number
4. Verify links from Settings > Strong/Weak and Client > Models navigate to the Try It Out tab
5. Verify inline try-it-out is removed from settings and strategy config
