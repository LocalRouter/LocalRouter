# Plan: Add Visual Embeds to Documentation Pages

## Context

The docs pages currently display only markdown text. The home page already demonstrates how to embed actual app components (FirewallApprovalDemo, GuardrailApprovalDemo) by importing shared components via `@app/*` path alias and wrapping them with demo data. We want to extend this pattern to doc pages, embedding interactive app UI at relevant sections.

## Approach

### 1. Create a doc embed registry system

**New file: `website/src/components/docs/DocEmbeds.tsx`**

A central mapping of `section-id -> React component`. The `DocContent` component in Docs.tsx will check this map and render the embed after the markdown content for matching sections.

### 2. Modify Docs.tsx to render embeds

**File: `website/src/pages/Docs.tsx`**

- Import the embed registry
- In `DocContent`, after rendering markdown, check if an embed exists for the current `id`
- Render the embed in a styled container (rounded border, centered, with a subtle "Live Preview" label)

### 3. Create demo components

All self-contained with hardcoded demo data (no Tauri mocking needed). Styled with `className="dark"` wrapper to scope CSS variables, matching the existing demo pattern.

#### a) Firewall Approval Flow
**Section: `approval-flow`**
- Reuse existing `FirewallApprovalDemo` from `website/src/components/FirewallApprovalDemo.tsx`
- Already exists, just needs to be registered in the embed map

#### b) GuardRails Content Safety
**Section: `content-safety-scanning`**
- Reuse existing `GuardrailApprovalDemo` from `website/src/components/GuardrailApprovalDemo.tsx`
- Already exists, just needs to be registered

#### c) Model Routing - Auto Route
**Section: `auto-routing`**
**New file: `website/src/components/docs/ModelRoutingDemo.tsx`**

Static demo showing:
- A mode selector dropdown showing "Auto Route" mode
- A prioritized model list with drag handles (static, non-interactive) showing models like "claude-sonnet-4 (Anthropic)", "gpt-4o (OpenAI)", "llama-3.3-70b (Ollama)"
- Visual indicator that models are tried in order with fallback arrows between them

#### d) Marketplace Search Results
**Section: `marketplace-overview`**
**New file: `website/src/components/docs/MarketplaceDemo.tsx`**

Static demo showing:
- A search bar with "filesystem" pre-filled
- 3 MCP server result cards with name, vendor, description, transport badges (stdio/remote), and Install button
- Uses the same Card/Badge/Button components from `@app/components/ui/`

#### e) Gated Installation - Firewall Popup for Marketplace
**Section: `gated-installation`**
Reuse `FirewallApprovalCard` with marketplace request type:
- `requestType="marketplace"` shows install approval
- Hard-coded demo showing a marketplace install approval dialog

**New file: `website/src/components/docs/MarketplaceInstallDemo.tsx`**

#### f) Monitoring Metrics Chart
**Section: `graph-data`**
**New file: `website/src/components/docs/MetricsDemo.tsx`**

Self-contained Recharts chart with hardcoded time-series data showing:
- A bar chart of requests over time
- Time range selector showing "Last 24 Hours"
- Metric type selector showing "Requests"
- Matches the visual style of the real `MetricsChart` component

## Files to create

| File | Purpose |
|------|---------|
| `website/src/components/docs/DocEmbeds.tsx` | Registry mapping section IDs to demo components |
| `website/src/components/docs/ModelRoutingDemo.tsx` | Auto-route model prioritization demo |
| `website/src/components/docs/MarketplaceDemo.tsx` | Marketplace search results demo |
| `website/src/components/docs/MarketplaceInstallDemo.tsx` | Gated installation approval demo |
| `website/src/components/docs/MetricsDemo.tsx` | Metrics chart demo |

## Files to modify

| File | Change |
|------|--------|
| `website/src/pages/Docs.tsx` | Import DocEmbeds, render in DocContent |

## Existing files to reuse

| File | Reused in |
|------|-----------|
| `website/src/components/FirewallApprovalDemo.tsx` | Firewall approval-flow embed |
| `website/src/components/GuardrailApprovalDemo.tsx` | GuardRails content-safety-scanning embed |
| `src/components/shared/FirewallApprovalCard.tsx` | MarketplaceInstallDemo (marketplace request type) |
| `src/components/ui/Card.tsx` | MarketplaceDemo, ModelRoutingDemo |
| `src/components/ui/Button.tsx` | All demos |
| `src/components/ui/Badge.tsx` | MarketplaceDemo |
| `src/components/ui/Input.tsx` | MarketplaceDemo |

## Embed placement summary

| Doc Section | Embed | Type |
|-------------|-------|------|
| Firewall > approval-flow | FirewallApprovalDemo | Existing |
| GuardRails > content-safety-scanning | GuardrailApprovalDemo | Existing |
| Model Selection > auto-routing | ModelRoutingDemo | New |
| Marketplace > marketplace-overview | MarketplaceDemo | New |
| Marketplace > gated-installation | MarketplaceInstallDemo | New |
| Monitoring > graph-data | MetricsDemo | New |

## Verification

1. Run `cd website && npm run dev` to start the dev server
2. Navigate to `/docs/firewall` - verify FirewallApprovalDemo appears after the approval-flow text
3. Navigate to `/docs/guardrails` - verify GuardrailApprovalDemo appears
4. Navigate to `/docs/model-selection-routing` - verify ModelRoutingDemo appears
5. Navigate to `/docs/marketplace` - verify MarketplaceDemo and MarketplaceInstallDemo appear
6. Navigate to `/docs/monitoring` - verify MetricsDemo appears
7. Run `npx tsc --noEmit` from the website directory to verify types
