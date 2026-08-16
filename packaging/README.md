# Package-manager distribution

How LocalRouter reaches each package manager, what is automated, and what a
human has to do.

See `plan/2026-08-16-PACKAGE_MANAGER_DISTRIBUTION.md` for the reasoning behind
these choices.

## Channels

Every channel publishes automatically from `.github/workflows/release.yml`.
The differences are only in *where* the release lands and whether a third
party reviews it afterwards.

| Channel | Serves | Publishes to | Third-party gate |
|---|---|---|---|
| Homebrew | macOS (arm64 + x64) | our tap (`LocalRouter/homebrew-tap`) | none |
| Scoop | Windows x64 | same repo as the tap | none |
| AUR | Arch (x86_64 + aarch64) | `localrouter-bin` | none |
| APT / YUM | Debian, Ubuntu, Fedora, RHEL | `LocalRouter/packages` (Pages) | none |
| Flatpak | Linux | our own flatpak repo, also in `LocalRouter/packages` | none |
| WinGet | Windows x64 | **manual PR** to `microsoft/winget-pkgs` (CI renders the manifests) | MS validation + moderator merge |
| Snap | Linux | GitHub release asset; Snap Store when credentialed | one-time classic review |

Everything is rendered by `scripts/publish-packages.sh`, which resolves the
version, hashes the published release assets and substitutes the `__NAME__`
placeholders. Run it without `--push` to preview:

```bash
./scripts/publish-packages.sh --version 0.0.139 --out-dir /tmp/pkg
```

It refuses to emit a file with an unresolved placeholder, and treats a missing
release asset as fatal — an empty checksum would otherwise sail through and
produce a manifest that fails verification on the user's machine.

### Flatpak: our own repo, not Flathub

Flatpak supports third-party remotes natively, so LocalRouter hosts its own —
the exact flatpak analogue of a Homebrew tap, with no review queue:

- `build-flatpak` (release.yml) runs `flatpak-builder` per architecture.
- `publish-packages` merges the builds into the OSTree repo at
  `LocalRouter/packages` → `flatpak/`, re-signing each commit with the repo
  GPG key via `flatpak build-commit-from`
  (`packaging/linux-repo/build-flatpak-repo.sh`).
- Users install once with
  `flatpak install --from https://packages.localrouter.ai/flatpak/localrouter.flatpakref`
  and `flatpak update` follows every release.
- A single-file `.flatpak` bundle is also attached to each GitHub release.

Submitting the same manifest to Flathub later is optional extra reach.

### WinGet: manual PR, by decision

No self-hosting option exists (WinGet "REST sources" require running a
server), so the channel is a PR to `microsoft/winget-pkgs`. Automating that
PR needs a classic PAT and a bot opening PRs on Microsoft's repo — rejected
by decision. CI renders the manifests as the `winget-manifests-<version>`
artifact; a human files them. See `winget/README.md`.

### Snap: the one genuinely gated channel

snapd is hardwired to Canonical's store — there is no third-party snap
repository, period. So `build-snap`:

- always attaches the built `.snap` to the GitHub release
  (`sudo snap install --dangerous --classic ./localrouter_*.snap`), and
- uploads to the Snap Store when `SNAPCRAFT_STORE_CREDENTIALS` is set.

The store holds the **first** classic-confinement upload until a one-time
[store request](https://forum.snapcraft.io/c/store-requests) is granted
(precedent: VS Code). After that, uploads flow unattended.

## Not used, and why

- **Microsoft Store / Mac App Store / Google Play** — all charge a developer
  fee. Out of scope by decision.
- **Official `Homebrew/homebrew-cask`** — a self-submitted cask needs 225
  stars, 90 forks or 90 watchers ([Acceptable Casks][cask]). The repo is well
  under that, so LocalRouter ships from its own tap. Revisit later; migrating
  is a one-line change for users.
- **Flathub** — not *needed* (see above); optional later for discoverability.
- **Chocolatey** — redundant with WinGet for the same audience, and adds
  another moderation queue.
- **Nixpkgs** — worth doing eventually, but Nix users are well served by the
  AppImage and the effort/reach ratio is poor compared to the above.

[cask]: https://docs.brew.sh/Acceptable-Casks

## One-time setup

Prerequisites for the automated jobs. Each channel skips (or degrades)
gracefully while its piece is missing, so these can be done incrementally.

1. **`LocalRouter/homebrew-tap`** (public). Holds `Casks/localrouter.rb` and
   `bucket/localrouter.json`. Users then run:
   ```bash
   brew install --cask localrouter/tap/localrouter
   scoop bucket add localrouter https://github.com/LocalRouter/homebrew-tap
   ```
2. **`LocalRouter/packages`** (public, GitHub Pages enabled, custom domain
   `packages.localrouter.ai`). Holds the APT pool, the YUM repos and the
   flatpak OSTree repo.
3. **AUR account** with `localrouter-bin` registered and an SSH key uploaded.
4. **GPG key** for signing the APT/YUM metadata and the flatpak repo.
5. **Snap Store** (optional until wanted): free Snapcraft account,
   `snapcraft register localrouter`, one classic-confinement store request,
   then `snapcraft export-login` → `SNAPCRAFT_STORE_CREDENTIALS`.

### Repository secrets

| Secret | Used for |
|---|---|
| `PACKAGING_PAT` | pushing to `homebrew-tap` and `packages` — a **fine-grained** PAT restricted to those two repos (Contents: read/write) is sufficient and preferred |
| `AUR_SSH_PRIVATE_KEY` | pushing to the AUR |
| `APT_GPG_PRIVATE_KEY` | signing `Release` / `repomd.xml` / flatpak commits |
| `APT_GPG_PASSPHRASE` | unlocking that key |
| `SNAPCRAFT_STORE_CREDENTIALS` | `snapcraft upload` (optional) |

Without the GPG secrets the job still succeeds but publishes an **unsigned**
apt/yum/flatpak repo, which forces users into `[trusted=yes]` /
`--no-gpg-verify`. Treat that as temporary.

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
  requires the one-time store approval; the precedent to cite is VS Code.

## Size budget (GitHub Pages)

`LocalRouter/packages` is served by GitHub Pages, which soft-limits a site to
1 GB. The APT/YUM pool is pruned to the last 3 versions
(`build-linux-repo.sh`) and the flatpak repo to the latest commit per ref
(`build-flatpak-repo.sh --prune-depth=1`, plus static deltas so fresh installs
are one download instead of thousands of object fetches). Rough steady state:
~500 MB apt/yum + ~200 MB flatpak. If Pages ever complains, lower
`KEEP_VERSIONS` first.

Note the *git history* of `LocalRouter/packages` still grows with every
release since binaries are committed; if the repo itself gets unwieldy,
squash it to a fresh orphan commit occasionally — nothing consumes its
history.

## Validation status

The Linux channels are now **exercised by CI itself** — `build-flatpak` runs
`flatpak-builder`, `build-snap` runs `snapcraft`, and `publish-packages` runs
the real `dpkg-scanpackages`/`createrepo_c`/`ostree` — so the first release
after setup validates them end-to-end. Still never executed on a real target
machine:

- the Scoop manifest (needs a Windows box)
- the AUR PKGBUILD (needs `makepkg` on Arch)
- *installing/running* the built flatpak and snap (CI only builds them; the
  snap's classic-confinement library setup in particular — WebKit helper
  paths, `enable-patchelf` — should be smoke-tested on a clean Ubuntu VM
  before the store request)
