# Tauri UI, native configuration and capabilities — 2026-10-01

## Scope and method

Second-wave ownership from the parent reviewer covered all files under `src-tauri/src/ui/`, `src-tauri/capabilities/`, and `src-tauri/tauri.conf.json`. Command exports, dangerous filesystem/process operations, panic markers, validation boundaries, event payload contracts, and selected lifecycle code were inventoried. Deep manual review focused on skill installation/deletion, approval edits, provider-secret handling, incremental model refresh contracts, reverse-proxy URL retargeting, local-model input validators, monitor commands, and native capability/CSP configuration. The file table below distinguishes selected semantic review from structural review. Native app/runtime behavior was not launched or claimed verified.

No command signature or serialized response shape changed; no new Tauri command was registered. Existing TypeScript/demo IPC contracts remain applicable. The shared crate install method was coordinated directly with the MCP/marketplace reviewer.

## Implemented findings

### U01 — Direct marketplace installs bypassed safe download handling

File: `src-tauri/src/ui/commands_marketplace.rs`.

`marketplace_install_skill_direct` previously built destination paths from untrusted source/name/file strings and duplicated raw reqwest downloads. It did not reject traversal, check unsuccessful HTTP status, or enforce body limits. The UI path now calls the marketplace reviewer's shared `MarketplaceService::download_skill(&listing)` API, which validates portable single-component labels, rejects symlink destinations, validates nested file paths, checks HTTP status and bounds downloads to 16 MiB per file / 64 MiB per skill. This removes the unsafe second implementation; crate-level tests belong to the marketplace report. No marketplace download was executed.

### U02 — Managed skill deletion used lexical path prefixes

Files: new `src-tauri/src/ui/skill_paths.rs`, `commands.rs`, `commands_marketplace.rs`, `mod.rs`.

A lexical `starts_with` accepted paths such as `skills/../outside`; a parent symlink could also lead outside the intended directory. Passing the managed root itself could remove all skills. The new helper resolves the root and target, requires strict descendant containment, rejects a symbolic-link target and requires a directory containing `SKILL.md`. User-created and marketplace classification/deletion now use the same rule and delete the validated canonical target. A missing/non-skill path returns a clear error instead of allowing a broad deletion. Existing persistent config removal stays scoped to the requested marketplace path.

Tests cover ordinary user/marketplace paths, root rejection, parent traversal, missing/non-skill directories, leaf symlinks and escaping parent symlinks. This is a pre-operation confinement check rather than a claim of race-free hostile-filesystem protection: another local process able to replace directories during validation/removal can still require OS-specific descriptor-relative operations.

### U03 — Invalid approval edits silently reverted to the original request

Files: new `src-tauri/src/ui/input_validation.rs`, `commands_clients.rs`, `mod.rs`, `src/views/firewall-approval.tsx`.

`serde_json::from_str(...).ok()` previously discarded malformed edited arguments and proceeded to allow the unedited payload. This is especially wrong when a user attempted to remove sensitive content. Allow actions now parse through a fallible helper before any permission/tracker changes; invalid edits return an error. Deny/block/disable actions ignore edits and remain available even when an editor contains malformed JSON. The frontend likewise only builds edit payloads for allow actions, avoiding an earlier local JSON.parse failure on Deny. Submission errors now appear inline while retaining approval/editor controls, allowing correction or denial instead of replacing the entire window with an error message.

Tests verify malformed/empty edits are rejected, absent edits remain absent, and valid modified arguments are preserved. Actual native approval windows and end-to-end backend waiting behavior were not exercised.

### U04 — Skill names/descriptions could corrupt YAML frontmatter

Files: `src-tauri/src/ui/input_validation.rs`, `commands.rs`.

The create-skill command inserted user-provided names and descriptions between raw double quotes. Embedded quotes, backslashes or line breaks could create invalid frontmatter or inject extra keys. The document helper now serializes each field as a JSON-quoted YAML-compatible scalar and preserves the body. Tests parse the generated YAML and verify exact round trips for quotes, backslashes and a description containing apparent frontmatter delimiters/extra keys; blank descriptions are omitted.

### U05 — Reverse-proxy retargeting corrupted IPv6 and URL suffixes

File: `src-tauri/src/ui/commands_reverse_proxy.rs`.

The old `rsplit_once(':')` logic treated the last colon of a bracketed IPv6 address or password as a port separator, and a URL with only query/fragment suffixes was parsed as part of its authority. Retargeting now isolates credentials, respects IPv6 brackets and splits suffixes at `/`, `?` or `#`. It retains the established behavior for empty/schemeless values and ordinary provider paths. Original and new regression tests passed as standalone extracted pure Rust functions. This function does not replace full upstream URL validation; malformed URL acceptance remains governed by existing provider/reverse-proxy validation.

### U06 — Generated speech audio blocked by CSP

File: `src-tauri/tauri.conf.json`.

The speech panel creates `blob:` audio URLs, but the native CSP had no media directive and inherited `default-src 'self'`, which does not authorize blob playback. Added `media-src 'self' blob:`. The script and network policies are unchanged. Configuration JSON was parsed successfully; actual native audio playback was not launched.

