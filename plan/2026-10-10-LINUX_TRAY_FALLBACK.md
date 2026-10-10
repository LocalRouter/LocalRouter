# Linux: run without a tray instead of aborting (#22)

## Problem
On Linux the tray icon goes through `tray-icon` → `libappindicator-sys` 0.9,
which `dlopen`s libayatana-appindicator3 / libappindicator3 lazily the first
time a tray is built and **panics** if none loads. Release builds use
`panic = "abort"`, so the existing `catch_unwind` around `setup_tray` never
ran and the whole app died at "Setting up system tray":

- AppImage on Fedora 44: the host `libayatana-ido3` needs GLib ≥ 2.80
  (`g_once_init_leave_pointer`), the AppImage bundles the older GLib of its
  ubuntu-22.04 build → load fails → abort.
- Flatpak: the manifest shipped no AppIndicator library at all → abort.

Even with a survivable failure, closing the main window hid it ("minimized to
tray"), leaving no way back to the app and no way to quit it.

## Design
1. `src-tauri/src/ui/tray_support.rs`
   - `APPINDICATOR_LIBRARIES`: the four names `libappindicator-sys` tries, in
     its order (`.so.1` pair, then the `backcompat` `.so` pair, enabled by
     `libappindicator`'s default features).
   - `first_loadable(names)`: `libloading::Library::new` for each name — the
     same call (`RTLD_LAZY | RTLD_LOCAL`) the sys crate uses, so it fails
     exactly when the sys crate would panic. Collects one error per name.
   - `check_tray_backend()`: on Linux runs the probe and leaks the handle on
     success (the sys crate's load then reuses it; a GTK-linked library is
     never `dlclose`d). Always `Ok` elsewhere.
   - `TrayBackendUnavailable` `Display`: the load errors plus a hint (install
     libayatana-appindicator3; for "undefined symbol" use .deb/.rpm/Flatpak or
     `LD_PRELOAD` the system GLib).
   - `close_request_action(label, tray_present)`: `Quit` for the main window
     when there is no tray, `Hide` otherwise.
2. `main.rs` startup: probe first; on failure log a warning and skip
   `setup_tray`. The dead `catch_unwind` is removed.
3. `main.rs` window close: without a tray, closing the main window calls
   `app.exit(0)` (explicitly, since hidden popup windows would keep the
   process alive). Quitting rather than minimizing because the tray menu is
   otherwise the only Quit control, a hidden window is unrecoverable, and
   "close = quit" is the norm for apps without a tray icon. Popups still hide.
4. Tray consumers: `rebuild_tray_menu` returns before building a menu when
   there is no tray; `TrayGraphManager::update_tray_graph_impl` returns before
   rendering. The manager itself stays managed: `submit_firewall_approval`,
   tray-stats settings and the settings preview take it as `State`.
   `update_tray_icon` and `apply_presentation` already no-op without a tray.
5. Flatpak: `packaging/flatpak/ai.localrouter.app.yml` builds
   libayatana-appindicator (+ intltool, libdbusmenu, ayatana-ido,
   libayatana-indicator), mirroring flathub/shared-modules
   `libayatana-appindicator-gtk3.json` (commit cb9ec602), inlined with its
   three patches vendored in `packaging/flatpak/patches/`.
   `scripts/publish-packages.sh` copies `patches/` next to the rendered
   manifest. GNOME 48 (gnome-build-meta `gnome-48` sdk/sdk-deps/core-deps)
   ships none of these libraries.

## Tests
- `ui::tray_support` unit tests: missing names → one error per name in order;
  empty list; fall-through to a present system library; name list mirrors the
  sys crate; message contains errors and hint; `Ok` off Linux; close decision
  table (main with/without tray, popups).
- Cross-target clippy (`-D warnings`) of `tray_support.rs` for
  `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu` in a scratch crate.
- Rendered the flatpak manifest with `publish-packages.sh --only flatpak`
  against dummy assets: new modules pass through unchanged, placeholders
  resolve, patches land next to the manifest; YAML parses.
- Source tarball sha256 values verified by download; git commits verified
  against the peeled tags with `git ls-remote`.
- Not verifiable here: Linux runtime behaviour and the flatpak-builder run
  (CI `build-flatpak` job).

## Final steps
1. Plan review — done: every tray consumer checked (`rebuild_tray_menu`,
   `update_tray_icon`, `set_update_available`, graph manager loop and
   `apply_presentation`, firewall/menu refresh in `main.rs`, commands holding
   `TrayGraphManager` state).
2. Test coverage review — done: decision logic and probe covered; the
   `main.rs` glue is two small branches over those functions.
3. Bug hunt — done: hidden popups no longer keep a tray-less app alive
   (explicit exit); probe flags and order match the sys crate; graph loop
   exits its active phase when no sources were initialised; no menu is built
   for a missing tray.
4. Commit (no push for this task).
