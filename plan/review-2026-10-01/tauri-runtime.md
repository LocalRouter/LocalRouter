# Tauri runtime, launcher, updater, and integration-test review — 2026-10-01

## Scope and operating constraints

Second-wave ownership assigned by the root reviewer: `src-tauri/src/launcher`, `src-tauri/src/updater`, `cli.rs`, `main.rs`, `lib.rs`, `src-tauri/build.rs`, plus static inspection of integration tests/examples. This excludes `src-tauri/src/ui`, which another agent owns. No app launch, external config edits, CA trust changes, proxy launch, real credential access, inference, engine process, GPU, network download, or provider call was performed.

Every listed source was inventoried and scanned for startup/process/file-write/security boundaries. Substantive reads focused on launcher configuration preservation, filesystem backups, JSONC decoding, update policy, proxy lifecycle, command construction, marketplace skill installation, and bridge/startup wiring. Large main.rs and integration suites were sampled, not exhaustively executed or proved. Tests known to invoke model initialization/downloads were identified and excluded.

## Implemented changes

### Safe configuration updates and backups

`launcher/backup.rs` now creates both config replacement files and backups with owner-private Unix permissions from creation, writes/syncs before atomic persistence, and cleans temporary files on failure. Unique random backup suffixes prevent same-second overwrites of earlier recovery points, including files with identical basenames. A read error stops the edit rather than treating an unreadable file as absent. Identical-content writes return without replacing the inode. Pruning occurs after the config replacement succeeds.

The backup tests now inject a temporary backup directory, eliminating their previous write to the real user backup directory. Added regressions for successive versions, private permissions and refusal to overwrite unreadable destinations (represented by a directory). Promoted `tempfile` from dev-dependencies into normal dependencies in `src-tauri/Cargo.toml` because atomic replacement now uses it in production.

Eight launcher integrations previously parsed existing settings with `unwrap_or(empty settings)`, silently discarding user configuration when parsing failed. JSON and YAML parsing now uses checked shared helpers that require object/mapping roots; malformed/non-object input fails before a write. Updated Aider, Claude Code, Codex, Cursor, Droid, Goose, OpenClaw and OpenCode paths, including cleanup/undo paths. Codex/OpenCode config readers also propagate I/O errors. Comment-only YAML is accepted as an empty mapping. Pure fixtures cover malformed syntax, wrong root types, preserved unrelated settings, and empty YAML.

### JSONC correctness

The JSONC reader now rejects unterminated block comments instead of accepting a valid prefix or an apparently empty document and rewriting it. Removed comments become whitespace rather than concatenating neighboring tokens: `1/*comment*/2` is rejected instead of silently becoming `12`. Existing string escaping, URLs, comments, CRLF behavior, and trailing-comma handling remain. Strengthened the previous unterminated-comment test from “does not panic” to actual rejection.

### Marketplace callback confinement

Replaced the manual skill download/write callback in main.rs with `MarketplaceService::download_skill`, implemented by the MCP/marketplace agent. The callback now shares label/path validation, existing-symlink refusal, checked HTTP status, per-file and aggregate download limits. It returns the validated install directory for the existing config/rescan steps. This closes the duplicate unsafe installation path; tests of the shared path logic belong to the marketplace section. No skill was downloaded during this work.

### Update interval robustness

The update policy uses checked signed-to-unsigned conversion rather than narrowing `u64` intervals to `i64`. Very large intervals cannot wrap negative and immediately trigger updates, and future last-check timestamps do not cause a zero-interval check after clock rollback. Existing install-manager and manual/automatic behavior is preserved.

## Validation

An isolated temporary Cargo harness includes the exact repository `backup.rs`, `config_parse.rs`, `jsonc.rs`, and `dotenv.rs` modules without Tauri startup. Command:

```sh
env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-runtime-target rustup run stable cargo test --offline --manifest-path /private/tmp/localrouter-review-runtime-harness/Cargo.toml --lib
```

**37 tests passed, 0 failed**. All filesystem tests use temporary directories; all remaining cases use in-memory strings. Log: `/private/tmp/localrouter-review-runtime-tests.log`.

A second harness pass extracted the exact production `UpdateMode` and `InstallSource` declarations, `InstallSource` implementation, `UpdateCheckDecision` and `should_check_for_updates` function, plus the unmodified updater test module. It excluded background timers, app handles, filesystem detection and startup. The same Cargo command with the additional filter `updater_decision::tests` produced **11 passed, 0 failed**, including the oversized interval and one-second-future timestamp regressions. Log: `/private/tmp/localrouter-review-updater-tests.log`.

