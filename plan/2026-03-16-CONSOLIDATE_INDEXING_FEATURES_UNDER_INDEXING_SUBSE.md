# Consolidate Indexing Features Under "Indexing" Sub-Section

## Context

Three Optimize features share the same underlying `lr-context::ContentStore` FTS5/vector indexing engine:

1. **MCP Catalog Indexing** (was "MCP Catalog Compression") — defers MCP tool/resource/prompt definitions behind IndexSearch when catalog exceeds threshold
2. **Tool Responses Indexing** (was "MCP Response RAG") — indexes + compresses large tool responses, replaces with preview + search hint
3. **Indexed Conversation Memory** (was "Memory") — persistent conversation memory, searchable via MemorySearch/MemoryRead

Currently these are 3 independent flat pages under Optimize. We want to group them under a new **"Indexing"** parent page that houses shared settings (embedding model download), with the 3 features as always-visible indented children in the sidebar.

## Target Sidebar Structure

```
Optimize (collapsible, ⌘5)
├── GuardRails
├── Secret Scanning
├── JSON Repair
├── Compression
├── Strong/Weak
└── Indexing (new parent — clickable, navigates to indexing overview)
    ├── Catalog (indented, always visible when Optimize is expanded)
    ├── Responses (indented, always visible)
    └── Memory (indented, always visible)
```

## What Goes Where

### Common "Indexing" parent page

| Setting/Section | Currently In | Why Common |
|-----------------|-------------|------------|
| Embedding model download/status | Memory Info tab | `EmbeddingService` is shared across ALL ContentStore instances |
| How FTS5 indexing works | scattered in each | Same engine description for all 3 |
| Feature summary cards (3) | — | Quick status + "Configure" links to each child |

### Stays on MCP Catalog Indexing page (unchanged except rename)

- `catalog_compression` enable toggle
- `catalog_threshold_bytes` setting
- Live session preview (active sessions + catalog sources)
- Per-client enablement card (`FeatureClientsCard`)

### Stays on Tool Responses Indexing page (unchanged except rename)

- `response_threshold_bytes` setting
- Tool indexing permission trees (gateway, virtual, client tools)
- Try It Out tab (document input → compressed preview → search → read)
- Per-client enablement card

### Stays on Indexed Conversation Memory page (minor change)

- Privacy notice
- Tool definitions preview (MemorySearch + MemoryRead)
- How it works (memory-specific flow)
- Try It Out tab
- Settings: compaction model, tool name, top-k, session timeouts
- **Remove**: "Semantic Search (Optional)" card → replaced with link to Indexing page

## Implementation

### 1. `src/constants/features.ts`

Add `indexing` feature. Rename existing 3 child entries. Add `INDEXING_CHILDREN` constant.

```typescript
import { Library } from "lucide-react"  // new icon for Indexing

// Rename existing entries:
catalogCompression: { ..., name: "MCP Catalog Indexing",         shortName: "Catalog",   ... },
responseRag:        { ..., name: "Tool Responses Indexing",      shortName: "Responses", ... },
memory:             { ..., name: "Indexed Conversation Memory",  shortName: "Memory",    ... },

// Add to FEATURES:
indexing: { icon: Library, color: "text-cyan-500", borderColor: "border-cyan-500/30",
            name: "Indexing", shortName: "Indexing", viewId: "indexing" },

// New constant:
export const INDEXING_CHILDREN: FeatureKey[] = ['catalogCompression', 'responseRag', 'memory']
```

### 2. `src/components/layout/sidebar.tsx`

**a) Add `'indexing'` to `View` type.**

**b) Extend `NavItem` with optional `subItems`:**
```typescript
interface NavItem {
  id: View
  icon: React.ElementType
  label: string
  shortcut?: string
  subItems?: NavItem[]  // NEW — always-visible nested children
}
```

**c) Replace the flat FEATURES-generated children** in `resourceNavEntries`. Instead of generating all 8 from FEATURES, build manually:
```typescript
const resourceNavEntries: NavEntry[] = [
  {
    id: 'optimize-overview', icon: Zap, label: 'Optimize', shortcut: '⌘5',
    children: [
      { id: 'guardrails' as View, icon: FEATURES.guardrails.icon, label: FEATURES.guardrails.shortName },
      { id: 'secret-scanning' as View, icon: FEATURES.secretScanning.icon, label: FEATURES.secretScanning.shortName },
      { id: 'json-repair' as View, icon: FEATURES.jsonRepair.icon, label: FEATURES.jsonRepair.shortName },
      { id: 'compression' as View, icon: FEATURES.compression.icon, label: FEATURES.compression.shortName },
      { id: 'strong-weak' as View, icon: FEATURES.routing.icon, label: FEATURES.routing.shortName },
      {
        id: 'indexing' as View,
        icon: FEATURES.indexing.icon,
        label: FEATURES.indexing.shortName,
        subItems: INDEXING_CHILDREN.map(key => ({
          id: FEATURES[key].viewId as View,
          icon: FEATURES[key].icon,
          label: FEATURES[key].shortName,
        })),
      },
    ],
  },
]
```

