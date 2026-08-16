# WinGet submission (automated)

WinGet has no user-hostable equivalent of a Homebrew tap — third-party
"REST sources" exist but require running a server — so `microsoft/winget-pkgs`
is the channel. The **submission is automated anyway**: every release,
`publish-winget` in `.github/workflows/release.yml` renders the manifests and
runs `submit-winget.sh`, which forks `winget-pkgs` under the token's account,
pushes a one-commit branch, and opens the PR. Microsoft's validation bots plus
a moderator take it from there; new versions of an already-accepted package
are routinely approved without discussion.

## One-time setup

1. Create a **classic** PAT with the `public_repo` scope on the GitHub account
   that should own the fork (fine-grained PATs cannot open PRs against repos
   they don't own).
2. Save it as the `WINGET_PAT` repository secret.

That's it — the fork of `microsoft/winget-pkgs` is created automatically on
first run. While `WINGET_PAT` is unset the job logs a notice and skips, so
releases never fail on it.

Expect the **first** PR (the "New package" one) to get moderator questions;
answer them on the PR. Everything after that is hands-off.

## Manual fallback

The same script runs locally:

```bash
./scripts/publish-packages.sh --version 0.0.139 --only winget --out-dir /tmp/pkg
WINGET_PAT=<token> ./packaging/winget/submit-winget.sh \
  --version 0.0.139 --manifests-dir /tmp/pkg/winget
```

Or hand the rendered files in
`/tmp/pkg/winget/manifests/l/LocalRouter/LocalRouter/0.0.139/` to
[Komac](https://github.com/russellbanks/Komac) / `wingetcreate` if you prefer
their validation.

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