**Total: 48 passed, 0 failed.** The harness and its build artifacts live outside the repository. Source extraction validates the pure policy rather than whole-Tauri wiring; workspace compilation/Clippy validation is coordinated by the root reviewer. Changed sources were formatted, and `git diff --check` passed for owned files.

## Follow-ups and unchanged areas

- Proxy/ReverseProxy startup still has check-then-bind state transitions that merit per-manager/per-client lifecycle serialization under simultaneous UI calls.
- Several shell command snippets interpolate environment values without shell quoting. Values generated from ordinary loopback URLs/UUID secrets are currently narrow, but CA paths with spaces and user-configured URLs deserve a shared quoting layer; platform differences make a broad untested replacement inappropriate here.
- External tool configuration schemas/CLI commands were not checked against live upstream documentation or installed apps. This review fixes local preservation/error-handling bugs rather than claiming latest third-party compatibility.
- CA trust operations and system/provider relocation commands were reviewed statically only and never executed.
- The launcher backup directory still enforces a global last-10 policy, per existing behavior. Per-target recovery retention could be a product improvement.
- main.rs startup remains a large cross-component integration module; no full Tauri startup or OS integration test ran.
- CLI Clap argument restrictions, updater externally-managed-install gating, pure proxy configuration merge tests, and deliberate Tauri crate re-exports showed no established issue in the paths read.

## Integration-test safety observations

`routellm_fixes_verification.rs` has tests calling actual downloader functions, model `predict`, and model initialization when local files exist; `routellm_improvements_tests.rs` has a real model download retry path. Their names alone do not imply CPU-only safety. These were not run. Provider/MCP suites frequently use local fixture servers and temporary databases, but some system/launcher paths use environment-owned resources; only explicitly inspected filters may be run. Build/compile checks do not execute those tests.

## Exact runtime coverage inventory

D = changed/substantive targeted read; S = structural scan and selected call-path/declaration inspection. D is not a guarantee of whole-file line-by-line review.

| File | Lines | Coverage |
|---|---:|---|
| `src-tauri/build.rs` | 8 | S |
| `src-tauri/src/cli.rs` | 89 | D |
| `src-tauri/src/launcher/backup.rs` | 289 | D |
| `src-tauri/src/launcher/ca_bundle.rs` | 158 | D |
| `src-tauri/src/launcher/ca_trust.rs` | 277 | D |
| `src-tauri/src/launcher/integrations/aider.rs` | 248 | D |
| `src-tauri/src/launcher/integrations/claude_code.rs` | 319 | D |
| `src-tauri/src/launcher/integrations/codex.rs` | 250 | D |
| `src-tauri/src/launcher/integrations/config_parse.rs` | 52 | D |
| `src-tauri/src/launcher/integrations/continue_dev.rs` | 262 | S |
| `src-tauri/src/launcher/integrations/cursor.rs` | 219 | D |
| `src-tauri/src/launcher/integrations/dotenv.rs` | 165 | D |
| `src-tauri/src/launcher/integrations/droid.rs` | 216 | D |
| `src-tauri/src/launcher/integrations/goose.rs` | 314 | D |
| `src-tauri/src/launcher/integrations/jsonc.rs` | 282 | D |
| `src-tauri/src/launcher/integrations/mod.rs` | 221 | D |
| `src-tauri/src/launcher/integrations/openclaw.rs` | 371 | D |
| `src-tauri/src/launcher/integrations/opencode.rs` | 315 | D |
| `src-tauri/src/launcher/integrations/vscode.rs` | 160 | S |
| `src-tauri/src/launcher/integrations/zed.rs` | 96 | S |
| `src-tauri/src/launcher/mod.rs` | 120 | D |
| `src-tauri/src/launcher/proxy.rs` | 502 | D |
| `src-tauri/src/launcher/proxy_setup.rs` | 893 | D |
| `src-tauri/src/launcher/reverse_proxy.rs` | 250 | D |
| `src-tauri/src/launcher/reverse_setup.rs` | 1297 | D |
| `src-tauri/src/lib.rs` | 28 | D |
| `src-tauri/src/main.rs` | 3037 | D |
| `src-tauri/src/updater/mod.rs` | 358 | D |

## Integration-test inventory (static inspection only unless final validation explicitly says otherwise)

