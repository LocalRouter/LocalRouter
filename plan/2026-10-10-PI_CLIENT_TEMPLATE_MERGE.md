# Merge community PR #16: Pi (pi.dev) client template

PR #16 by ereli (issue #15) adds a Pi coding-agent client template: a
config-file integration writing `providers.localrouter` into Pi's
`models.json` and, under a sole-provider rule, `defaultProvider` /
`defaultModel` into `settings.json`.

## Progress / todo
- [x] Merge `refs/remotes/pr/16` with `--no-ff` (keeps contributor commits).
- [x] Resolve conflicts against master.
- [x] Drop unrelated changes (website Vite plugin, README run instructions).
- [x] Replace the redrawn logo with the official asset and record its source.
- [x] Harden `pi.rs` config writes; align with master's strict config parsing.
- [x] Keep Tauri types and the demo mock in sync (no frontend-visible command changes).
- [x] Plan review.
- [x] Test coverage review.
- [x] Bug hunt.
- [x] Commit (not pushed; the merge is handed back for review).

## Conflict resolution
- `README.md`: master's run instructions (`npm ci`, `cargo tauri dev --no-watch`)
  kept; the PR's alternative instructions are unrelated to Pi and dropped.
- `src-tauri/src/ui/commands_clients.rs`: the PR extracts the integration
  model list into `build_integration_model_list`, while master added
  routing-policy destinations to the inline version. The extracted function
  is kept and `configured_auto_models` now merges both: the auto model is
  listed only when `AutoModelConfig::has_chat_candidates()` (the same check
  the chat pipeline uses), followed by prioritized, available and
  routing-policy models, de-duplicated.

## Changes on top of the PR
- `website/vite.config.ts`: `resolveMainAppDepsFromWebsite` removed. CI and
  the Pages deploy run root `npm ci` before building the website, so the
  redirect is unnecessary and could resolve a different package version.
- Logo: `public/icons/pi.svg` is now the official https://pi.dev/logo.svg
  (identical to `logo-auto.svg`, which the upstream README embeds), copied
  unmodified and recorded in `public/icons/SOURCES.md`. The PR's file was a
  recoloured redraw. `website/public/icons/pi.svg` removed: the website's
  shared-icons plugin serves `public/icons`.
- `ServiceIcon`: `pi` matches only exactly (substring matching would give
  the Pi logo to `pinecone`, `openapi`, `copilot`, ...); the `π` symbol
  fallback removed in favour of the generic provider icon.
- `pi.rs`:
  - strict parsing via `config_parse::json`; both files validated before
    either is written; non-object `providers` is an error;
  - defaults are claimed only when LocalRouter owns them, or when unset and
    LocalRouter is the only custom provider (Pi's built-in providers are not
    in `models.json`, so the old rule overrode e.g. an `anthropic` default);
  - a user-chosen LocalRouter default model is kept while still offered;
  - unchanged files are not rewritten;
  - `PI_CODING_AGENT_DIR` honoured;
  - tests use a temp backup directory (the shared one is pruned to 10 files).
- `backup.rs`: `default_backup_dir()` and `write_with_backup_in()` exposed for
  the injectable-path writer.

## Final steps
- Plan review: every PR file was reviewed; the two shared behaviour changes
  kept from the PR (`configure_app_permanent` goes through `sync_config` for
  integrations that need a model list, so client modes are honoured; model
  lists are only built for gateway LLM mode and an empty list is an error
  rather than an empty provider) apply to OpenCode and OpenClaw as well.
- Test coverage: 16 `pi.rs` tests (sole/multi provider, built-in default,
  model refresh/preservation, idempotent re-sync, fallback model, unsync with
  and without files, malformed inputs on both paths, agent dir resolution) and
  two new `configured_auto_models` tests (no chat route, routing policy).
- Bug hunt: found and fixed the built-in-provider default override, the
  default-model reset on every sync, silent replacement of malformed files,
  the half-write when `settings.json` is malformed, and test backups landing
  in the user's backup directory.

## Follow-ups (not done)
- Pi's docs list a built-in `builtin:mcp` extension; the template keeps
  `supportsMcp: false` until its config format is checked.
- `launcher::integrations::tests::test_config_file_integrations_configure_permanent`
  (pre-existing) calls `configure_permanent` for other integrations against the
  developer's real home directory.
