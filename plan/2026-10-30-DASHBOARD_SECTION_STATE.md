# Dashboard section state that survives an abrupt exit

## Problem

The Dashboard's folded sections (`Request traffic`, `Usage limits`), the request
monitor's filter and the detail dock's side and size were kept in the WebView's
`localStorage`. In the macOS app (WKWebView) WebKit buffers those writes and
flushes them later; when the process ends without a graceful WebKit shutdown (a
debug app stopped from the terminal, a crash, a force quit) the recent writes
are lost. The debug app's WebKit store showed an empty WAL and no `traffic` key
after a session in which sections were toggled, so the state was not remembered.

## Design

- **Backend store** — `lr_config::UiStateStore` (`crates/lr-config/src/ui_state.rs`)
  keeps a JSON map in `ui_state.json` beside `settings.yaml`, in the directory
  `ConfigManager::config_path()` lives in (so `~/.localrouter-dev/` and
  `~/.localrouter/` stay separate). Each change is written before the command
  returns: temp file, `fsync`, rename. It is separate from the config, so a UI
  toggle never rewrites `settings.yaml` or emits `config-changed`. Bounds: keys
  ≤ 200 characters, values ≤ 16 KB of compact JSON, ≤ 500 keys; `null` removes
  a key; an unchanged value writes nothing; a malformed file is set aside as
  `ui_state.json.corrupt`.
- **Commands** — `get_ui_state` (whole map) and `set_ui_state({ key, value })`
  in `src-tauri/src/ui/commands_ui_state.rs`; the write runs on the blocking
  pool. Types in `src/types/tauri-commands.ts`; the website demo mocks them
  with an in-memory map.
- **Frontend** — `src/lib/ui-state-store.ts` (no React/Tauri imports, unit
  tested) holds the map. `App` loads it alongside the setup-wizard check before
  the first view renders, so sections appear in their saved state without a
  flicker (a 3 s timeout keeps a stuck backend from blocking startup). Writes
  update memory and the localStorage mirror at once and are queued to the
  backend, one in flight, latest value per key, so they arrive in order. A key
  the backend has never stored is adopted from localStorage on first read and
  written through, so state saved before this change carries over. When the
  backend is unavailable (or in the website demo after a reload) localStorage
  remains the source.
- **Hook** — `usePersistentUiState(key, fallback, decode)` in
  `src/hooks/usePersistentUiState.ts` (built on `useSyncExternalStore`), plus
  `readUiState` / `writeUiState` for one-off reads and writes. Decoders in
  `src/views/dashboard/section-state.ts` accept both the new JSON values and the
  legacy localStorage strings.
- **Users** — `useCollapsed` (both Dashboard sections), the detail dock's side
  and per-side size, and the request monitor's filter. Keys are unchanged.

## Steps

- [x] Store, commands, TS types, demo mock
- [x] Store/hook on the frontend, App startup load, migration
- [x] Migrate dashboard sections, dock, monitor filter
- [x] Rust tests (persist/reload, bounds, atomic write, failure, corrupt file)
- [x] Unit tests (source of truth, migration, ordering/coalescing, fallback, timeout)
- [x] Plan review, test coverage review, bug hunt
- [x] Commit and push (branch `feat/dashboard-section-state`)
