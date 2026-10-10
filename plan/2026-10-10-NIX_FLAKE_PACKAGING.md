# Nix flake packaging (GitHub issue #20)

## Problem

Issue #20 asks for Nix packaging. LocalRouter ships Homebrew, Scoop, AUR,
APT/YUM, Flatpak, Snap and WinGet, but nothing for Nix. (The reporter's
"Please build @vibe/local-web first" error comes from vibe-kanban and is
unrelated to LocalRouter.)

## Design

- **Binary repackage, not a source build.** Same strategy as the AUR,
  Flatpak and Snap recipes: unpack the released `.deb`
  (`LocalRouter_<ver>_amd64.deb` / `_arm64.deb`). A from-source Tauri build
  needs node plus a full npm install, which the Nix sandbox cannot do without
  vendoring the npm tree. `meta.sourceProvenance = [ binaryNativeCode ]`,
  `license = agpl3Plus`, `mainProgram = "localrouter"`.
- **Flake in this repository** (`flake.nix` at the root), so
  `nix run github:LocalRouter/LocalRouter` works with no third-party repo or
  review queue, the Nix equivalent of the Homebrew tap. Outputs:
  `packages.<system>.{default,localrouter}`, `apps.<system>.default`,
  `checks.<system>.localrouter`, `overlays.default` (`pkgs.localrouter`), for
  `x86_64-linux` and `aarch64-linux`. `flake.lock` pins nixpkgs
  (`nixos-unstable`) and is committed.
- **Derivation** `packaging/nix/package.nix`: `dpkg-deb -x`, then
  `autoPatchelfHook` against nixpkgs' WebKitGTK 4.1 / GTK 3 / libsoup 3 /
  glib / cairo / gdk-pixbuf / libgcc_s, and `wrapGAppsHook3` for GIO modules
  (glib-networking for TLS), GSettings schemas and GStreamer plugins (base +
  good; the Try-it-out speech panel plays and records audio through WebKit).
  The wrapper also prepends `libayatana-appindicator` to `LD_LIBRARY_PATH`
  (the tray `dlopen()`s `libayatana-appindicator3.so.1`, so autoPatchelf never
  sees it) and sets `LOCALROUTER_INSTALL_SOURCE=nix` with `--set-default`.
  The deb's `install-source` marker (`deb`) is dropped: it is only read from
  the absolute `/usr/share` path.
- **Install-source detection.** New `InstallSource::Nix` (`"nix"`): not
  self-updatable, label "Nix", upgrade command
  `nix profile upgrade LocalRouter` (the element name `nix profile install
  github:LocalRouter/LocalRouter` assigns). Detected from the wrapper's env
  override, or from an executable under `/nix/store/` (covers a third-party
  derivation that does not set the env). TS `InstallSource` union updated.
- **Release integration.** `nix` channel in `scripts/publish-packages.sh`
  renders `packaging/nix/sources.json.tmpl` (`__VERSION__`,
  `__SHA256_AMD64__`, `__SHA256_ARM64__`) with the same missing-asset and
  unresolved-placeholder guards as every other channel. Because the flake
  reads `master`, publishing means committing: the new `update-nix` job in
  `release.yml` (needs `version-bump` + `create-release`, `contents: write`,
  `GITHUB_TOKEN` only, independent of the disabled `publish-packages` job)
  renders the file and commits it to `master`. It skips prereleases, refuses
  to move the pin to an older version, and retries up to five times from the
  fresh tip if `master` moved, so every push is a fast-forward.
- Pinned now to **v0.0.153** (latest published release).

## Verification

All in a throwaway `nixos/nix` container (aarch64 host), worktree mounted
read-only and copied to a non-git `/work` so untracked files are visible:

- `nix flake lock`, `nix flake show`: all outputs evaluate.
- `nix build .#default`: autoPatchelf "0 dependencies could not be satisfied".
- `ldd result/bin/.localrouter-wrapped`: 0 "not found".
- Wrapper (makeBinaryWrapper) sets GIO_EXTRA_MODULES, XDG_DATA_DIRS,
  GST_PLUGIN_SYSTEM_PATH_1_0, LD_LIBRARY_PATH (ayatana) and
  LOCALROUTER_INSTALL_SOURCE=nix.
- `localrouter --version` → `localrouter 0.0.153`; `--help` works;
  `nix run .#default -- --version` works.
- Headless launch under `xvfb-run` + `dbus-run-session`: app starts, server
  listens, `GET /health` → `ok`, "System tray initialized successfully", and
  the process maps the ayatana libraries from the store.
- `nix profile add` of the flake → element named `LocalRouter`;
  `nix profile upgrade LocalRouter` succeeds.
- `nix flake check --all-systems`: all checks passed.
- x86_64: derivation evaluates; `nix store prefetch-file` of both debs gives
  SRI hashes equal to the pinned hex (converted with `nix hash convert`).
- `publish-packages.sh --only nix` on the downloaded debs renders a file
  identical to the committed `sources.json`; a missing asset is fatal.
- The `update-nix` commit loop was run against a local bare repo: bump,
  idempotent re-run, refusal to regress to an older version, and retry after
  `master` moved.
- `cargo fmt --check`, `cargo test -p lr-utils install_source` (21 pass,
  including two new Nix tests), clippy `-D warnings` on `lr-utils` and
  `localrouter` (all targets).

Not verified: an x86_64-linux build (an emulated build ran the Docker VM out
of disk and was abandoned), a real desktop session (tray menu interaction,
window rendering on a GPU), and the `update-nix` job on GitHub Actions itself.
The pinned 0.0.153 binary predates `InstallSource::Nix`: it ignores the `nix`
override and reports Direct (or Docker in a container) until the first
release containing this change, which `update-nix` pins automatically.

## Mandatory final steps

1. **Plan review** — done. Every design item above is implemented; the
   overlay and `checks` output were added as planned; docs in
   `packaging/README.md` (channel table, Nix section with flake-input usage,
   "Not used" and validation-status entries) and the main README install
   block.
2. **Test coverage review** — done. Rust: `nix_store_binary_detected_from_exe_path`,
   `nix_wrapper_override_beats_a_stray_deb_marker`, and Nix added to the
   not-self-updatable, has-upgrade-command, serde-name and updater
   managed-externally lists. Packaging: Docker build/run checks above; the
   release job's shell logic exercised against a local bare repo.
3. **Bug hunt** — done. Checked: unsupported systems throw a clear error;
   `meta.platforms` derives from the same arch map as the URLs; the pin can't
   regress under concurrent releases; prereleases don't reach flake users;
   the deb's stale `deb` marker is not shipped; `result` symlinks are
   git-ignored. Known trade-off: the wrapper's `LD_LIBRARY_PATH` prefix
   (ayatana only) is inherited by spawned MCP servers/agents; it contains
   nothing but the appindicator libraries, so it cannot shadow their deps.
4. **Commit** — done on `community/nix` (not pushed, per instructions for
   this task).
