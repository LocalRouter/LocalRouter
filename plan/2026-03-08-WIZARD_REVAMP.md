# Client Creation Wizard Revamp

**Date**: 2026-03-08
**Status**: In Progress

## Goals

1. Simplify wizard from 8 steps to 4 steps
2. Fix auto router bug (auto mode not applied on creation)
3. Replace custom duplicated components with shared ones
4. Make client creation quick and focused

## New Flow

```
Current:  Welcome → Template → Mode → Name → Models → MCP → Skills → Credentials  (8 steps)
New:      Template → Name+Mode → Models → Credentials  (4 steps)
```

## Changes

### Step 1: Template (keep as-is)
- `StepTemplate.tsx` - no changes needed, works well

### Step 2: Name + Mode (merge two steps)
- Combine `StepName` and `StepMode` into new `StepNameAndMode`
- Name input + LLM/MCP checkboxes in one step

### Step 3: Models (replace with StrategyModelConfiguration)
- **Key change**: Create the client BEFORE this step so we have a `strategy_id`
- Embed `StrategyModelConfiguration` with `clientContext` - this component:
  - Handles both Allowed Models and Auto Route modes correctly
  - Saves directly to backend via debounced `update_strategy`
  - Uses `UnifiedModelsSelector` for proper permissions
  - Manages RouteLLM download, pricing info, etc.
- No more custom routing mode logic in the wizard

### Step 4: Credentials (keep as-is)
- `StepCredentials.tsx` → `HowToConnect` - no changes needed

### Removed Steps
- `StepWelcome` - removed (adds friction)
- `StepMcp` - removed (configure after creation, defaults to "off")
- `StepSkills` - removed (configure after creation, defaults to "off")

### Creation Flow Change
Previously: Create client at the end (pre-credentials step)
Now: Create client after Name+Mode step, BEFORE Models step

This means:
1. Template → Name+Mode → **create_client()** → Models (live editing) → Credentials
2. Models step uses real `StrategyModelConfiguration` which saves directly to backend
3. No more batched permission-setting calls at the end

### Auto Router Bug Fix
The old wizard had its own `AutoModelConfig` type with `enabled: boolean` field,
but the real `AutoModelConfig` in `StrategyModelConfiguration` uses `permission: PermissionState`.
The wizard was sending `enabled: true` but the backend expects `permission: 'allow'`.
By using `StrategyModelConfiguration` directly, this mismatch is eliminated.

## Files Modified

- `src/components/wizard/ClientCreationWizard.tsx` - Complete rewrite
- `src/components/wizard/steps/StepNameAndMode.tsx` - New (merge of StepName + StepMode)

## Files Removed (can delete later or keep unused)

- `src/components/wizard/steps/StepWelcome.tsx` - No longer used
- `src/components/wizard/steps/StepModels.tsx` - Replaced by StrategyModelConfiguration
- `src/components/wizard/steps/StepMcp.tsx` - Removed from wizard
- `src/components/wizard/steps/StepSkills.tsx` - Removed from wizard
- `src/components/wizard/steps/StepName.tsx` - Merged into StepNameAndMode
- `src/components/wizard/steps/StepMode.tsx` - Merged into StepNameAndMode