### U07 — Development server port could disagree with the native window

File: root `vite.config.ts` (also described in frontend report).

The Tauri development URL is fixed to `http://127.0.0.1:1420`. Vite now uses `strictPort: true`, so it reports a collision instead of selecting another port that the native window will not load.

## Verification and exact results

1. Standalone standard-library confinement tests:

   ```text
   rustc --edition 2021 --test src-tauri/src/ui/skill_paths.rs -o /tmp/localrouter-skill-paths-tests
   /tmp/localrouter-skill-paths-tests
   test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
   ```

2. Offline CPU-only input/path helper harness:

   A temporary Cargo package at `/tmp/localrouter-ui-input-tests` imports the actual repository helper modules via `#[path = ...]`, with only `serde_json = "1"` and `serde_yaml = "0.9"`. This avoids compiling or initializing native/model dependencies. Initial cargo invocation hit `sccache: Operation not permitted`; disabling the wrapper allowed the offline test to run.

   ```text
   RUSTC_WRAPPER= cargo test --offline --manifest-path /tmp/localrouter-ui-input-tests/Cargo.toml
   Finished `test` profile [unoptimized + debuginfo] target(s) in 26.32s
   running 6 tests
   test input_validation::tests::blank_skill_descriptions_are_omitted ... ok
   test input_validation::tests::malformed_edits_are_rejected_instead_of_discarded ... ok
   test input_validation::tests::skill_frontmatter_preserves_quoted_multiline_input ... ok
   test skill_paths::tests::accepts_user_and_marketplace_skills ... ok
   test skill_paths::tests::rejects_root_parent_escape_and_non_skill_directories ... ok
   test skill_paths::tests::rejects_symlink_targets_and_escaping_parent_symlinks ... ok
   test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
   ```

   The standalone harness emits three expected dead_code warnings for functions used only by its unit tests; production callers use them. It resolved cached compatible serde releases in its own temporary lockfile, not the repository lockfile. The repository's integrated build remains a separate validation layer.

3. Retargeting regression functions were copied verbatim from `commands_reverse_proxy.rs` into `/tmp/localrouter-retarget-tests.rs` and compiled using `rustc --test`:

   ```text
   test retarget_preserves_ipv6_credentials_and_query_suffixes ... ok
   test retargets_only_the_port ... ok
   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
   ```

4. Modified Rust files were formatted with `rustfmt --edition 2021`; all five capability/configuration JSON documents parsed successfully. `git diff --check` passed during implementation.

5. The desktop and website production builds passed TypeScript and Vite after these cross-layer changes; the final inline approval-error change is typechecked separately. Frontend tests and final build sizes/warnings are recorded in `frontend-website.md`.

These results do not substitute for a complete `cargo test -p localrouter` native application build. The parent agent owns aggregate Cargo validation and any environmental failures. No GPU, inference, browser, desktop window, model installation or real upstream interaction was used by this reviewer.

## Observations without additional changes

- Local-model path/model ID/import validators already reject parent components, nulls, relative imports and unsupported extensions as appropriate. Detailed remote/download implementation is owned by the models/engines reviewer; none was run.
- Provider secret-bearing `api_key`/`custom_headers` values are stripped before on-disk provider config is produced by the inspected helper; key migration/deletion errors are logged without printing the key itself.
- Monitor commands obtain running server state and return errors when it is unavailable. Event summary fields used by the frontend match the inspected Rust/TypeScript contracts.
- Incremental refresh emits started/provider/completed events and the frontend fix now registers before starting it. Failure events do not provide per-provider error details; broad event-contract redesign was avoided.
- Engine-install commands parse known recipe IDs and delegate supervision/installation to the engine subsystem. Capability files scope approval windows by their corresponding labels. Their larger default permission set was not narrowed speculatively without native interaction tests.
- Tray formatting already handles non-finite/negative values in compact output and has boundary tests. Tray graph/menu code received structural review and selected lifecycle reads rather than a native rendering audit.

## Remaining evidence gaps

- Native Tauri capability enforcement, real OAuth redirect/cancellation, focus timing, popup positioning, tray layout and actual media playback need an explicitly permitted interactive run.
- Rust command integration with the whole native crate is distinct from the isolated helper tests reported above.
- Managed deletion validation is not an OS-level atomic transaction; hostile concurrent filesystem replacement is outside demonstrated protection.
- Some command modules are thousands of lines long; structural inventory plus selected deep review is not exhaustive proof of every authorization/state transition.
- Provider-renaming secret migration is best-effort in existing code; stronger transactional rollback would require a coordinated registry/keychain/config redesign.
- Several version/probe commands launch subprocesses; the inspected coding-agent version command lacks a dedicated timeout. This was documented rather than broadened into a process-supervision rewrite.

## Per-file coverage

Legend: D = direct semantic inspection of relevant behavior; S = command/pattern/structure audit, with selected inspected snippets. All are first-party Rust/native configuration files.

