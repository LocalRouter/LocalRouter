# Offline Model Warning Feature

**Date:** 2026-01-20
**Status:** Planned (Not Started)
**Depends On:** Router Refactor (2026-01-19-ROUTER-REFACTOR-PROGRESS.md)

## Overview

Add a UI warning in the routing strategy configuration component that alerts users when their prioritized model list doesn't include any offline/local models. This helps users understand that internet connectivity issues will interrupt their workflow if they only rely on cloud-based models.

## Problem Statement

When users configure an auto-routing strategy with only cloud-based models (OpenAI, Anthropic, Gemini, etc.), they may not realize that:
1. Internet connectivity issues will completely block their API requests
2. All requests will incur costs and API rate limits
3. Latency will be higher compared to local models

This warning helps users make informed decisions about their routing strategy configuration.

## Requirements

### Functional Requirements

1. **Detection Logic**: Automatically detect if the prioritized models list contains at least one offline/local model
2. **Visual Warning**: Display a clear, non-intrusive warning message when no offline models are present
3. **Real-time Updates**: Warning should appear/disappear as users add/remove models from the prioritized list
4. **Local Model Classification**: Support classification of these model types as "offline/local":
   - Ollama models (always local)
   - LM Studio models (always local)
   - OpenAI-compatible models with `is_local: true` flag
5. **Contextual Help**: Provide actionable advice (e.g., "Consider adding an Ollama model as a fallback")

### Non-Functional Requirements

1. **Performance**: Detection should be instant (<100ms) as users modify the model list
2. **UX**: Warning should be visible but not annoying (no popups/modals)
3. **Accessibility**: Warning should be accessible to screen readers
4. **Internationalization**: Warning text should be easy to translate (future)

## Design

### UI Component Location

The warning should appear in the **routing strategy configuration section** when users are:
1. Creating a new auto-routing strategy
2. Editing an existing auto-routing strategy
3. Viewing the prioritized models list

**Placement**: Directly above or below the prioritized models list, with a warning icon and border.

### Warning Message

```
⚠️ No offline models selected

Your current routing strategy only includes cloud-based models. Internet
connectivity issues may interrupt your workflow.

Consider adding a local model (Ollama, LM Studio) as a fallback for offline use.
```

**Variations:**
- **Info mode** (when at least one offline model present): Show a checkmark or success message
- **Dismissible** (optional): Allow users to dismiss the warning with "Don't show again" checkbox

### Visual Design

```tsx
<div className="warning-box warning-offline-models">
  <div className="warning-icon">⚠️</div>
  <div className="warning-content">
    <h4>No offline models selected</h4>
    <p>
      Your current routing strategy only includes cloud-based models.
      Internet connectivity issues may interrupt your workflow.
    </p>
    <p className="warning-action">
      Consider adding a local model (Ollama, LM Studio) as a fallback
      for offline use.
    </p>
  </div>
</div>
```

**CSS Styling:**
- Background: Light yellow/orange (`#FFF9E6` or similar)
- Border: Solid 2px warning color (`#F59E0B`)
- Padding: 12px
- Border radius: 6px
- Font: Slightly smaller than body text
- Icon: Large enough to be visible (20-24px)

### Detection Logic

#### Helper Function

```typescript
/**
 * Determines if a provider/model combination is offline/local
 * @param provider - Provider name (e.g., "ollama", "openai")
 * @param model - Model name (e.g., "llama3.3", "gpt-4")
 * @param providerConfigs - List of provider configurations from backend
 * @returns true if the model runs locally without internet
 */
function isOfflineModel(
  provider: string,
  model: string,
  providerConfigs: ProviderConfig[]
): boolean {
  // 1. Hard-coded local providers
  const alwaysLocalProviders = ['ollama', 'lmstudio'];
  if (alwaysLocalProviders.includes(provider.toLowerCase())) {
    return true;
  }

  // 2. OpenAI-compatible providers with is_local flag
  if (provider.toLowerCase() === 'openai_compatible') {
    const providerConfig = providerConfigs.find(p => p.name === provider);
    if (providerConfig && providerConfig.provider_config?.is_local === true) {
      return true;
    }
  }

  // 3. Check if base_url is localhost (fallback for backward compat)
  const providerConfig = providerConfigs.find(p => p.name === provider);
  if (providerConfig?.provider_config?.base_url) {
    const baseUrl = providerConfig.provider_config.base_url.toLowerCase();
    if (
      baseUrl.includes('localhost') ||
      baseUrl.includes('127.0.0.1') ||
      baseUrl.includes('::1')
    ) {
      return true;
    }
  }

  // 4. All other providers are online-only
  return false;
}
```

#### Warning Display Logic

```typescript
/**
 * Checks if the prioritized models list has at least one offline model
 */
function hasOfflineModel(
  prioritizedModels: Array<{ provider: string; model: string }>,
  providerConfigs: ProviderConfig[]
): boolean {
  return prioritizedModels.some(({ provider, model }) =>
    isOfflineModel(provider, model, providerConfigs)
  );
}

// Usage in component
const showWarning = !hasOfflineModel(prioritizedModels, providerConfigs);
```

### Integration Points

#### 1. Routing Strategy Configuration Component

**File**: `src/components/routers/RouterEditorForm.tsx` (or similar)

**Changes**:
1. Import provider configurations from backend
2. Add warning component above/below prioritized models list
3. Update warning visibility when models are added/removed
4. Provide link to add offline models

#### 2. Backend Support (Tauri Commands)

**File**: `src-tauri/src/ui/commands.rs`

