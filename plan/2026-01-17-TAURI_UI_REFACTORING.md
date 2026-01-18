# Tauri UI Refactoring Plan

## Overview
Refactor the LocalRouter AI Tauri UI to create a consistent, maintainable structure with:
- Simplified list pages (no inline editing)
- Enhanced detail pages with tab navigation
- Reusable `DetailPageLayout` component
- Consistent navigation patterns
- Collapsed sidebar by default

## Key Changes Summary

### 1. Sidebar Changes
- **Default state**: All expandable sections collapsed on load (currently auto-expanded)
- **Rename**: "Server" → "Preferences"
- **File**: `src/components/Sidebar.tsx`

### 2. Main List Pages (Simplified)
All list pages show clean, clickable cards that navigate to detail pages. No inline editing.

**ApiKeysTab** (`src/components/tabs/ApiKeysTab.tsx`)
- Simple card list with name, status badge, creation date
- "Create New API Key" button at top
- Remove: inline name editing, enable/disable toggle, rotate, delete, model selection display
- Click card → navigate to ApiKeyDetailPage

**ProvidersTab** (`src/components/tabs/ProvidersTab.tsx`)
- Tab 1: "Add Provider" - grid of provider types + OAuth
- Tab 2: "Active Instances" - simple card list with icon, name, health badge
- Remove: edit, enable/disable, remove buttons from list
- Click card → navigate to ProviderDetailPage

**ModelsTab** (`src/components/tabs/ModelsTab.tsx`)
- Keep search/filter/sort controls
- Simple card list with model info, specs, capability badges
- Remove: chat button
- Click card → navigate to ModelDetailPage

### 3. Detail Pages (Enhanced with Tabs)

**ApiKeyDetailPage** (`src/components/apikeys/ApiKeyDetailPage.tsx`)
- Tab 1: "Settings" - name input + enabled checkbox
- Tab 2: "Model Selection" - model selection/routing config (moved from list page)
- Tab 3: "Chat" - chat testing interface

**ProviderDetailPage** (`src/components/providers/ProviderDetailPage.tsx`)
- Tab 1: "Configuration" - provider config form
- Tab 2: "Models" - list of models from this provider (clickable, navigate to ModelDetailPage)
- Tab 3: "API Keys" - NEW - list of API keys using this provider (clickable, navigate to ApiKeyDetailPage)
- Tab 4: "Chat" - chat testing interface

**ModelDetailPage** (`src/components/models/ModelDetailPage.tsx`)
- Tab 1: "Details" - specs + provider link (clickable) + API keys using this model (clickable list)
- Tab 2: "Chat" - chat testing interface

### 4. Reusable Component

**DetailPageLayout** (`src/components/layouts/DetailPageLayout.tsx` - NEW FILE)
- Based on ProviderDetailPage's current structure
- Props: icon, title, subtitle, badges, actions, tabs array
- Features: header card, horizontal tab navigation, conditional tab rendering
- Flexible to handle 1-4 tabs

## Implementation Approach

### Phase 1: Foundation
1. Create `DetailPageLayout` component
   - Use ProviderDetailPage (lines 219-282) as template
   - Make it reusable with flexible props
   - Test with mock data

2. Update Sidebar
   - Change line 35: `useState<Set<string>>(new Set())` (empty = collapsed)
   - Change line 102: `label: 'Preferences'`

### Phase 2: List Pages Refactoring
3. Refactor ApiKeysTab (lines 396-529 have complex inline editing)
   - Simplify to basic card list
   - Move all editing to detail page
   - Keep only: name, enabled badge, creation date, click navigation

4. Refactor ProvidersTab
   - Add tab navigation for "Add Provider" vs "Active Instances"
   - Simplify active instances to cards (remove edit/disable/remove buttons)
   - Keep health status display

5. Refactor ModelsTab (line 286-294 has chat button)
   - Remove chat button
   - Make cards clickable for navigation
   - Keep search/filter/sort

### Phase 3: Detail Pages Refactoring
6. Refactor ApiKeyDetailPage
   - Use DetailPageLayout component
   - 3 tabs: Settings | Model Selection | Chat
   - Move ModelSelectionTable to "Model Selection" tab

7. Refactor ProviderDetailPage
   - Use DetailPageLayout component
   - 4 tabs: Configuration | Models | API Keys | Chat
   - Add "API Keys" tab with list query

8. Refactor ModelDetailPage
   - Use DetailPageLayout component
   - 2 tabs: Details | Chat
   - Embed provider link and API keys list in Details tab

### Phase 4: Backend Support
9. Add backend commands (if needed)
   - `get_api_keys_for_provider(instance_name)` - for ProviderDetailPage API Keys tab
   - `get_api_keys_for_model(provider_instance, model_id)` - for ModelDetailPage Details tab
   - Note: Can implement with client-side filtering initially

### Phase 5: Navigation & Integration
10. Update App.tsx
    - Rename server → preferences in tab routing
    - Ensure navigation helpers work for cross-page links

11. Test all navigation paths
    - List → Detail
    - Detail → Related Detail (e.g., Provider → Model → API Key)
    - Sidebar → Detail