| File | Lines | Coverage and focus |
|---|---:|---|
| `src-tauri/src/ui/commands.rs` | 5537 | D — skill creation/deletion, memory archive path handling, command boundaries |
| `src-tauri/src/ui/commands_clients.rs` | 4456 | D — approval edit validation and persistent-action ordering; selected client contracts |
| `src-tauri/src/ui/commands_coding_agents.rs` | 361 | D — selected session/version/lifecycle commands |
| `src-tauri/src/ui/commands_engines.rs` | 137 | D — recipe dispatch, installer and supervisor delegation |
| `src-tauri/src/ui/commands_free_tier.rs` | 358 | D — selected override/reset/usage persistence commands |
| `src-tauri/src/ui/commands_local_models.rs` | 1529 | D — local model/input/import validators and command inventory |
| `src-tauri/src/ui/commands_marketplace.rs` | 728 | D — direct installation, download reuse, deletion/classification |
| `src-tauri/src/ui/commands_mcp.rs` | 2427 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/commands_mcp_metrics.rs` | 296 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/commands_metrics.rs` | 485 | D — selected rate-limit/filter/graph command paths |
| `src-tauri/src/ui/commands_monitor.rs` | 127 | D — monitor store commands and state access |
| `src-tauri/src/ui/commands_providers.rs` | 1657 | D — secret storage/migration and incremental refresh contract |
| `src-tauri/src/ui/commands_reverse_proxy.rs` | 664 | D — binding lookup, upstream reachability, URL retargeting |
| `src-tauri/src/ui/commands_routellm.rs` | 220 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/input_validation.rs` | 70 | D — new pure parsing and YAML serialization helpers/tests |
| `src-tauri/src/ui/mod.rs` | 31 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/skill_paths.rs` | 101 | D — new path confinement helper/tests |
| `src-tauri/src/ui/tray.rs` | 695 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/tray_font.rs` | 393 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/tray_format.rs` | 187 | D — compact formatting and existing boundaries/tests |
| `src-tauri/src/ui/tray_graph.rs` | 1980 | S — structure and targeted pattern audit |
| `src-tauri/src/ui/tray_graph_manager.rs` | 2412 | D — selected state/event/render ownership paths |
| `src-tauri/src/ui/tray_menu.rs` | 1566 | S — structure and targeted pattern audit |
| `src-tauri/capabilities/default.json` | 95 | D — configuration/capability JSON review |
| `src-tauri/capabilities/elicitation-form.json` | 20 | D — configuration/capability JSON review |
| `src-tauri/capabilities/firewall-approval.json` | 21 | D — configuration/capability JSON review |
| `src-tauri/capabilities/sampling-approval.json` | 20 | D — configuration/capability JSON review |
| `src-tauri/tauri.conf.json` | 82 | D — configuration/capability JSON review |

Final source review also tightened U01's listing lookup: it now requires both the requested name and source URL to match. Previously a same-name skill from a different configured source could win the OR predicate and be installed instead of the listing selected in the UI. The frontend already sends both exact fields; no command/mock contract changed.

## Independent review of parent-owned fixes

At the parent's request, this reviewer then inspected the changed functions and relevant surrounding code in `crates/lr-api-keys/src/keychain_trait.rs`, `crates/lr-oauth/src/browser/callback_server.rs`, `crates/lr-engines/src/download.rs`, `crates/lr-local-models/src/download.rs`, `crates/lr-local-models/src/library.rs`, and `crates/lr-config/src/storage.rs`. This was read-only review; no edits or additional crate-test execution were performed by this reviewer.

- Found an incomplete concurrency fix in the library: although removal held the entries lock through deletion, import and completed-download insertion validated/read model files before acquiring that lock. They could validate, pause, then publish an entry after removal deleted the file. Reported this to the parent, who accepted it and is moving the lock before validation/candidate construction in both operations. Final implementation and regression results belong to the parent's core report.
- No newly introduced defect was identified in file-keychain persistence/publication order, cache-operation lock order, state/issuer validation before OAuth error delivery, orphan-strategy deduplication, owner-only exclusive config temp creation, or the local-model download check-and-insert critical section.
- Suggested a direct slow-cache-read versus store/delete/invalidate regression in addition to the existing concurrent-miss test. The locking implementation looked correct on inspection; this is a coverage suggestion rather than a demonstrated defect.
- The archive guard checks archive member components, with a fresh managed staging root assumed by its callers; it does not independently reject a symlink for the destination root itself. It also intentionally rejects duplicate archive writes to existing symlinks. Filesystem replacement races remain outside the demonstrated guarantee.
- Config write/sync failures can still leave their temporary file, an existing cleanup limitation; the new mode restricts that file to the owner on Unix.

After all frontend changes, the final CPU-only regression run passed **17 tests (15.8s)** and both desktop/website TypeScript checks exited 0. These results and all interactive/native verification limitations are retained in `frontend-website.md`.

Coordinator integration note: the independently identified model-library pre-lock validation race was resolved by acquiring the library index lock before import and downloaded-candidate validation. This joins validation, publication and deletion under the same operation lock.
