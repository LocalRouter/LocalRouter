# Remove dead code hidden behind lint suppressions

Branch: `refactor/remove-dead-code`. Pure removal/refactor, no behaviour change.

## Scope

1. File- and crate-wide `#![allow(dead_code)]` in `crates/` and `src-tauri/src`:
   remove each, let rustc/clippy (`--all-targets`) report what is unused, then
   delete unused code or keep it with a narrow item-level
   `#[allow(dead_code)]` plus a one-line reason (serde-only fields, Debug-only
   fields, FFI, other-platform `cfg` branches). Items used only by tests become
   `#[cfg(test)]`.
2. Item-level `#[allow(dead_code)]`: review where cheap to decide.
3. Unused `pub` items of library crates (grep across the workspace), keeping
   anything that is deliberate public API (CLI, tests, OpenAPI, serde, config).
4. Tauri commands never invoked by the frontend
   (`node scripts/check-demo-mock-coverage.mjs`): delete only those without any
   consumer (frontend, tray, CLI, tests, winXP demo, events, docs). Remove all
   four locations (Rust command + `generate_handler!`, TS types, demo mock,
   docs).

## Rules

- Config safety first: never remove/rename a serde field, enum variant or
  config type that may appear in `settings.yaml` or another persisted file.
- Shared test-support modules under `tests/` (compiled into several test
  binaries) keep their `#![allow(dead_code)]`: each binary uses a subset.
- Do not edit `crates/lr-proxy`, `crates/lr-secret-scanner`, `crates/lr-monitor`,
  `src-tauri/src/launcher/proxy.rs` (concurrent work) beyond clearly dead items.
- Real bugs found are reported, not fixed (except one-line obvious fixes).
- Commit per crate / coherent group.

## Checklist

- [x] lr-utils, lr-api-keys, lr-router, lr-config
- [x] lr-monitoring (incl. `graphs`/`mcp_graphs`)
- [x] lr-oauth (`OAuthClientManager::load`, browser/*)
- [x] lr-clients (manager, token_store)
- [x] lr-providers (crate-wide allow, `key_storage::list_provider_keys`, oauth)
- [x] lr-mcp (manager, protocol, oauth, transport, gateway/*)
- [x] lr-server (state.rs)
- [x] src-tauri (updater, tray*)
- [x] Item-level allows review
- [x] Unused Tauri commands (all four locations)
- [x] Validation: fmt, clippy `--workspace --all-targets -D warnings`, per-crate
      tests, `localrouter --lib --bins`, frontend/website checks, demo-mock
      coverage
- [x] Plan Review: compare implementation against this plan, fill gaps
- [x] Test Coverage Review: confirm no test lost coverage of live code; tests
      only exercising deleted code are removed with it
- [x] Bug Hunt: re-read diffs for accidental behaviour change (removed side
      effects, removed trait impls used via dyn/generics, serde changes)
- [x] Commit and push

## Progress notes

Method: all 37 file/crate-wide `#![allow(dead_code)]` in `crates/*/src` and
`src-tauri/src` removed; all item-level allows temporarily disabled to see
which were load-bearing (127 of 139 were not). Unused `pub` items found with a
whole-workspace identifier count (declaration is the only occurrence,
including tests, benches, examples, strings and serde attributes), iterated
until no new ones appeared. Unused dependencies taken from cargo's
`unused_dependencies` lint.

Kept allows (each with a reason in the source):
- Required fields of deserialized API responses that are never read (OpenAI,
  llama.cpp, Jan, GPT4All, LocalAI, LM Studio, Cohere, GitHub Copilot's
  untagged token response): removing them would relax parsing.
- `FileUpload` / `ImageEditForm`: OpenAPI schema-only multipart types.
- `AppIntegration::name`: integration metadata asserted only by tests.
- Platform `cfg_attr` allows (untouched, plus `GraphConfig::windows_linux`).
- `#![allow(dead_code)]` in shared test-support modules under `tests/`.

Test-only items moved behind `#[cfg(test)]`: `CachedKeychain::clear_cache` /
`invalidate`, `McpViaLlmSession::new`, `KNOWN_TEMPLATE_IDS`, the single-pane
tray `generate_graph` path and `GraphConfig::macos_template`.

Tauri commands: 74 deleted (Rust, registration, TS types, demo mocks). Kept
because a plan describes the UI that would call them as not yet built:
`get_feature_endpoint_matrix` (2026-03-15 feature matrix), `reset_cost_backoff`
(2026-03-23 Reset button), `submit_coding_agent_approval`,
`get/set_coding_agent_approval_mode` (2026-03-13 deferred Phase 3 UI),
`clear_monitor_events`, `get_monitor_stats`, `set_monitor_max_capacity`
(2026-03-17 monitor header), `marketplace_get_pending_install`,
`marketplace_install_respond`, `marketplace_list_pending_installs`,
`marketplace_install_mcp_server_direct` (2026-02-03 install popup,
2026-10-10 bearer token), `create_strategy`, `delete_strategy`,
`create_test_client_for_strategy` (2026-02-10 revert path), `reload_config`
(2026-11-21 manual reload path).

Not done: `pub` items referenced only by tests (about 115, many deliberate test
hooks or used by `src-tauri/tests`) were left alone.

