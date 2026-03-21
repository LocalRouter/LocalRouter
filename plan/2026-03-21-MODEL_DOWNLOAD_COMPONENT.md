# Plan: Unified Model Download Component

## Context
Four places in the UI download models (compression, strong/weak, embeddings, guardrails provider pull), each with duplicated state management, event listeners, and progress UI. The user wants a shared component for all download flows, placed in the Info tab, and guardrails models that require pulling should start disabled and be enabled on success.

## Architecture: Hook + Component

Split into two pieces so the guardrails case (list UI, not a card) can reuse the logic:

1. **`useModelDownload` hook** — state machine for download lifecycle + event listeners
2. **`ModelDownloadCard` component** — presentational card using the hook's state

---

## New Files

### `src/hooks/useModelDownload.ts`

State machine: `idle → downloading → downloaded | failed → (retry) → downloading`

```typescript
interface UseModelDownloadConfig {
  isDownloaded: boolean           // initial status from parent
  downloadCommand: string         // invoke command name
  downloadArgs?: Record<string, unknown>
  progressEvent?: string          // event name for progress (optional)
  completeEvent?: string          // event name for completion (optional, sync commands use invoke resolution)
  failedEvent?: string            // event name for failure (optional)
  normalizeProgress?: (payload: any) => number  // extract 0-1 from payload, default: p.progress
  eventFilter?: (payload: any) => boolean       // filter events (for provider pull)
  onComplete?: () => void
  onFailed?: (error: string) => void
}

// Returns: { status, progress, error, startDownload, retry }
```

Handles both sync commands (invoke blocks, progress events fire during) and async commands (invoke returns immediately, events drive state).

**Bug fix — "Another download is already in progress"**: Both `lr-compression` and `lr-embeddings` downloaders use a `DOWNLOAD_LOCK` mutex with `try_lock()`. If the user navigates away during a download and returns, the component remounts, `installing` resets to `false`, and clicking download again hits the held lock. The hook fixes this by:
- Always setting up progress event listeners on mount (even in `idle` state)
- If a progress event arrives while `idle`, automatically transitioning to `downloading`
- This picks up in-flight downloads after component remount

### `src/components/shared/ModelDownloadCard.tsx`

```typescript
interface ModelDownloadCardProps {
  title: string
  description?: string
  modelName?: string              // shown on success
  modelInfo?: string              // e.g. "80 MB"
  status: 'idle' | 'downloading' | 'downloaded' | 'failed'
  progress: number                // 0-100
  error: string | null
  onDownload: () => void
  onRetry: () => void
  downloadLabel?: string          // default: "Download"
  children?: React.ReactNode      // extra content below status
}
```

States:
- **idle**: Description + Download button
- **downloading**: Progress bar with percentage
- **downloaded**: CheckCircle2 + modelName + modelInfo badge
- **failed**: XCircle + error message + Retry button

---

## Modified Files

### `src/views/compression/index.tsx` — Info tab gets download card

- Add `useModelDownload` hook with `install_compression` / `compression-download-progress` / `compression-download-complete`
- Add `ModelDownloadCard` in **Info tab** between "Model Status" and "Default: Prompt Compression" cards
- In **Settings tab**, replace inline download button (lines 528-555) with state from the same hook instance
- Remove: `installing` state variable, inline download onClick handler

### `src/views/strong-weak/index.tsx` — Model tab uses download card

- Replace manual state (`isDownloading`, `downloadProgress`, `handleDownload`) + 3 `listenSafe` calls with `useModelDownload` hook
- Replace Status card + Download Progress card in Model tab with `ModelDownloadCard`
- Keep existing status badge for post-download states (loaded/unloaded/initializing)

### `src/views/indexing/index.tsx` — Info tab uses download card

- Replace `isDownloading` state + `downloadEmbeddingModel` function with `useModelDownload` hook
- Replace Semantic Search button (lines 116-133) with `ModelDownloadCard`
- Keep the benchmark table as `children` of the card (shown when downloaded)

### `src/views/guardrails/index.tsx` — Pull progress tracking

Different pattern (not a card, but per-model progress in a list):
- When `selection.needsPull`, add model as **disabled** (`enabled: false`)
- Track pull progress in state: `Record<string, { progress: number; status: string }>`
- Listen to `provider-model-pull-progress` globally, key by `provider_id:model_name`
- On `provider-model-pull-complete`: enable model via `toggle_safety_model`, remove from progress map
- On `provider-model-pull-failed`: remove from progress map, show error

### `src/views/guardrails/guardrails-panel.tsx` — Show pull progress in model list

- Add `pullProgress` prop: `Record<string, { progress: number; status: string }>`
- In model list items: when model has active pull, show inline progress bar + status text below name
- Show "Pulling..." badge instead of "Disabled" badge during pull

---

## Implementation Order

1. Create `src/hooks/useModelDownload.ts`
2. Create `src/components/shared/ModelDownloadCard.tsx`
3. Refactor `src/views/indexing/index.tsx` (simplest case)
4. Refactor `src/views/compression/index.tsx`
5. Refactor `src/views/strong-weak/index.tsx`
6. Refactor `src/views/guardrails/index.tsx` + `guardrails-panel.tsx`
7. Run `npx tsc --noEmit` to verify types

## Verification
- `cargo tauri dev` — test each download flow in the UI
- Verify progress bars animate for all 4 download types
- Verify retry works after simulated failure
- Verify guardrails models start disabled and become enabled after pull completes
- `npx tsc --noEmit` — type check passes
