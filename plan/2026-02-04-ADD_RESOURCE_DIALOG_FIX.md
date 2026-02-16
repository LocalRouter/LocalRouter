# Fix Add Resource Dialog Issues

## Issues to Fix

1. **Skills marketplace shows "No skills found"** - Skills search isn't triggering on tab open
2. **Duplicate "Custom MCP Server" button** - Remove from McpServerTemplates since we now have the Custom tab
3. **MCP marketplace popup dialog** - Should be inline, not a separate popup
4. **Template/Marketplace selection flow** - Should show form as "page 2" with back button, not switch to Custom tab

## New Design: Page-Based Flow

Instead of 3 tabs (Templates | Custom | Marketplace), use a **2-tab + page flow**:

```
┌─────────────────────────────────────────┐
│ Add MCP                                 │
├─────────────────────────────────────────┤
│ [Templates] [Marketplace]               │  ← Page 1: Tabs
│                                         │
│  ┌─────┐ ┌─────┐ ┌─────┐               │
│  │ Git │ │Slack│ │ ... │  ← Click any  │
│  └─────┘ └─────┘ └─────┘               │
└─────────────────────────────────────────┘
                    ↓ Click
┌─────────────────────────────────────────┐
│ Add MCP                                 │
├─────────────────────────────────────────┤
│ ← Back                                  │  ← Page 2: Form
│                                         │
│ [GitHub Icon] GitHub                    │  ← Selected item header
│ "Repositories, issues, pull requests"  │
│                                         │
│ Server Name: [GitHub____________]       │  ← Pre-filled form
│ Command: [npx -y @mcp/server-github___] │
│ ...                                     │
│                                         │
│              [Cancel] [Create]          │
└─────────────────────────────────────────┘
```

## Implementation Steps

### Step 1: Remove "Custom MCP Server" button from McpServerTemplates
**File:** `src/components/mcp/McpServerTemplates.tsx`

Remove the dashed "Custom MCP Server" button (lines 420-429). The Templates tab now only shows actual templates.

### Step 2: Refactor MCP dialog to page-based flow
**File:** `src/views/resources/mcp-servers-panel.tsx`

Changes:
1. Remove `createTab` state (no more tabs within dialog)
2. Add `dialogPage: "select" | "configure"` state
3. Add `selectedSource: { type: "template" | "marketplace", data: McpServerTemplate | McpServerListing } | null` state
4. Change dialog content:
   - Page "select": Show 2 tabs (Templates | Marketplace)
   - Page "configure": Show form with back button and pre-filled fields
5. When template clicked → set selectedSource, switch to "configure" page
6. When marketplace item "Add" clicked → set selectedSource, switch to "configure" page
7. Back button → clear selectedSource, switch to "select" page

### Step 3: Remove popup dialogs from MarketplaceSearchPanel
**File:** `src/components/add-resource/MarketplaceSearchPanel.tsx`

Changes:
1. Remove the `Dialog` for MCP install configuration
2. Remove the `AlertDialog` for Skills install confirmation
3. Change `handleMcpInstallClick` to call a new callback prop `onSelectMcp(item: McpServerListing)` instead of opening dialog
4. Change `handleSkillInstallClick` to call a new callback prop `onSelectSkill(item: SkillListing)` instead of installing directly
5. Change button from "Download" icon to "Add" or "+" text

Add new props:
```typescript
interface MarketplaceSearchPanelProps {
  type: "mcp" | "skill"
  onSelectMcp?: (item: McpServerListing) => void  // NEW: for MCP items
  onSelectSkill?: (item: SkillListing) => void    // NEW: for Skill items
  // ...
}
```

### Step 4: Create unified configuration form component
**File:** `src/components/add-resource/McpConfigForm.tsx` (NEW)

Extract the MCP configuration form into a reusable component:
- Accepts `source: { type: "template" | "marketplace", template?: McpServerTemplate, listing?: McpServerListing }`
- Shows source header (icon, name, description)
- Shows setup instructions if available
- Pre-fills form based on source
- Handles form submission
- Has Back and Create buttons

### Step 5: Fix Skills marketplace search
**File:** `src/components/add-resource/MarketplaceSearchPanel.tsx`

The `searchSkills()` function is called in `useEffect` when `config?.enabled` changes, but it may not trigger on first render inside the tab.

Fix: Call `searchSkills()` immediately when the component mounts (not just when config changes).

### Step 6: Update Wizard MCP step
**File:** `src/components/wizard/steps/StepMcp.tsx`

Apply the same page-based flow changes as Step 2.

### Step 7: Update Provider dialog
**File:** `src/views/resources/providers-panel.tsx`

Same page-based flow: When a provider template is clicked, show the form as a "next page" with back button instead of staying on same tab.

### Step 8: Create Skills config page
**File:** `src/components/add-resource/SkillConfigForm.tsx` (NEW)

Create a skill configuration/preview component:
- Shows skill name, description, and icon
- Shows detected resources (like what appears when opening an existing skill)
- Shows detected scripts
- Has Back and Install buttons
- User can review what they're adding before installation

## Files to Modify

| File | Changes |
|------|---------|
| `src/components/mcp/McpServerTemplates.tsx` | Remove "Custom MCP Server" button |
| `src/views/resources/mcp-servers-panel.tsx` | Page-based flow, remove Custom tab |
| `src/components/add-resource/MarketplaceSearchPanel.tsx` | Remove popups, add onSelectMcp/onSelectSkill callbacks, fix skills search |
| `src/components/wizard/steps/StepMcp.tsx` | Same page-based flow |
| `src/views/resources/providers-panel.tsx` | Same page-based flow |
| `src/views/skills/index.tsx` | Page-based flow with skill config page |

## Files to Create

| File | Purpose |
|------|---------|
| `src/components/add-resource/McpConfigForm.tsx` | Reusable MCP configuration form |
| `src/components/add-resource/SkillConfigForm.tsx` | Skill preview/config form with resources & scripts |

## Verification

1. **MCP Templates tab**: Click template → see form page with back button, fields pre-filled
2. **MCP Marketplace tab**: See results load, click "Add" → see form page with back button, fields pre-filled
3. **Skills Marketplace tab**: See results load (not "No skills found"), click "Add" → see skill config page with details, resources, scripts, and Install button
4. **Wizard MCP step**: Same flow as MCP dialog
5. **Provider Templates tab**: Click template → see form page with back button, fields pre-filled
6. **Back button**: Returns to selection without losing form data until explicitly cleared
