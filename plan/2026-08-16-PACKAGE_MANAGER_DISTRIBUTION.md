# Package Manager Distribution

**Date**: 2026-08-16
**Status**: Implemented (channels pending one-time account/repo setup)
**Scope**: Publish LocalRouter to free, no-fee package managers / app stores.

## Goal

Today LocalRouter is only installable by downloading a file from GitHub
Releases (or `docker pull`). Add first-class package-manager distribution so
users can run `brew install`, `winget install`, `yay -S`, `flatpak install`,
`snap install`, or `apt install` — with updates flowing through the same
channel they installed from.

Explicitly **out of scope** (cost money): Microsoft Store, Mac App Store,
Google Play, iOS App Store, Setapp.

Also skipped by decision: Chocolatey (redundant with WinGet), Nixpkgs
(community-maintained, low incremental reach for the effort).

## Channel decisions

| Channel | Gate | Automation | Notes |
|---|---|---|---|
| Homebrew (own tap) | none | CI, automatic | Official `homebrew-cask` needs 225★ for self-submission; repo has 25★ |
| WinGet | none | **manual PR** (by decision) | `LocalRouter.LocalRouter`, NSIS installer |
| Scoop (own bucket) | none | CI, automatic | Same repo as the tap |
| AUR (`localrouter-bin`) | none | CI, automatic | Repacks the `.deb` |
| Flathub | PR review | manual | Sandbox work required — see below |
| Snap Store | classic-confinement review | manual | Same sandbox reasons |
| APT/YUM repo | none | CI, automatic | Self-hosted on GitHub Pages |

### Why Homebrew uses our own tap

`docs/brew.sh/Acceptable-Casks` requires, for a **self-submitted** cask, one
of: 90 forks / 90 watchers / 225 stars. LocalRouter is at 25 stars / 7 forks,
so a PR to `Homebrew/homebrew-cask` would be closed on notability grounds.
A personal tap has no such gate and gives an identical UX after one
`brew tap`. Revisit the official cask once the repo clears 225 stars.

### Why Flatpak/Snap need real work

LocalRouter spawns **arbitrary host binaries**:

- `crates/lr-mcp/src/transport/stdio.rs:132` — MCP stdio servers (`npx`, `uvx`, …)
- `crates/lr-coding-agents/src/manager.rs:1110` — coding agents (`claude`, `aider`, …)

and it is a **tray-resident** app. Under a Flatpak/Snap sandbox both break.
Mitigations implemented here:

- `flatpak-spawn --host` wrapper for all subprocess spawns when running
  inside Flatpak (requires `--talk-name=org.freedesktop.Flatpak`).
