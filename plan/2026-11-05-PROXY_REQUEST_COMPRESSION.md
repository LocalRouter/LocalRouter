# Compressed native requests and secret redaction

- [x] Reproduce Codex's zstd-compressed Responses request through the actual proxy transport, using the upstream client's encoding behavior.
- [x] Decode supported HTTP content encodings with input/output/memory limits before active policy parsing, for MITM and reverse proxy requests.
- [x] Request identity response encoding explicitly; absence of Accept-Encoding permits compressed responses.
- [x] Preserve encoded bytes/headers for unchanged forwarding; remove stale content encoding, size and integrity headers after rewriting.
- [x] Cover compressed masking/restoration, compressed requests without secrets, nested encodings, chunked framing, malformed input, unsupported encoding, limits and passive transparency.
- [x] Allow Codex hosted-tool declarations alongside locally restored function tools; test the real AppFirewall and ensure all outbound inputs remain masked. Keep unsupported binary transports blocked.
- [x] Support verified default Codex freeform grammars and MCP tool namespaces, retaining rejection of unknown grammars and validating namespaced function calls without ambiguity.
- [x] Plan review: compare every planned behavior with implementation and close gaps.
- [x] Test coverage review: add missing transport and failure-path regressions.
- [x] Bug hunt: inspect framing, header propagation, bounds, parsing and monitoring.
- [x] Run relevant Rust tests, all-target lint and formatting with rustup stable; record actual proof and limits.
- [x] Commit/push signed changes with Matus identity and integrate using no-ff merges; retain unrelated work.
- [x] Validate the exact merged SHA in hosted release-validation CI.
- [x] Publish the corrective stable release after validation, monitor every enabled release and Docker job, and verify the live updater and artifact signatures.
- [x] Commit/push the release verification record.

The previous release's transport tests used uncompressed synthetic JSON. They did not cover Codex's actual zstd content encoding. This task closes that specific gap without claiming a paid/live model test. Never bypass redaction or send plaintext after a decoding error. Do not capture user secrets to diagnose the failure.

Evidence so far: upstream Codex rust-v0.124.0 codex-client request preparation uses zstd level 3 and Content-Encoding: zstd. The initial end-to-end compressed request failed before this change because ActiveInterceptor parsed encoded bytes as JSON and produced Value::Null. The new regression uses that upstream request serializer directly (test-only dependency), then sends its encoded body through CONNECT, TLS and chunked HTTP into the real interceptor/scanner/restore path.

The decoding contract includes zstd (bounded window), gzip, HTTP zlib/deflate and Brotli, at most four layers, with the existing 16 MiB body limit applied to every decoded layer. Invalid or unsupported encodings fail before upstream in active mode. Passive Forward preserves original encoded bytes, encoding headers and checksums. Rewritten bodies remove stale representation headers; response negotiation explicitly requests identity.

Codex metadata review also found the blanket hosted-tool guard and blanket custom-grammar guard blocked default web search, image-generation declarations, apply_patch, code-mode exec, and MCP namespaces. Tool declarations now coexist with redaction: all provider-facing JSON stays masked; originals are restored only on the local return. Known upstream Codex Lark grammars are pinned and recognized exactly, while arbitrary constrained grammars retain the explicit protection error. Namespaced function schemas remain distinct, including identically named functions in different namespaces.

## Local validation complete

Rustup stable 1.99.1 was verified current. `cargo test --workspace -j 3` passed on the final source: 113 executable/doc-test result groups and 4,993 executed cases (counts include the desktop lib and binary test targets). Focused components: scanner 103 passed (2 native-only ignored), proxy 90 unit + 10 MITM + 4 passthrough + 11 reverse transport cases, desktop 419 passed in each lib/bin target (5 intentionally ignored). Workspace all-target Clippy with `-D warnings` and `cargo fmt --all -- --check` passed.

