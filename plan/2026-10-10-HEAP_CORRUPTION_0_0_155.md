# Heap corruption and vanished tray icon in 0.0.155 (macOS)

## Problem

After the auto-update 0.0.154 → 0.0.155 (launched 21:56:43 on 2026-10-10)
the LocalRouter menu bar icon was missing. About 3 minutes later the app
aborted (`~/Library/Logs/DiagnosticReports/localrouter-2026-10-10-215957.ips`):
EXC_BREAKPOINT, `BUG IN CLIENT OF LIBMALLOC: memory corruption of free block`,
raised on the main thread in `_xzm_xzone_malloc_from_freelist_chunk` while
WebKit handled a mouse event. The detection site is only where malloc noticed
that a free block had been written to. After relaunch the icon was back.

## Evidence

- Unified log of the crashed process (pid 51295): at 21:56:51.570, 6.5 s
  after launch, the process sent `NSStatusItemClearAutosaveStateAction` and
  MenuBarAgent unregistered its `com.apple.appkit.status-items` scene, i.e.
  the `NSStatusItem` was removed (`removeStatusItem`). No status item scene was
  registered again before the crash. Searching the log since 2026-10-08 finds
  this action exactly once, in the crashed process; normal quits and updates
  do not produce it.
- The config backups show the tray graph setting toggled off and on at
  21:57:11 and 21:57:13 (`tray_graph_enabled`), consistent with an attempt to
  get the icon back; it did not return.
- At the time of the crash a tokio worker was inside
  `TrayGraphManager::update_tray_graph_impl`, waiting for a main-thread tray
  call (`run_item_main_thread!`'s channel); every other thread was idle.
- tray-icon removes the status item only in `TrayIcon::remove`, reached from
  `set_visible(false)` (never called) or from `Drop` of the platform
  `TrayIcon`, which runs when the last handle goes.

## Root cause

`tauri::tray::TrayIcon` (2.11) wraps `tray_icon::TrayIcon`, which is an
`Rc<RefCell<platform::TrayIcon>>`; Tauri declares the wrapper `Send + Sync`
("we make sure it always runs on the main thread"), but handles are created
and dropped on whichever thread asks:

- `AppHandle::tray_by_id` returns `Arc::unwrap_or_clone` of the resource
  table entry, i.e. clones the `Rc` on the calling thread;
- every `TrayIcon` method (`set_menu`, `set_tooltip`, `with_inner_tray_icon`,
  …) clones the handle on the calling thread into a main-thread task, where
  the clone is dropped;
- the caller's handle is dropped on the calling thread.

LocalRouter called `tray_by_id` from background threads all the time: the
tray graph manager (presence check every update, icon/tooltip push),
`rebuild_tray_menu` (health-status events emitted from concurrent health
checks, firewall cleanup, usage text changes, ~40 async commands),
`update_tray_icon` (server status, `llm-request`). These non-atomic
increments/decrements race each other and the main thread. A lost increment
lets the count reach zero while handles are still alive: the platform
`TrayIcon` is dropped, which removes the status item (the icon vanishes), and
its allocation is freed. Every later handle clone/drop and `RefCell` borrow
writes the count/borrow flag into that freed block, which xzone malloc
reports as "memory corruption of free block" the next time it hands out
memory from that chunk.

The race is old (the pattern predates 0.0.154); its odds depend on how many
tray accesses overlap. 0.0.155 is not proven to have added a specific new
caller; the startup burst (health checks for all providers completing at
once, the tray graph manager's first updates, usage polling and its tray
redraws) is where the count was lost here. The 0.0.153 crash
(`TRAY_MENU_USE_AFTER_FREE`, garbage `MenuChild` behind a menu item) is
consistent with the same corruption, but that is unproven.

## Reproduction

A temporary (uncommitted) hook in the debug build spawned 6 threads calling
the real `ui::tray::update_tray_icon` / `rebuild_tray_menu` every 2 ms. On the
unfixed code the app aborted within about a second, both runs, with Rust's
debug UB check in `Rc::inc_strong` (strong count already 0):

```
RcInner<RefCell<tray_icon::platform_impl::platform::TrayIcon>>::inc_strong
<tray_icon::TrayIcon as Clone>::clone
<tauri::tray::TrayIcon as Clone>::clone
Arc<tauri::tray::TrayIcon>::unwrap_or_clone
TrayManager::tray_by_id
AppHandle::tray_by_id
localrouter::ui::tray::update_tray_icon
```

Release builds have no such check, so the same clone writes into the freed
block instead (the production crash).

## Fix

- `ui::tray::with_tray(app, f)`: the only way to reach the tray icon. It
  dispatches to the main thread (inline when already there) and takes, uses
  and drops the handle there, so the `Rc` is never touched off the main
  thread.
- `ui::tray::tray_present(app)`: a `TrayPresent` marker managed in
  `setup_tray`, for the "is there a tray" checks that used `tray_by_id`.
- `rebuild_tray_menu`, `update_tray_icon`, both `apply_presentation`
  variants in `tray_graph_manager.rs`, the tray graph manager's presence check
  and the window close handler use them. The macOS `apply_presentation` no
  longer blocks its task on the main thread.
- Regression guard: workspace `clippy.toml` disallows
  `tauri::AppHandle::tray_by_id` and `tauri::App::tray_by_id`; CI's
  `-D warnings` fails any new call. The single allowed call is inside
  `with_tray`, on the main thread.

## Verification

- Clippy guard checked: a probe call to `app.handle().tray_by_id(..)` and to
  `app.tray_by_id(..)` both fail `cargo clippy -D warnings` with the
  disallowed-method error.
- Same stress hook on the fixed build, with `MallocScribble=1
  MallocGuardEdges=1 MallocStackLogging=1`, for 11 minutes (about 1.4 million
  tray calls from 6 threads, a menu rebuild every ~40 ms on two of them)
  while clicking the tray item through System Events every ~20 s: no panic,
  no malloc error, the status item present (70x24) in all 68 samples, no
  `NSStatusItemClearAutosaveStateAction` in the process log, RSS flat
  (388 → 428 MB); stopped by SIGTERM.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo test -p localrouter` (43 suites, all
  pass). These ran in a private target dir: in the shared one, another
  worktree's newer `lr-usage` replaced this branch's build.

## Final steps

- [x] Plan review: every former `tray_by_id` caller (`rebuild_tray_menu`,
  `update_tray_icon`, both `apply_presentation`s, the tray graph presence
  check, the window close handler) goes through `with_tray`/`tray_present`;
  `grep tray_by_id` finds only the call inside `with_tray`; the menu-keeper
  swap still runs on the main thread.
- [x] Test coverage review: the race cannot be unit-tested without a Tauri
  runtime (the crate has no `tauri/test` mock); the clippy ban is the
  regression guard and was shown to fire for both `AppHandle` and `App`. The
  fix was exercised end to end by the stress run above.
- [x] Bug hunt: `with_tray` runs inline on the main thread, so tray event
  handlers that rebuild synchronously still work; Tauri does not hold its
  tray-icon lock while running handlers, so `tray_by_id` there cannot
  deadlock. `TrayPresent` is managed after the icon is built, so a rebuild
  before that is a no-op as before. After exit cleanup `tray_by_id` returns
  `None` and `with_tray` does nothing. Errors from the dispatched tray calls
  are logged instead of returned (their callers only logged them). The
  macOS icon push no longer waits for the main thread, so the change hash is
  recorded before the icon lands; a push that fails is logged and the next
  differing icon is pushed again.
- [x] Commit (not pushed, per instructions for this fix).