**d) Update `renderNavCollapsible`** — when rendering children, check for `subItems`. If present, render the parent as a clickable item, then render sub-items with extra indentation (no additional collapse toggle — always visible when Optimize is expanded):
```tsx
{group.children.map(child => {
  if (child.subItems) {
    return (
      <div key={child.id}>
        {renderNavItem(child)}
        <div className="ml-3 border-l border-border/50 pl-1 mt-0.5 space-y-0.5">
          {child.subItems.map(renderNavItem)}
        </div>
      </div>
    )
  }
  return renderNavItem(child)
})}
```

**e) Update auto-expand logic** — add indexing children to the check:
```typescript
const hasActiveChild = entry.children.some(child =>
  child.id === activeView || child.subItems?.some(sub => sub.id === activeView)
)
```

### 3. `src/App.tsx`

- Import `IndexingView` from `@/views/indexing`
- Add case to `renderView()`:
```typescript
case 'indexing':
  return <IndexingView activeSubTab={activeSubTab} onTabChange={handleChildViewChange} />
```

### 4. `src/views/indexing/index.tsx` (CREATE)

New parent page layout:

```
Header: Library icon + "Indexing" title
Subtitle: "FTS5 search engine powering MCP Catalog Indexing, Tool Responses Indexing, and Conversation Memory"

Body (scrollable, max-w-2xl):
├── How It Works card
│   - FTS5 full-text search with 3-layer fallback
│   - Markdown/JSON/plain text chunking
│   - Search + Read tool pattern
│   - Optional hybrid mode with embeddings
│
├── Semantic Search card (moved from Memory)
│   - Embedding model download button + status
│   - CheckCircle2 when downloaded/loaded
│   - "Applies to all three indexing features"
│
├── Feature summary cards (3x):
│   MCP Catalog Indexing: icon + name + enabled status + "Configure →"
│   Tool Responses Indexing: icon + name + indexing summary + "Configure →"
│   Indexed Conversation Memory: icon + name + ExperimentalBadge + status + "Configure →"
```

Uses existing Tauri commands: `get_embedding_status`, `install_embedding_model`, `get_context_management_config`, `get_memory_config`.

### 5. `src/views/memory/index.tsx`

Replace the "Semantic Search (Optional)" card on Info tab with a compact note:
```tsx
<Card>
  <CardContent className="py-3">
    <p className="text-sm text-muted-foreground">
      Semantic search (hybrid FTS5 + embeddings) is configured on the{' '}
      <button onClick={() => onTabChange?.('indexing')} className="text-primary hover:underline">
        Indexing page
      </button>.
    </p>
  </CardContent>
</Card>
```

### 6. `src/views/optimize-overview/index.tsx`

Replace the 3 individual cards (MCP Catalog Indexing, Tool Responses Indexing, Indexed Conversation Memory) with a single "Indexing" card:
- Shows `FEATURES.indexing.icon` + "Indexing" title
- Lists the 3 features as bullet points with their icons and new names
- "Configure" button navigates to `indexing` view

### 7. `website/src/components/demo/TauriMockSetup.ts`

No new commands needed — the Indexing page uses existing commands. No mock changes required.

## Files to Modify

| File | Action |
|------|--------|
| `src/constants/features.ts` | Add `indexing` entry + `INDEXING_CHILDREN`, rename 3 features |
| `src/components/layout/sidebar.tsx` | Add `'indexing'` to View, `subItems` to NavItem, build nested nav, update auto-expand |
| `src/App.tsx` | Add `'indexing'` case, import IndexingView |
| `src/views/indexing/index.tsx` | **CREATE** — parent page with embedding download + feature cards |
| `src/views/memory/index.tsx` | Replace embedding card with link to Indexing page |
| `src/views/optimize-overview/index.tsx` | Replace 3 cards with 1 "Indexing" card |
| `src/views/catalog-compression/index.tsx` | Rename title: "MCP Catalog Indexing" |
| `src/views/response-rag/index.tsx` | Rename title: "Tool Responses Indexing" |

## Verification

1. Sidebar: Optimize expands → shows GuardRails..Strong/Weak + Indexing with 3 indented children
2. Click "Indexing" → parent page with embedding model download + 3 feature summary cards
3. Click "Catalog" / "RAG" / "Memory" → navigates to existing feature pages (unchanged)
4. Auto-expand: navigating to any of the 3 child views expands the Optimize collapsible
5. Collapsed sidebar: shows Optimize icon only (same as before)
6. "Configure" buttons on Indexing page navigate to correct child views
7. Memory Info tab: embedding card replaced with link to Indexing page
8. Optimize overview: 3 cards replaced with 1 Indexing card
