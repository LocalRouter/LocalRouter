# Plan: Add "Try it out" Tab to Response RAG Page

## Context

The Response RAG page (`src/views/response-rag/index.tsx`) currently has 2 tabs (Info, Settings). The user wants a "Try it out" tab to interactively demonstrate how Response RAG works: show an original document, its compressed/indexed summary, and let users test IndexSearch and IndexRead tools.

The Catalog Compression page already has a similar "Try it out" tab with `CompressionPreview`—we follow that pattern.

## Changes

### 1. Backend: Add `lr-context` dependency + 3 Tauri commands

**`src-tauri/Cargo.toml`** — Add `lr-context = { workspace = true }` (already in workspace)

**`src-tauri/src/ui/commands.rs`** — Add:

- Module-level static preview store using `std::sync::OnceLock` + `parking_lot::Mutex<Option<lr_context::ContentStore>>`
- `truncate_to_char_boundary` helper (6 lines, same as `gateway_tools.rs:862`)
- `RagPreviewIndexResult` struct wrapping `compressed_preview`, `index_result`, `sources`
- Three commands:
  - `preview_rag_index(content, label, response_threshold_bytes)` → creates ContentStore, indexes content, builds compressed preview, stores for subsequent queries
  - `preview_rag_search(query?, queries?, limit?, source?)` → searches preview store
  - `preview_rag_read(label, offset?, limit?)` → reads from preview store

All `lr_context` types (`IndexResult`, `SearchResult`, `ReadResult`, `SourceInfo`, `ChunkToc`) already derive `Serialize`.

**`src-tauri/src/main.rs`** — Register 3 commands after `query_session_context_index` (~line 1869)

### 2. Frontend types

**`src/types/tauri-commands.ts`** — Add:
- `RagContentType`, `RagMatchLayer` type aliases
- `RagChunkToc`, `RagIndexResult`, `RagSearchHit`, `RagSearchResult`, `RagReadResult`, `RagSourceInfo` interfaces
- `RagPreviewIndexResult` interface (wraps compressed_preview + index_result + sources)
- `PreviewRagIndexParams`, `PreviewRagSearchParams`, `PreviewRagReadParams`

### 3. Frontend UI

**`src/views/response-rag/index.tsx`** — Add:
- Third tab trigger: "Try it out" (`value="preview"`)
- `RagPreview` component with:
  1. **Input Card**: Textarea pre-populated with sample markdown doc (~2KB API reference), label input, threshold input, "Index" button, indexing stats
  2. **Side-by-side panels** (ResizablePanelGroup): Original document vs Compressed preview
  3. **Chunk TOC** section: Shows indexed chunk hierarchy with depth-based indentation
  4. **IndexSearch card**: Query input + Search button + results rendered as markdown
  5. **IndexRead card**: Source label (from sources list) + offset + limit inputs + Read button + results as `<pre>`

### 4. Demo mock

**`website/src/components/demo/TauriMockSetup.ts`** — Add mock handlers for `preview_rag_index`, `preview_rag_search`, `preview_rag_read`

## File List

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Add `lr-context` dep |
| `src-tauri/src/ui/commands.rs` | Static store + 3 commands |
| `src-tauri/src/main.rs` | Register commands |
| `src/types/tauri-commands.ts` | Add types |
| `src/views/response-rag/index.tsx` | Add tab + RagPreview component |
| `website/src/components/demo/TauriMockSetup.ts` | Add mocks |

## Verification

1. `cargo test && cargo clippy` — backend compiles
2. `npx tsc --noEmit` — frontend types check
3. `cargo tauri dev` → navigate to Response RAG → Try it out → click Index → see side-by-side → search → read
