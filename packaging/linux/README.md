# Linux package marker files

These one-line files are installed to `/usr/share/localrouter/install-source`
by the deb and rpm bundlers (see `bundle.linux.*.files` in
`src-tauri/tauri.conf.json`).

`crates/lr-utils/src/install_source.rs` reads the marker to tell an apt install
from a dnf install from an AUR install — at runtime those are indistinguishable,
since all three put the binary in `/usr/bin`. Knowing which one it is lets the
Updates settings tab print the right upgrade command.

The AUR PKGBUILD and the Snap/Flatpak recipes write their own value over this
one, because they repack the `.deb`. Note that a stale marker cannot cause a
misdetection anyway: AppImage/Flatpak/Snap are all identified by live runtime
signals that are checked *before* the marker.