## Critical Files

### To Create
- `src/components/layouts/DetailPageLayout.tsx` - Core reusable component

### To Modify
- `src/components/Sidebar.tsx` - Collapse by default, rename Server
- `src/components/tabs/ApiKeysTab.tsx` - Major simplification
- `src/components/tabs/ProvidersTab.tsx` - Add tabs, simplify list
- `src/components/tabs/ModelsTab.tsx` - Remove chat button
- `src/components/apikeys/ApiKeyDetailPage.tsx` - 3 tabs with DetailPageLayout
- `src/components/providers/ProviderDetailPage.tsx` - 4 tabs with DetailPageLayout, add API Keys tab
- `src/components/models/ModelDetailPage.tsx` - 2 tabs with DetailPageLayout
- `src/App.tsx` - Update tab routing for Preferences

### Reference (Don't Modify)
- `src/components/providers/ProviderDetailPage.tsx` (lines 219-282) - Template for DetailPageLayout structure
- `src/components/visualization/ChatInterface.tsx` - Reusable chat component

## Data Flow

### List Pages
- Read-only display of items
- Click navigation to detail pages
- Create modals (for API keys, providers)

### Detail Pages
- Load item data on mount
- Tab-specific data loading (e.g., models for provider)
- Save/update actions within tabs
- Cross-navigation to related items

### New Queries Needed
```typescript
// Client-side filtering initially, optimize later if needed
const getApiKeysForProvider = (providerInstance: string, allKeys: ApiKey[]) => {
  return allKeys.filter(key =>
    key.model_selection?.some(model => model.provider === providerInstance)
  )
}

const getApiKeysForModel = (providerInstance: string, modelId: string, allKeys: ApiKey[]) => {
  return allKeys.filter(key =>
    key.model_selection?.some(model =>
      model.provider === providerInstance && model.model_id === modelId
    )
  )
}
```

## Verification Steps

### Functional Checks
1. Sidebar starts collapsed ✓
2. "Preferences" tab exists and works ✓
3. All list pages show simple cards ✓
4. All list cards are clickable and navigate ✓
5. No inline editing on list pages ✓
6. All detail pages use DetailPageLayout ✓
7. All tabs display correct content ✓
8. Settings can be edited and saved ✓
9. Chat interfaces work in all detail pages ✓
10. Cross-navigation works (Provider → Model, Model → API Key, etc.) ✓

### Visual Checks
1. Consistent styling across all pages
2. Header cards look clean (icon, title, badges, actions)
3. Tab navigation is clear and intuitive
4. List items have hover states
5. Loading states display correctly
6. Error states display correctly

### User Flow Testing
**API Key Flow:**
- API Keys list → Click key → Detail page
- Detail → Settings tab → Edit name, toggle enabled → Save
- Detail → Model Selection tab → Configure models → Save
- Detail → Chat tab → Send message

**Provider Flow:**
- Providers → Add tab → Select provider type → Create → Detail page
- Detail → Configuration tab → Edit settings → Save
- Detail → Models tab → Click model → ModelDetailPage
- Detail → API Keys tab → Click key → ApiKeyDetailPage
- Detail → Chat tab → Send message

**Model Flow:**
- Models list → Search/filter → Click model → Detail page
- Detail → Details tab → Click provider link → ProviderDetailPage
- Detail → Details tab → Click API key → ApiKeyDetailPage
- Detail → Chat tab → Send message

## Styling Guidelines

### List Item Cards
```css
background: gray-50
border: gray-200
rounded-lg
padding: 1rem
hover: gray-100
cursor: pointer
```

### Tab Navigation
```css
Active tab: blue-500 border-bottom, blue-600 text
Inactive tab: gray-600 text, hover → gray-900
```

### Badges
- Success (green): enabled, healthy
- Warning (yellow): disabled, degraded
- Error (red): error, unhealthy
- Default (gray): capabilities, info

## Success Criteria

All of the following must be true:
1. ✓ Sidebar starts collapsed, expands on click
2. ✓ "Preferences" tab exists (renamed from Server)
3. ✓ List pages show simple, clean cards
4. ✓ No inline editing on list pages
5. ✓ All detail pages use DetailPageLayout component
6. ✓ ApiKeyDetailPage has 3 tabs (Settings, Model Selection, Chat)
7. ✓ ProviderDetailPage has 4 tabs (Configuration, Models, API Keys, Chat)
8. ✓ ModelDetailPage has 2 tabs (Details, Chat)
9. ✓ All chat interfaces work
10. ✓ Cross-navigation works (clicking links navigates to correct detail page)
11. ✓ All settings save correctly
12. ✓ No console errors
13. ✓ Consistent styling across all pages

## Notes

- Keep existing ChatInterface component unchanged
- Preserve all existing Tauri command invocations
- Maintain event listener patterns
- Use existing navigation via `onTabChange` prop
- Model selection logic stays the same, just moved to detail page
- Provider configuration forms stay the same, just wrapped in DetailPageLayout
- All functionality is preserved, just reorganized for better UX
