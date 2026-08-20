# WinGet submission (manual, by decision)

WinGet has no user-hostable equivalent of a Homebrew tap — third-party
"REST sources" exist but require running a server — so `microsoft/winget-pkgs`
is the channel, and getting in means opening a PR there.

Automating that PR was built and then **deliberately removed**: it requires a
classic PAT (fine-grained tokens cannot open PRs against repos you don't own)
and means a bot opening PRs on someone else's repository under your name.
Decision: no classic PATs, no auto-created PRs in other people's repos.

So CI renders the manifests every release and uploads them as the
`winget-manifests-<version>` artifact on the `publish-packages` job; a human
submits them.

## Per release

1. Download `winget-manifests-<version>` from the release run's summary page
   (or render locally:
   `./scripts/publish-packages.sh --version 0.0.139 --only winget --out-dir /tmp/pkg`).
2. Submit the `manifests/l/LocalRouter/LocalRouter/<version>/` directory to
   <https://github.com/microsoft/winget-pkgs> — either:
   - **Web UI, no token needed**: fork winget-pkgs on github.com, "Add file →
     Upload files" into `manifests/l/LocalRouter/LocalRouter/<version>/` on a
     branch, open the PR. Title it
     `New version: LocalRouter.LocalRouter version <version>`
     (`New package:` for the very first one) — the moderation tooling keys
     off that format.
   - Or [Komac](https://github.com/russellbanks/Komac) /
     [`wingetcreate`](https://github.com/microsoft/winget-create) if you're
     comfortable giving those your own token interactively.

Microsoft's validation bots check the manifests, then a moderator merges.
Expect questions on the first PR; new versions afterwards are routine.

## Known friction

- **The installer is unsigned.** `src-tauri/tauri.conf.json` sets
  `certificateThumbprint: null`, so SmartScreen will warn on first run and
  WinGet's validation will flag the missing signature. This is accepted, not
  overlooked — an OV/EV code-signing certificate costs money.
- **Install-source detection is best-effort on Windows.** A WinGet install and
  a hand-run NSIS installer land in the same directory, so the app cannot tell
  them apart and its self-updater stays enabled for both. To pin it, set
  `LOCALROUTER_INSTALL_SOURCE=winget` in the user environment. See
  `crates/lr-utils/src/install_source.rs`.