One full-run MCP reconnect case (`recovery_retries_explicitly_expired_sessions_but_not_uncertain_writes`) failed once with "Server not running: reset". Its complete 28-test target passed on rerun, and both subsequent complete workspace runs passed. No reconnect implementation or fixture was changed. The failure log is retained, not counted as a green run.

Logs: /tmp/localrouter-request-compression/. Initial compressed regression failed before implementation; passing tests now use codex-client rust-v0.124.0's actual request encoder with local TLS upstreams. They do not exercise a paid live ChatGPT call. Automatic models.dev snapshot refresh and Cargo-generated lockfile updates are included under repository policy. No installed app was replaced or restarted.

Installed-client compatibility check: `/opt/homebrew/bin/codex --version` reports 0.166.0. Public rust-v0.166.0 sources were fetched read-only: http-client/src/request.rs still uses zstd level 3 and Content-Encoding: zstd; core/assets/tools/apply_patch.lark and the CODE_MODE_FREEFORM_GRAMMAR in core/src/tools/code_mode/execute_spec.rs match the pinned test fixtures exactly. Results are retained in /tmp/localrouter-request-compression/codex-0.166/conformance.json. This is source/wire-format conformance, not a live ChatGPT request.

## Corrective release validation

- Signed implementation commit: 2afb4e70; signed no-ff master merge: 08570e92f496bebfdc772e2b121437f0be2b58a5.
- Exact-SHA hosted CI run 42095731741 passed: workspace Rust tests, Clippy, formatting, frontend/unit/build and website checks. The source was unchanged during this run.
- Stable v0.0.176 release dispatched once after CI passed: run 42100716600. Tag 6b2b2c5b3a3b2e732cbd4cff2578f3be4fefc43b includes the validated merge. The complete diff from that source is exactly the four manifests/lockfiles with 0.0.175 changed to 0.0.176; no dependency or implementation changes were introduced by the version bump.
- Updated paper/docs are prepared locally for publication after the application release. The initial docs/static build passed; final publication and verification remain pending.

Application v0.0.176 was published at https://github.com/LocalRouter/LocalRouter/releases/tag/v0.0.176 at 2026-11-05T20:48:30Z. All five platform build jobs passed. Downloaded both macOS updater bundles, both Linux AppImages and the Windows NSIS installer and verified all five signatures with minisign using the application's configured updater public key; file sizes matched release metadata. The docs/static build passed after integrating the v0.0.176 version bump (33 prerendered public pages, 34 verified route outputs). Supplemental package jobs, Docker and live website verification are still being monitored.

## Final publication result

- Release workflow 42100716600 completed successfully: all five signed application targets, both Snap builds, both Flatpak builds, publication, and Nix pin update passed. The existing package-manager repository publishing job remains explicitly disabled (`if: false`) and was skipped.
- Latest-release API confirms stable v0.0.176 with 32 nonempty assets. The application's real `releases/latest/download/latest.json` endpoint serves v0.0.176 with all five platforms. All five downloaded updater artifacts passed cryptographic verification using the configured public updater key.
- Docker workflow 42106849400 succeeded. Public GHCR tags 0.0.176, v0.0.176 and latest share sha256:c09192fd9d0e5abce562d39c17bda61813f2348a62dd35b635c48db9ca7a17dd and include Linux amd64 and arm64 manifests.
- Website deployment 42107346706 at f8345cd6320a889d81207612433e80629eb39e95 succeeded. Fetched the public root, its bundled script, secret-scanning docs and research paper; verified the new zstd, MCP namespace and installed 0.166.0 conformance text.
- Plan review, test coverage review and bug hunt are complete. The corrective source and final verification record are committed and pushed with Matus Faro's signing identity, using no-ff merges and preserving unrelated root files. No failed enabled release jobs remain.

The wire proof uses local upstreams and synthetic credentials with the actual upstream request encoder. It does not claim a paid/live ChatGPT test, universal secret detection, or native Windows/Linux credential-store runtime verification. The application release is published; installing the update is a separate user action.
