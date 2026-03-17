# Plan: Centralize Feature Constants (Icons, Colors, Names)

## Context

The 7 optimization features (GuardRails, Secret Scanning, JSON Repair, Prompt Compression, Strong/Weak Routing, Catalog Compression, Response RAG) each have an icon, color, and display name that are duplicated and sometimes inconsistent across 18+ files. The only shared constant today is `OPTIMIZE_COLORS` in `src/views/optimize-overview/constants.ts`, which only has color strings and lives in a view-specific directory.

**Goal**: Create a single source of truth for feature icon, color, border color, full name, short name, and view ID. Use it everywhere to eliminate inconsistencies.

## Key Inconsistencies Found

- **LLM Optimization page** (`src/views/llm-optimization/index.tsx`): Uses `text-muted-foreground` for 4/5 feature icons instead of canonical colors
- **Client tabs** (`guardrails-tab.tsx`, `secret-scanning-tab.tsx`, `compression-tab.tsx`): Hardcode color strings instead of using constants
- **FirewallApprovalCard**: Hardcodes `text-red-500` and `text-orange-500`
- **Try-it-out guardrails tab**: Hardcodes `text-red-500`
- **Command palette**: Hardcodes `Shield` and `Cpu` with no colors
- **OptimizeDiagram**: Hardcodes border color strings like `border-red-500/30`
- **Every file** independently imports feature icons from lucide-react

## New Constants File

Create `src/constants/features.ts`:

```typescript
import { Shield, KeyRound, Wrench, Minimize2, Cpu, BookText, Database } from "lucide-react"
import type { LucideIcon } from "lucide-react"

export interface FeatureDefinition {
  icon: LucideIcon
  color: string          // e.g. "text-red-500"
  borderColor: string    // e.g. "border-red-500/30"
  name: string           // full name for headings
  shortName: string      // compact name for sidebar
  viewId: string         // navigation view ID
}

export const FEATURES = {
  guardrails:          { icon: Shield,   color: "text-red-500",     borderColor: "border-red-500/30",     name: "GuardRails",            shortName: "GuardRails",       viewId: "guardrails" },
  secretScanning:      { icon: KeyRound, color: "text-orange-500",  borderColor: "border-orange-500/30",  name: "Secret Scanning",       shortName: "Secret Scanning",  viewId: "secret-scanning" },
  jsonRepair:          { icon: Wrench,   color: "text-amber-500",   borderColor: "border-amber-500/30",   name: "JSON Repair",           shortName: "JSON Repair",      viewId: "json-repair" },
  compression:         { icon: Minimize2,color: "text-blue-500",    borderColor: "border-blue-500/30",    name: "Prompt Compression",    shortName: "Compression",      viewId: "compression" },
  routing:             { icon: Cpu,      color: "text-purple-500",  borderColor: "border-purple-500/30",  name: "Strong/Weak Routing",   shortName: "Strong/Weak",      viewId: "strong-weak" },
  catalogCompression:  { icon: BookText, color: "text-teal-500",    borderColor: "border-teal-500/30",    name: "Catalog Compression",   shortName: "Catalog",          viewId: "catalog-compression" },
  responseRag:         { icon: Database, color: "text-emerald-500", borderColor: "border-emerald-500/30", name: "Response RAG",          shortName: "RAG",              viewId: "response-rag" },
} as const satisfies Record<string, FeatureDefinition>

export type FeatureKey = keyof typeof FEATURES

/** Backward-compatible alias - maps feature keys to text color class */
export const OPTIMIZE_COLORS = Object.fromEntries(
  Object.entries(FEATURES).map(([key, def]) => [key, def.color])
) as { readonly [K in FeatureKey]: string }
```

## Files to Modify (in order)

### Phase 1: Create constant + backward-compat shim
1. **Create** `src/constants/features.ts` — new file with `FEATURES` and `OPTIMIZE_COLORS` re-export
2. **Update** `src/views/optimize-overview/constants.ts` — re-export from `@/constants/features`

### Phase 2: Migrate 7 feature views (already import OPTIMIZE_COLORS)
Each: switch import to `@/constants/features`, use `FEATURES[key].icon` + `.color`, remove feature icon from lucide import

3. `src/views/guardrails/index.tsx` — `FEATURES.guardrails`
4. `src/views/secret-scanning/index.tsx` — `FEATURES.secretScanning`
5. `src/views/json-repair/index.tsx` — `FEATURES.jsonRepair`
6. `src/views/compression/index.tsx` — `FEATURES.compression`
7. `src/views/strong-weak/index.tsx` — `FEATURES.routing`
8. `src/views/catalog-compression/index.tsx` — `FEATURES.catalogCompression`
9. `src/views/response-rag/index.tsx` — `FEATURES.responseRag`

### Phase 3: Optimize overview + diagram
10. `src/views/optimize-overview/index.tsx` — use `FEATURES` for all 7 cards (icons + colors)
11. `src/views/optimize-overview/OptimizeDiagram.tsx` — use `FEATURES` for icons + colors + borderColors

### Phase 4: LLM Optimization page (biggest inconsistency fix)
12. `src/views/llm-optimization/index.tsx` — replace `text-muted-foreground` with canonical colors, use `FEATURES` icons

### Phase 5: Client tabs
13. `src/views/clients/tabs/guardrails-tab.tsx` — replace hardcoded `Shield` + `text-red-500`
14. `src/views/clients/tabs/secret-scanning-tab.tsx` — replace hardcoded `KeyRound` + `text-orange-500`
15. `src/views/clients/tabs/compression-tab.tsx` — replace hardcoded `Minimize2` + `text-blue-500`

### Phase 6: Shared components
16. `src/components/shared/FirewallApprovalCard.tsx` — `guardrail` and `secret_scan` cases in `getHeaderContent`
17. `src/views/try-it-out/guardrails-tab/index.tsx` — replace hardcoded `Shield` + `text-red-500`
18. `src/components/layout/command-palette.tsx` — replace hardcoded `Shield` and `Cpu` imports, add colors

### Phase 7: Sidebar
19. `src/components/layout/sidebar.tsx` — use `FEATURES[key].icon` + `.shortName` for the optimize children array

### Phase 8: Cleanup
20. Delete body of `src/views/optimize-overview/constants.ts` (keep as re-export shim, or delete if all importers migrated)

## Files NOT changed (intentionally)

- **Dashboard** (`src/views/dashboard/index.tsx`): Uses different icons for stats context (GitBranch for routing, FileDown for compression). These represent metrics, not features — leave as-is.
- **Client LLM optimize tab** (`llm-optimize-tab.tsx`): Just a compositor, no icons/colors.
- **TauriMockSetup** (`website/`): No feature icon/color references.

## Verification

1. `npx tsc --noEmit` — type check passes
2. `cargo tauri dev` — visual inspection of:
   - Sidebar icons match feature views
   - Optimize overview cards have correct colored icons
   - LLM Optimization page cards now have colored icons (not muted)
   - Client tabs (GuardRails/Secret Scanning/Compression) show correct colored icons
   - OptimizeDiagram pills have matching icon + border colors
   - FirewallApprovalCard guardrail/secret cases show correct icons + colors
   - Command palette shows correct icons
3. Grep for hardcoded feature icons to confirm no remaining stale references:
   ```
   rg 'Shield.*text-red|KeyRound.*text-orange|Wrench.*text-amber|Minimize2.*text-blue|Cpu.*text-purple|BookText.*text-teal|Database.*text-emerald' src/
   ```
