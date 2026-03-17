# Refactor QuickSetupTab to Three-Tabbed Connection Modes

## Context

The QuickSetupTab in HowToConnect.tsx currently mixes three connection modes in a flat layout: manual setup instructions (collapsed `<details>`), a "Try It Out" button, and a "Keep config in sync" toggle. The user wants these separated into a clear three-tabbed view: **Manual**, **Temporary**, and **Auto**.

## Approach

Restructure the `QuickSetupTab` component (lines 152-526 of `HowToConnect.tsx`) to use inner tabs. No backend changes needed — this is purely a frontend layout refactoring.

## File to Modify

`src/components/client/HowToConnect.tsx` — the only file changing.

## Changes

### 1. Add Badge import (line 14 area)

```tsx
import { Badge } from "@/components/ui/Badge"
```

### 2. Extract a `LaunchResultDisplay` helper (after `CopyableCodeBlock`, ~line 149)

Avoids duplicating the result display JSX between Temporary and Auto tabs. Uses `LaunchResult` type, `Label`, and `CopyableCodeBlock` already in scope.

### 3. Split `result` state into two

Replace the single `result` state with `temporaryResult` and `autoResult`. Update handlers:
- `handleTryItOut` → uses `setTemporaryResult`
- `handleToggleSyncConfig` / `handleManualSync` → use `setAutoResult`

### 4. Compute tab visibility + grid class

```tsx
const innerTabCount = 1 + (supportsTryItOut ? 1 : 0) + (supportsPermanent ? 1 : 0)
const innerGridCols = innerTabCount === 1 ? "grid-cols-1" : innerTabCount === 2 ? "grid-cols-2" : "grid-cols-3"
```

### 5. Restructure the render

**Shared above tabs** (unchanged):
- App header (icon, name, description)
- Install status box

**Inner tabs:**

| Tab | Shown When | Default | Content |
|-----|-----------|---------|---------|
| **Manual** | Always | Yes | Env vars / config file instructions, MCP proxy config, docs link — no longer collapsed |
| **Temporary** | `supports_try_it_out` | No | Description ("One-time — no files modified"), Try It Out button, `temporaryResult` display |
| **Auto** | `supports_permanent_config` | No | Sync toggle (on/off), sync-now button when on, `autoResult` display. Tab trigger has purple **EXPERIMENTAL** badge |

**Tab trigger for Auto:**
```tsx
<TabsTrigger value="auto" className="text-xs gap-1">
  <RefreshCcw className="h-3 w-3" />
  Auto
  <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400 ml-1 text-[10px] px-1.5 py-0">
    EXPERIMENTAL
  </Badge>
</TabsTrigger>
```

### Content Migration Map

| Old Location | New Location |
|---|---|
| Header (icon, name) | Shared — above tabs |
| Install status | Shared — above tabs |
| Try It Out button | **Temporary** tab |
| Config sync toggle | **Auto** tab |
| "One-time" description | **Temporary** tab (above button) |
| Result display | **Split** into `temporaryResult` / `autoResult` per tab |
| MCP Proxy info | **Manual** tab |
| Collapsed Manual Instructions | **Manual** tab (no longer collapsed) |

## What Does NOT Change

- No backend/Rust changes
- No TypeScript type changes
- No demo mock changes (`TauriMockSetup.ts`)
- Outer HowToConnect tabs (Quick Setup / Models / MCP) unchanged
- `QuickSetupTab` props interface unchanged
- All `useEffect` hooks unchanged
- Handler function logic unchanged (only which `setResult` they call)

## Verification

1. `npx tsc --noEmit` — TypeScript compiles
2. Template with both capabilities (e.g. claude-code) → all three tabs visible
3. Template with neither capability → only Manual tab
4. Template with only `supports_try_it_out` → Manual + Temporary
5. Template with only `supports_permanent_config` → Manual + Auto (with EXPERIMENTAL badge)
6. Try It Out result in Temporary tab doesn't affect Auto tab and vice versa
7. Manual tab content is directly visible (no collapsed `<details>`)
