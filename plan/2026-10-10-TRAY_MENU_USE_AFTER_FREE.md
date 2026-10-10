# Tray menu use-after-free crash (macOS)

## Problem

LocalRouter 0.0.153 aborted at 12:04 on 2026-10-10
(`~/Library/Logs/DiagnosticReports/localrouter-2026-10-10-120401.ips`):
`core::result::unwrap_failed` in `muda::PlatformIcon::to_nsimage`, called from
`muda::MenuItem::fire_menu_item_action` during
`NSMenuTrackingSession _performPostTrackingDismissalActions`, i.e. while
AppKit ran the action of a clicked menu item.

`to_nsimage` is only reached from the About-panel branch for an About item
with an icon. LocalRouter has no About item with an icon (Tauri's default
app menu sets none), so the item's state was garbage:

- muda's `NSMenuItem` subclass keeps a raw `*const MenuChild` ivar
  (`muda-0.19.3/src/platform_impl/macos/mod.rs:1030`) that nothing clears
  when the `MenuChild` is dropped.
- `rebuild_tray_menu` calls `TrayIcon::set_menu`, and tray-icon drops the
  previous menu immediately (`tray-icon-0.25.1 .../macos/mod.rs:127`);
  Tauri keeps no other reference.
- The tray menu is rebuilt from background events while it can be open:
  health-status changes, firewall request expiry, and (since the usage
  tracking feature) usage text changes every 30 s.

Rebuilding while the menu was on screen freed the displayed items; clicking
one read freed memory. With the usage section rebuilding every 30 s while
usage changes, 0.0.154 would hit this more often.

## Fix

`src-tauri/src/ui/tray_menu_keeper.rs`: every menu handed to the tray is kept
(`TrayMenuKeeper`, managed state seeded with the initial menu in
`setup_tray`). `rebuild_tray_menu` swaps menus on the main thread; replaced
menus are freed only by a later rebuild that runs while no menu can be open,
which on macOS is when the main run loop is in `NSDefaultRunLoopMode` (menus
track in `NSEventTrackingRunLoopMode`; a nil mode counts as possibly open).
The action of a dismissed menu has run by then, because AppKit performs it
before the tracking session returns. Other platforms keep the last replaced
menu only.

## Verification

- Unit tests for the bookkeeping (kept until a closed rebuild; nothing freed
  while possibly open; nil run loop mode counts as open).
- Dev app (`cargo tauri dev --no-watch`, debug log of freed/kept counts):
  idle rebuilds free the previous menu (`freeing 1, keeping 1`); with the tray
  menu held open via AppleScript, three background rebuilds kept 2, 3, 4
  menus; the first rebuild after closing freed 4. Opening the menu, waiting
  for a background rebuild and then clicking an item of the replaced menu
  delivered `tray_stats_open__all` and the app kept running.
- `cargo clippy -p localrouter --all-targets -D warnings`, `cargo fmt --check`,
  `cargo test -p localrouter --lib -- ui::tray`.

## Final steps

- [x] Plan review: every `set_menu` call goes through `rebuild_tray_menu`;
  the initial menu is seeded into the keeper.
- [x] Test coverage review: bookkeeping unit-tested; AppKit behaviour checked
  in the dev app.
- [x] Bug hunt: if `set_menu` fails the new menu is current in the keeper
  while the tray still shows the old one, which stays retired and alive; a
  missing keeper state falls back to the previous behaviour.
- [x] Commit and push.
