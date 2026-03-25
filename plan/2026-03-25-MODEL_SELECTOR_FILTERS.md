# Model Selector Filters

**Date**: 2026-03-25
**Status**: Implemented

## Summary

Added filtering and grouping controls to the disabled models section of the ThreeZoneModelSelector component used in client model configuration.

## Changes

### New Filters
1. **Group by provider toggle** - Independent toggle (decoupled from sort option) to show models grouped by provider with collapsible headers, or as a flat list
2. **Free tier only** - Toggle to filter models to only show those from providers with free tier access (kind != 'none')
3. **Capability filters** - Toggle chips for Vision, Function Calling, and Embedding capabilities
4. **Context window sorting** - New sort options: Context Large→Small and Context Small→Large

### Not Implemented
- **Sort by most recent** - No date/recency data available in the model catalog

### Files Modified
- `src/components/strategy/ThreeZoneModelSelector.tsx` - Filter UI, state, and logic
- `src/views/clients/tabs/unified-models-tab.tsx` - Pass capabilities and context window data as new props
- `website/src/components/demo/TauriMockSetup.ts` - Enhanced demo mock capabilities per model

### Data Flow
- `unified-models-tab.tsx` extracts `capabilities` and `context_window` from `list_all_models_detailed` response
- Passes `modelCapabilities: Record<string, string[]>` and `modelContextWindows: Record<string, number>` to ThreeZoneModelSelector
- ThreeZoneModelSelector uses these for filtering and sorting

### UI Design
- Collapsible filter row behind a "Filters" button with active count badge
- Filter chips are pill-shaped toggle buttons
- Group by provider and free tier toggles have distinct color schemes (primary / green)
- Capability filters grouped after a visual divider
- "Clear" link to reset all filters at once
