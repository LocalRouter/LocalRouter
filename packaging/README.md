# Package-manager distribution

How LocalRouter reaches each package manager, what is automated, and what a
human has to do.

See `plan/2026-08-16-PACKAGE_MANAGER_DISTRIBUTION.md` for the reasoning behind
these choices.

## Channels

| Channel | Serves | Automated? | Source |
|---|---|---|---|
| Homebrew | macOS (arm64 + x64) | ✅ `release.yml` | `homebrew/localrouter.rb.tmpl` |
| Scoop | Windows x64 | ✅ `release.yml` | `scoop/localrouter.json.tmpl` |
| AUR | Arch (x86_64 + aarch64) | ✅ `release.yml` | `aur/PKGBUILD.tmpl` |
| APT / YUM | Debian, Ubuntu, Fedora, RHEL | ✅ `release.yml` | `linux-repo/` |
| WinGet | Windows x64 | ❌ manual | `winget/` |
| Flathub | Linux | ❌ manual | `flatpak/` |
| Snap Store | Linux | ❌ manual | `snap/` |

Everything is rendered by `scripts/publish-packages.sh`, which resolves the
version, hashes the published release assets and substitutes the `__NAME__`
placeholders. Run it without `--push` to preview:

```bash
./scripts/publish-packages.sh --version 0.0.139 --out-dir /tmp/pkg
```

It refuses to emit a file with an unresolved placeholder, and treats a missing
release asset as fatal — an empty checksum would otherwise sail through and
produce a manifest that fails verification on the user's machine.

## Not used, and why

- **Microsoft Store / Mac App Store / Google Play** — all charge a developer
  fee. Out of scope by decision.
- **Official `Homebrew/homebrew-cask`** — a self-submitted cask needs 225
  stars, 90 forks or 90 watchers ([Acceptable Casks][cask]). The repo is well
  under that, so LocalRouter ships from its own tap. Revisit later; migrating
  is a one-line change for users.
- **Chocolatey** — redundant with WinGet for the same audience, and adds
  another moderation queue.
- **Nixpkgs** — worth doing eventually, but Nix users are well served by the
  AppImage and the effort/reach ratio is poor compared to the above.

[cask]: https://docs.brew.sh/Acceptable-Casks

## One-time setup

Not yet done — these are prerequisites for the automated job to work.

1. **`LocalRouter/homebrew-tap`** (public). Holds `Casks/localrouter.rb` and
   `bucket/localrouter.json`. Users then run:
   ```bash
   brew install --cask localrouter/tap/localrouter
   scoop bucket add localrouter https://github.com/LocalRouter/homebrew-tap
   ```
2. **`LocalRouter/packages`** (public, GitHub Pages enabled, custom domain
   `packages.localrouter.ai`). Holds the APT pool and YUM repos.
3. **AUR account** with `localrouter-bin` registered and an SSH key uploaded.
4. **Fork of `microsoft/winget-pkgs`** under the LocalRouter org.
5. **GPG key** for signing the APT/YUM metadata.

### Repository secrets

| Secret | Used for |
|---|---|
| `PACKAGING_PAT` | pushing to `homebrew-tap` and `packages` |
| `AUR_SSH_PRIVATE_KEY` | pushing to the AUR |
| `APT_GPG_PRIVATE_KEY` | signing `Release` / `repomd.xml` |
| `APT_GPG_PASSPHRASE` | unlocking that key |

Without the GPG secrets the job still succeeds but publishes an **unsigned**
repo, which forces users into `[trusted=yes]`. Treat that as temporary.

## Manual channels

- **WinGet** — see `winget/README.md`.
- **Flathub** — open a PR to `flathub/flathub` creating `ai.localrouter.app`,
  using the rendered `flatpak/ai.localrouter.app.yml`.
- **Snap** — `snapcraft upload`, then request classic confinement on
  <https://forum.snapcraft.io/c/store-requests>.

CI renders all three every release and uploads them as the
`manifests-for-review-<version>` artifact, so submitting is a download-and-file
job rather than a re-derivation.

## The updater interaction

This is the part that is easy to get wrong. LocalRouter ships a Tauri
auto-updater that replaces the app bundle in place. If a package manager also
owns those files, the two fight: Homebrew records a version and checksum for
the bundle it placed, and an in-place self-update leaves the cask permanently
"outdated" while the next `brew upgrade` overwrites whatever the app installed.

`crates/lr-utils/src/install_source.rs` detects the owning package manager and
`src-tauri/src/updater/mod.rs` refuses to run the timer when one is found. The
Updates settings tab then shows the correct upgrade command instead of a dead
"Check for updates" button.

Detection order matters and is tested: live runtime signals (`APPIMAGE`,
`/.flatpak-info`, `$SNAP`) are checked **before** the
`/usr/share/localrouter/install-source` marker file, because Tauri builds the
AppImage from the deb tree and the Flatpak/Snap recipes repack that same deb —
so all three images contain a marker saying `deb`.

Two known gaps:

- **WinGet on Windows** is indistinguishable from a hand-run NSIS installer;
  both land in the same directory. The self-updater stays on for both. Set
  `LOCALROUTER_INSTALL_SOURCE=winget` to override.
- **`SystemPackage`** is the fallback when the binary is in `/usr/bin` but no
  marker is present (a third-party repackage). Self-updating is disabled, but
  no upgrade command is shown, since we cannot know which manager to name.

## Sandboxing (Flatpak and Snap)

LocalRouter spawns host binaries — MCP stdio servers (`npx`, `uvx`) and coding
agents (`claude`, `aider`) — and lives in the system tray. Neither survives a
default sandbox.

- **Flatpak** needs `--talk-name=org.freedesktop.Flatpak` so
  `crates/lr-utils/src/sandbox.rs` can proxy every spawn through
  `flatpak-spawn --host`, plus
  `--talk-name=org.kde.StatusNotifierWatcher` for the tray and
  `--filesystem=home` for agent configs and project directories. Binary
  *lookup* is also proxied — `find_binary` asks the host's login shell, because
  `which` inside the sandbox would report every tool as missing.
- **Snap** uses `confinement: classic`, so no rewriting is needed. Classic
  requires manual store approval; the precedent to cite is VS Code.

## Validation status

Written to spec but **not yet built or installed** on their target OS, because
this was authored on macOS:

- the Scoop manifest (needs a Windows box)
- the AUR PKGBUILD (needs `makepkg` on Arch)
- the Flatpak manifest (needs `flatpak-builder` on Linux)
- `snap/snapcraft.yaml` (needs `snapcraft` on Linux)
- `linux-repo/build-linux-repo.sh` end-to-end (needs `dpkg-scanpackages` and
  `createrepo_c`; its version-parsing and pruning logic *is* tested)

Validate each on its own platform before the first publish.