- `src-tauri/tests/access_control_tests.rs` — 418 lines; inventory/risk scan.
- `src-tauri/tests/audio_endpoint_tests.rs` — 1079 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/cache_integration_test.rs` — 187 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/client_auth_tests.rs` — 357 lines; inventory/risk scan.
- `src-tauri/tests/coding_agents_e2e_test.rs` — 594 lines; inventory/risk scan.
- `src-tauri/tests/completions_endpoint_tests.rs` — 457 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/debug_null_deserialize.rs` — 40 lines; inventory/risk scan.
- `src-tauri/tests/debug_value_id.rs` — 28 lines; inventory/risk scan.
- `src-tauri/tests/feature_adapter_integration_tests.rs` — 1057 lines; inventory/risk scan.
- `src-tauri/tests/image_edits_endpoint_tests.rs` — 306 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/mcp_auth_config_tests.rs` — 381 lines; network/listener or mocked HTTP references, keychain/environment references.
- `src-tauri/tests/mcp_bridge_tests.rs` — 366 lines; inventory/risk scan.
- `src-tauri/tests/mcp_client_capabilities_tests.rs` — 158 lines; inventory/risk scan.
- `src-tauri/tests/mcp_gateway_integration_tests.rs` — 244 lines; inventory/risk scan.
- `src-tauri/tests/mcp_gateway_mock_integration_tests.rs` — 2799 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/mcp_gateway_streaming_tests.rs` — 269 lines; inventory/risk scan.
- `src-tauri/tests/mcp_integration_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_notification_forwarding_tests.rs` — 253 lines; inventory/risk scan.
- `src-tauri/tests/mcp_stateless_http_tests.rs` — 258 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/mcp_tests/common.rs` — 635 lines; network/listener or mocked HTTP references, keychain/environment references.
- `src-tauri/tests/mcp_tests/concurrent_requests_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/error_scenarios_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/health_check_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/manager_lifecycle_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/mod.rs` — 55 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/oauth_client_tests.rs` — 325 lines; keychain/environment references.
- `src-tauri/tests/mcp_tests/oauth_server_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/proxy_integration_tests.rs` — 9 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/request_validation.rs` — 223 lines; inventory/risk scan.
- `src-tauri/tests/mcp_tests/sse_transport_tests.rs` — 172 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/mcp_tests/stdio_transport_tests.rs` — 412 lines; inventory/risk scan.
- `src-tauri/tests/memory_e2e_test.rs` — 259 lines; inventory/risk scan.
- `src-tauri/tests/metrics_integration_tests.rs` — 697 lines; inventory/risk scan.
- `src-tauri/tests/metrics_storage_tests.rs` — 493 lines; inventory/risk scan.
- `src-tauri/tests/metrics_tauri_commands_tests.rs` — 386 lines; inventory/risk scan.
- `src-tauri/tests/oauth_browser_integration_tests.rs` — 318 lines; network/listener or mocked HTTP references, keychain/environment references.
- `src-tauri/tests/openapi_tests.rs` — 431 lines; inventory/risk scan.
- `src-tauri/tests/permission_inheritance_tests.rs` — 1199 lines; inventory/risk scan.
- `src-tauri/tests/provider_integration_tests.rs` — 7 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/anthropic_tests.rs` — 86 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/bug_detection_tests.rs` — 363 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tests/cohere_tests.rs` — 90 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/common.rs` — 674 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tests/gemini_tests.rs` — 120 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/http_scenarios.rs` — 589 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tests/mod.rs` — 84 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/ollama_tests.rs` — 119 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/openai_compatible_detailed.rs` — 608 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tests/openai_compatible_tests.rs` — 280 lines; inventory/risk scan.
- `src-tauri/tests/provider_tests/request_validation.rs` — 213 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tests/sse_scenarios.rs` — 481 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/provider_tool_calling_tests.rs` — 252 lines; inventory/risk scan.
- `src-tauri/tests/route_helpers_tests.rs` — 426 lines; inventory/risk scan.
- `src-tauri/tests/routellm_fixes_verification.rs` — 222 lines; potential model/download execution.
- `src-tauri/tests/routellm_improvements_tests.rs` — 181 lines; potential model/download execution.
- `src-tauri/tests/router_routellm_integration_tests.rs` — 259 lines; inventory/risk scan.
- `src-tauri/tests/router_strategy_tests.rs` — 1732 lines; inventory/risk scan.
- `src-tauri/tests/server_stop_cancellation_tests.rs` — 286 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/skills_e2e_test.rs` — 503 lines; inventory/risk scan.
- `src-tauri/tests/stream_usage_endpoint_tests.rs` — 295 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/systemone_endpoint_tests.rs` — 735 lines; network/listener or mocked HTTP references.
- `src-tauri/tests/tauri_serialization_tests.rs` — 110 lines; inventory/risk scan.
- `src-tauri/tests/tool_calling_tests.rs` — 255 lines; inventory/risk scan.
- `src-tauri/tests/unified_api_tests.rs` — 444 lines; network/listener or mocked HTTP references.

## Example inventory

- `examples/feature_adapters.md` — static inventory; not executed.
- `examples/streaming-client-browser.html` — static inventory; not executed.
- `examples/streaming-client-example.ts` — static inventory; not executed.