- `--talk-name=org.kde.StatusNotifierWatcher` for the tray
  (see tauri-apps/tauri#13599).
- Snap uses `confinement: classic` — the same precedent as VS Code, which is
  a dev tool that must spawn host toolchains. Classic requires a manual
  review request on the Snapcraft forum.

## Architecture

### New: install-source awareness

The Tauri auto-updater must **not** fight the package manager. A
`brew upgrade --cask localrouter` and an in-app self-update targeting the
same `/Applications/LocalRouter.app` will corrupt each other's state.

- `crates/lr-utils/src/install_source.rs` — pure detection of how this
  binary was installed, plus `is_self_updatable()`.
- `crates/lr-utils/src/sandbox.rs` — `is_flatpak()` / `is_snap()` and
  `host_invocation()`, which rewrites a spawn to go through
  `flatpak-spawn --host --watch-bus` when sandboxed. Consumed by `lr-mcp` and
  `lr-coding-agents`. `crates/lr-utils/src/binary.rs` also routes binary
  *lookup* through the host, since `which` inside the sandbox reports every
  host tool as missing.
- `src-tauri/src/updater/mod.rs` — new
  `UpdateCheckDecision::ManagedExternally`, short-circuiting the timer.
- New Tauri command `get_install_source` so Settings can render
  "Managed by Homebrew — run `brew upgrade --cask localrouter`" instead of
  a dead "Check for updates" button.

Detection precedence (highest first) — **revised during implementation**, see
bug 3 below:
1. `LOCALROUTER_INSTALL_SOURCE` env var (packager override, always wins)
2. Live runtime signals: `APPIMAGE`, then `/.flatpak-info` / `FLATPAK_ID` /
   `SNAP`
3. `install-source` marker file, at `/usr/share/localrouter/install-source` on
   Linux and next to the executable elsewhere (written by the deb/rpm bundler
   config, the AUR PKGBUILD, Flatpak and Snap)
4. `/.dockerenv`
5. Path heuristics — `\scoop\apps\`, Homebrew `Caskroom`, Linux `/usr/bin`
6. `Direct` → self-update stays enabled (preserves today's behaviour for
   plain DMG/MSI installs)

Steps 2 and 3 are in that order because Tauri builds the AppImage from the deb
tree and the Flatpak/Snap recipes repack the same deb, so all three images
carry a marker file saying `deb`.

Detection is a pure function over injected inputs so it is unit-testable on
any host OS.

### Repository layout

```
packaging/
├── README.md                  # how each channel is published
├── homebrew/localrouter.rb.tmpl
├── scoop/localrouter.json.tmpl
├── aur/PKGBUILD.tmpl
├── winget/                    # 3 manifests + manual Komac instructions
├── flatpak/                   # manifest, metainfo.xml, .desktop
├── snap/snapcraft.yaml
└── linux-repo/                # apt + yum repo builders
scripts/publish-packages.sh    # renders templates from a release tag
```

Templates are rendered by `scripts/publish-packages.sh`, which resolves the
version, downloads release assets, computes SHA256s, and substitutes.
The same script backs both the CI job and manual runs.

### Release pipeline changes

`.github/workflows/release.yml`:
- add `rpm` to the Linux bundle matrix (needed for the YUM repo)
- new `publish-packages` job after `create-release`: brew tap, Scoop bucket,
  AUR, and the APT/YUM repo refresh
- WinGet deliberately excluded from CI — run Komac by hand

## Constraints and risks

- **GitHub Pages size**: the APT/YUM pool holds real `.deb`/`.rpm` payloads
  (~50 MB each). Pages soft-limits a site to 1 GB. At this repo's release
  cadence (v0.0.138 already) the pool must be **pruned to the last 3
  versions**, which the builder script enforces.
- **Unsigned Windows binaries**: `tauri.conf.json` has
  `certificateThumbprint: null`. WinGet accepts unsigned installers but
  SmartScreen will warn. Not fixed here (a cert costs money).
- **Not locally verifiable**: Flatpak, Snap, Scoop, and the AUR PKGBUILD
  cannot be built or installed from macOS. They are written to spec and
  must be validated on their target OS before first publish.

## Manual prerequisites (user actions, not done by this plan)

1. Create `LocalRouter/homebrew-tap` (public, holds the cask + Scoop bucket)
2. Create `LocalRouter/packages` (public, GitHub Pages, holds apt/yum repo)
3. Create the AUR package `localrouter-bin` and register an SSH deploy key
4. Fork `microsoft/winget-pkgs` under the LocalRouter org
5. Repo secrets: `PACKAGING_PAT` (cross-repo push), `AUR_SSH_PRIVATE_KEY`,
   `APT_GPG_PRIVATE_KEY`, `APT_GPG_PASSPHRASE`

## Implementation phases

- [x] **P1** — install-source + sandbox detection in `lr-utils`, with tests
- [x] **P2** — wire sandbox `host_command()` into `lr-mcp` + `lr-coding-agents`
- [x] **P3** — updater suppression + `get_install_source` command + frontend
- [x] **P4** — packaging templates (brew, scoop, aur, winget)
- [x] **P5** — `scripts/publish-packages.sh` renderer
- [x] **P6** — Flatpak + Snap manifests
- [x] **P7** — APT/YUM repo builders + `rpm` bundle target
- [x] **P8** — CI `publish-packages` job
- [x] **P9** — docs: `packaging/README.md`, root README install matrix

## Mandatory final steps

- [x] **Plan review** — re-read this plan against the implementation
- [x] **Test coverage review** — cover detection precedence + script rendering
- [x] **Bug hunt** — fresh-eyes pass for path/quoting/precedence bugs
- [x] **Commit** — only files touched by this work

## Bug-hunt findings (fixed)

Found during the mandatory fresh-eyes pass, each verified by reproducing it:

1. **Empty checksums shipped silently.** A missing release asset substituted an
   empty string, which the `__PLACEHOLDER__` guard could not see, producing a
   cask with `sha256 ""`. Missing assets are now fatal, and hashes are
   validated against `^[0-9a-f]{64}$`.
2. **`gpgcheck=1ABCDEF123`.** `${VAR:+1}${VAR:-0}` expands to `1` *plus the
   key id* when the variable is set. Replaced with an explicit if/else.
3. **AppImage misdetected as an apt install.** Tauri builds the AppImage from
   the deb tree, so the squashfs carries the deb's marker file. The marker was
   being read before the `APPIMAGE` env var, which would have disabled
   self-updates for the one self-updatable Linux format. Runtime signals now
   take precedence over the marker; the same applies to Flatpak and Snap,
   which repack the same deb.
4. **`<<<` in Flatpak/Snap build commands.** Both run under `/bin/sh`, where
   the bash here-string is a syntax error. Rewritten as `echo x | install`.
5. **GPG signing would have hung in CI.** The passphrase line was a no-op
   (`echo ... > /dev/null`) and no loopback pinentry was configured. Signing
   now feeds the passphrase on stdin with `--pinentry-mode loopback`.
6. **Empty AUR key produced an opaque ssh failure.** CI now fails fast with a
   clear message when the secret is unset.
7. **Duplicate `--env=PATH=`** when `ExecutionEnv` also sets PATH. The
   non-sandboxed path relied on last-wins; under Flatpak that became two
   conflicting flags. Now the shell PATH is only added when unset.

Also corrected: the `install_source` module doc still described the pre-fix
precedence order.

## Verification status

- `cargo test --workspace`: 2974 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --all -- --check`: clean
- `npx tsc --noEmit`: clean (the website's pre-existing
  `GuardrailApprovalDemo.tsx` error is untouched and unrelated)
- All nine manifests render and parse (JSON / YAML / XML / `ruby -c` /
  `bash -n`), with no unresolved placeholders
- Repo-pruning logic exercised end-to-end against a synthetic pool

Not verifiable from macOS, and flagged in `packaging/README.md`: building or
installing the Scoop, AUR, Flatpak and Snap packages, and running
`build-linux-repo.sh` against real `dpkg-scanpackages` / `createrepo_c`.
