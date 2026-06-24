# Global Server Start/Stop Toggle (LLM + MCP)

**Date:** 2026-06-24
**Goal:** One toggle that starts/stops both the LLM HTTP server and the MCP
gateway together. Surfaced in the system tray (above Quit) and in the UI to the
right of the bottom-left status menu. **Stopping must kill all in-flight
requests**, including active streaming completions / SSE.

## Background

- LLM + MCP are both served by the single Axum server (`lr-server`); the MCP
  gateway lives in `AppState`. Stopping the Axum server stops serving both.
- `ServerManager::stop()` currently only `handle.abort()`s the accept-loop task.
  `axum::serve` spawns a **detached** task per connection, so aborting the accept
  loop does NOT terminate in-flight requests/streams. This is the gap.
- `restart_server` (event `server-restart-requested`, handled in `main.rs`)
  already starts the server from a stopped state (its internal stop is a no-op
  when stopped), so the toggle is: running → `stop_server`, stopped →
  `start_server` (emits `server-restart-requested`).

## Implementation

### Backend — kill in-flight (crates/lr-server)
1. Add deps: `tokio-util` (CancellationToken), `http-body` (Body::size_hint).
2. `start_server`: create a `CancellationToken`; pass to `build_app`; run
   `axum::serve(...).with_graceful_shutdown(token.cancelled())`; return the
   token.
3. `build_app`: add an **outermost** kill-switch middleware:
   - Race the handler against `token.cancelled()` → 503 if cancelled before a
     response is produced (kills pre-response in-flight).
   - For streaming responses (no exact `size_hint`), wrap the body in a stream
     that stops emitting once the token is cancelled (kills active SSE/streams
     mid-flight). Buffered responses (exact size) pass through untouched so
     normal JSON keeps its Content-Length.
4. `ServerManager`: store the token; `stop()` cancels it (kills in-flight) then
   aborts the handle.

### Backend — commands (src-tauri)
5. Add `start_server` Tauri command (emits `server-restart-requested`). Keep
   existing `stop_server` / `get_server_status`. Register in `main.rs`.

### Tray (src-tauri/src/ui)
6. `tray_menu.rs`: add `toggle_server` item directly above the Quit separator,
   labelled "Stop Server" / "Start Server" by current status.
7. `tray.rs`: handle `toggle_server` — stop or start by status, then rebuild
   the menu and emit status.

### UI (src/components/layout/sidebar.tsx)
8. Add a clickable start/stop toggle to the right of the bottom-left status dot,
   driven by `health.server_running`, invoking `start_server` / `stop_server`.

### Types / demo
9. `tauri-commands.ts`: params/types for `start_server`.
10. Demo mock: `start_server` handler.

## Final steps (mandatory)
- Plan review vs implementation.
- Test coverage: add a test that a streaming response is terminated on cancel.
- Bug hunt: layer ordering (outermost), no deadlock between cancel + abort,
  status events fire on both paths.
- `cargo clippy/fmt/test` (stable) + `tsc`/build. Commit. Then release v0.0.121.

## Notes
- winXP demo tray (external repo `/Users/matus/dev/winXP`) should eventually
  mirror the new tray item; out of scope for this release (noted to user).