**New Command** (if needed):
```rust
#[tauri::command]
pub async fn is_provider_local(
    provider_registry: tauri::State<'_, Arc<ProviderRegistry>>,
    provider_name: String,
) -> Result<bool, String> {
    // Implementation to check if provider is local
    // Returns true for ollama, lmstudio, and openai_compatible with is_local flag
    Ok(/* ... */)
}
```

**Note**: This command may not be needed if provider configs are already available in the frontend.

#### 3. Provider Configuration

The warning depends on the `is_local` flag in provider configurations. This was introduced in the Offline Mode Implementation Plan.

**Example Provider Config** (YAML):
```yaml
providers:
  - name: "LocalAI"
    provider_type: "openai_compatible"
    enabled: true
    provider_config:
      base_url: "http://localhost:8080/v1"
      is_local: true  # THIS FLAG
```

## Implementation Steps

### Step 1: Update Progress Tracker
✅ **DONE** - Added to `plan/2026-01-14-PROGRESS.md` under Phase 7.4 (Routers Tab)

### Step 2: Add Detection Logic (Frontend)

1. Create helper function `isOfflineModel()`
2. Create helper function `hasOfflineModel()`
3. Add unit tests for detection logic

**Files to Create/Modify**:
- `src/utils/modelHelpers.ts` (new file for helper functions)
- `src/utils/modelHelpers.test.ts` (unit tests)

### Step 3: Add Warning Component

1. Create reusable warning component
2. Add styling (CSS/Tailwind)
3. Integrate into RouterEditorForm

**Files to Create/Modify**:
- `src/components/common/WarningBox.tsx` (reusable component)
- `src/components/routers/RouterEditorForm.tsx` (integration)

### Step 4: Wire Up Provider Configurations

1. Fetch provider configs from backend (if not already available)
2. Pass to warning component
3. Update when providers change

**Files to Modify**:
- `src/components/routers/RouterEditorForm.tsx`
- May need to add Tauri command if provider configs aren't exposed

### Step 5: Add E2E Tests

1. Test warning shows when no offline models
2. Test warning disappears when offline model added
3. Test with different provider types (ollama, openai_compatible, etc.)

**Files to Create**:
- `src/e2e/routerOfflineWarning.test.ts` (or similar)

### Step 6: Documentation

1. Update user documentation to explain warning
2. Add screenshots
3. Explain how to add offline models

**Files to Update**:
- `README.md` (user guide section)
- `plan/2026-01-14-ARCHITECTURE.md` (if needed)

## Testing Plan

### Unit Tests

1. **isOfflineModel()**:
   - Test ollama provider → true
   - Test lmstudio provider → true
   - Test openai_compatible with is_local=true → true
   - Test openai_compatible with is_local=false → false
   - Test openai_compatible with localhost URL → true
   - Test openai provider → false
   - Test anthropic provider → false

2. **hasOfflineModel()**:
   - Test empty list → false
   - Test list with only online models → false
   - Test list with one offline model → true
   - Test list with mixed models → true

### Integration Tests

1. **Warning Display**:
   - Render RouterEditorForm with no offline models
   - Verify warning is visible
   - Add an offline model
   - Verify warning disappears

2. **Provider Configuration**:
   - Test with real provider configs from backend
   - Verify detection works with actual data

### E2E Tests

1. **Router Creation**:
   - Create new router with only online models
   - Verify warning appears
   - Add ollama model to prioritized list
   - Verify warning disappears
   - Save router
   - Reload page
   - Verify warning state persists

2. **Router Editing**:
   - Edit existing router with offline models
   - Remove all offline models
   - Verify warning appears
   - Cancel editing
   - Verify changes reverted

## Edge Cases

1. **Empty Model List**: No warning (or different warning about empty list)
2. **Provider Not Found**: Assume online (show warning)
3. **Provider Config Missing**: Fallback to URL detection
4. **Concurrent Edits**: Warning should update immediately
5. **Slow Provider Fetch**: Show loading state while fetching configs

## Future Enhancements

1. **Auto-Suggest**: When warning shows, offer a button to "Add recommended offline model"
2. **Settings**: Allow users to permanently dismiss warning (with setting to re-enable)
3. **Network Detection**: Integrate with OS network status to show more urgent warning when offline
4. **Model Recommendations**: Suggest specific local models based on user's hardware
5. **Cost Analysis**: Show estimated cost savings by adding local models

## Dependencies

- **Router Refactor** (2026-01-19-ROUTER-REFACTOR-PROGRESS.md): Must be completed first
- **Provider Configuration**: Requires `is_local` flag in provider configs
- **Routing Strategy UI**: Must have routing strategy configuration UI implemented

## Timeline

**Estimated Effort**: 2-3 days

**Breakdown**:
- Detection logic: 4 hours
- Warning component: 3 hours
- Integration: 4 hours
- Testing: 4-5 hours
- Documentation: 2 hours
- **Total**: ~17-18 hours

## Success Criteria

1. ✅ Warning appears when no offline models in prioritized list
2. ✅ Warning disappears when offline model added
3. ✅ Detection works for ollama, lmstudio, and openai_compatible providers
4. ✅ Warning is visually clear and non-intrusive
5. ✅ All unit tests pass
6. ✅ All E2E tests pass
7. ✅ Documentation updated

## References

- [Offline Mode Implementation Plan](./OFFLINE-MODE-IMPLEMENTATION-PLAN.md) - Defines `is_local` flag
- [Router Refactor Progress](./2026-01-19-ROUTER-REFACTOR-PROGRESS.md) - Routing strategy changes
- [Progress Tracker](./2026-01-14-PROGRESS.md) - Phase 7.4 (Routers Tab)

---

**Document Version**: 1.0
**Last Updated**: 2026-01-20
**Author**: LocalRouter AI Team
