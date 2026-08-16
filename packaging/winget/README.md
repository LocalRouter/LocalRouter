# WinGet submission (manual)

WinGet is deliberately **not** automated in `release.yml`. Microsoft's
community repo runs an automated validation pipeline plus a human review, and
a bad automated submission burns goodwill on the `winget-pkgs` moderators. Run
it by hand and read the validation result.

## One-time setup

1. Fork <https://github.com/microsoft/winget-pkgs> under the `LocalRouter` org.
2. Install [Komac](https://github.com/russellbanks/Komac):
   `winget install RussellBanks.Komac`
3. Create a **classic** PAT with `public_repo` scope (fine-grained PATs are
   not supported) and export it as `GITHUB_TOKEN`.

## Per release

Render the manifests for the version you just shipped:

```bash
./scripts/publish-packages.sh --version 0.0.139 --only winget --out-dir /tmp/pkg
```

That writes the three manifests to
`/tmp/pkg/winget/manifests/l/LocalRouter/LocalRouter/0.0.139/` with the real
SHA256 of the published NSIS installer.

Then either submit those files directly, or let Komac regenerate them from the
release (it re-derives the installer metadata, which catches drift):

```bash
komac update LocalRouter.LocalRouter \
  --version 0.0.139 \
  --urls https://github.com/LocalRouter/LocalRouter/releases/download/v0.0.139/LocalRouter_0.0.139_x64-setup.exe \
  --submit
```

For the **very first** submission use `komac new LocalRouter.LocalRouter`
instead, and expect review questions.

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
