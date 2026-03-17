# Add Models Tab to LLM Providers View

## Context
The LLM Providers page currently only has a "Providers" tab. A `ModelsPanel` component already exists at `src/views/resources/models-panel.tsx` with full functionality (search, filter, list+detail layout, pricing display) but was never wired into the view. We just need to connect it.

## Changes

### 1. `src/views/resources/index.tsx`
- Import `ModelsPanel` from `./models-panel`
- Add `<TabsTrigger value="models">Models</TabsTrigger>` after the Providers trigger
- Add `<TabsContent value="models">` with `<ModelsPanel>` passing `selectedId` and `onSelect` using the existing `handleItemSelect` pattern
- Update `parseSubTab` comments to include "models" format

### Verification
- Run `npx tsc --noEmit` to verify types
- `cargo tauri dev` and navigate to LLM Providers → Models tab
- Confirm model list loads, search/filter works, detail panel shows pricing and capabilities
