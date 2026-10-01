# LocalRouter comprehensive CPU-only review and improvement report

Date: 2026-10-01. Repository: `/Users/matus/dev/localrouterai`.

## Outcome and reading guide

This review implemented dozens of corrections across security boundaries, credential persistence, provider streaming, rate limits, metrics, MCP transports, tool execution, local file handling, desktop interactions, website behavior, launcher configuration, CI and packaging. The changes touch **122 source, test, configuration and existing documentation files**, plus the review artifacts. The full section reports are incorporated below, including before/after behavior, affected paths, regression evidence, inventories and remaining limitations.

The most consequential corrections are:

1. Caller-supplied trace headers can no longer suppress policy enforcement or usage accounting.
2. A client's Responses continuation cannot load another client's saved conversation.
3. File credentials and OAuth credentials are written privately and atomically, with failed saves preserving live state.
4. Archive extraction, skill reads/deletion and marketplace installs enforce stronger filesystem boundaries.
5. Thirteen completion provider adapters preserve UTF-8 across arbitrary stream chunks; Responses and Ollama framing also handle previously lost records.
6. Token/cost budgets participate in admission, multiple windows no longer multiply charges, and metrics rollups avoid duplicate totals without hiding unaggregated gaps.
7. MCP connections retain write ownership under concurrency and cancellation, pending requests clean up, and configured WebSocket authentication headers reach the server.
8. UI requests and listeners no longer overwrite newer client credentials, monitor selections, model refreshes or connection state in the corrected race paths.
9. Model job creation and library file ownership checks are synchronized; noisy skill scripts have bounded output and effective timeouts.
10. Malformed launcher configuration is rejected without silent replacement; packaging rejects destructive retention inputs and completes unsigned Flatpak metadata.

### Coverage honesty

The initial repository inventory contains **1,277 tracked files in 99 sections**, including **29 Rust library crates**, the Tauri shell, desktop React application, website, tests, workflows, packaging, scripts, documentation and assets. Every major first-party section received structural inspection and risk-oriented searches; changed behavior and selected high-risk paths received deeper semantic review. Section inventories distinguish those levels.

This is **not** a claim that every line of the approximately 434,000-line repository received a complete manual audit. Historical plans, generated code/data, binary assets, third-party code and generated website bundles were inventoried or sampled as appropriate. Numerical model behavior and GPU execution were excluded. Untouched source is not certified defect-free.

### Parallel execution

The session permits four concurrent agents. The coordinator and three subagents used all four slots, with disjoint source ownership and a second wave for the remaining areas:

| Reviewer | First wave | Second wave / integration |
|---|---|---|
| Coordinator | Configuration, credentials/OAuth, scanners, guards, local engines/models, compression, build and packaging | Cross-boundary security review, shared validation, independent integration review, report assembly |
| API/routing/providers | Server, router, all provider modules | Tauri runtime, launcher integrations, updater, test/example safety inventory |
| Frontend/website | Desktop UI, hooks, MCP client, website, frontend build/tests | Tauri command boundaries/capabilities and independent review of coordinator fixes |
| MCP/context/tools | MCP transports/gateway, tools, context, memory, skills, marketplace, coding agents, proxy, response sessions | Metrics, monitor, trace/types, utilities, clients, catalog |

Source review proceeded in parallel. Large Cargo jobs shared one isolated build directory and were sequenced to avoid compiler/cache collisions; many pure/Node/test-binary checks ran independently. No subagent made a separate commit.

## Important behavior changes

| Change | Practical effect |
|---|---|
| Untrusted trace hops reset at HTTP ingress | Legitimate multi-hop wire requests may now repeat transformations, approval prompts or accounting. Correlation IDs remain. Authenticated, request-bound provenance is required before safely restoring cross-process deduplication. |
| Token/cost limits are checked | Requests after an exhausted configured budget can now be rejected where they were previously admitted. In-flight reservation/overshoot remains a separate design problem. |
| Client-scoped Responses history | A foreign, missing or expired previous response ID follows the existing start-fresh behavior instead of exposing foreign history. |
| Rollup totals choose non-overlapping buckets | Usage totals may be lower because the same requests are no longer counted in multiple aggregation tiers. Historical partial buckets still follow existing timestamp semantics. |
| Invalid approval edits return an error | The UI keeps the approval open and displays the error, rather than silently approving the original unedited arguments. Denial remains possible. |
| Malformed launcher configuration is rejected | Users receive an actionable error; damaged existing files are preserved for repair. |
| Stricter filesystem/path validation | Ambiguous/traversing/symlinked managed paths and oversized marketplace payloads can now be rejected. Ordinary engine library symlink entries remain supported, but archive writes through them do not. |
| Credential-store tests are opt-in | Three real OS-keychain tests are ignored by default with explicit reasons. Mock/file tests remain automatic. |
| Fixed development ports | Vite fails clearly when a required development port is occupied, keeping its URL consistent with the native window. |

## CPU-only execution contract

Executed: source inspection, CPU compilation/linting, TypeScript checks and bundling, pure/local tests, temporary filesystem/SQLite fixtures, synthetic HTTP/WebSocket loopback servers and small fake-process fixtures.

Not executed: GPU device initialization, model inference, real model/engine downloads, real provider/OAuth exchanges, real credential-store tests, desktop/browser launch, coding-agent/package-manager startup, CA installation, external publishing/deployment, installer execution or pushing commits. Native and ML libraries were compiled as dependencies where required; compilation is not model or GPU execution.

All coordinated Rust builds used `LOCALROUTER_SKIP_CATALOG_FETCH=1`, `RUSTC_WRAPPER=`, `--offline`, and `CARGO_TARGET_DIR=/private/tmp/localrouter-review-target`. Final coordinated Rust checks use `rustup run stable` (Rust 1.99.0). The first MCP test build used the installed 1.98.1 toolchain; this is explicitly recorded, and late shared changes receive stable-toolchain validation. Existing installed npm dependencies were used locally; CI now uses `npm ci`.

The one pre-existing user change, `crates/lr-catalog/catalog/modelsdev_raw.json`, is excluded from review edits/staging. Its recorded SHA-256 is `db2c064f1f4489103eebe1ae62dcbf666de3b336720a8f5733daa186f99eee7d`; preservation is checked before handoff.

## Validation results

Suite counts represent their own executions and must not be added as a count of unique tests: some suites were rerun after later changes. No unfiltered whole-workspace test execution is claimed.

| Check | Final result | Scope / evidence |
|---|---|---|
| Core initial library run | **183 passed** | types 28, utils 51, json-repair 44, secret-scanner 28, monitor 32; later changed suites rerun below |
| OAuth callbacks and library | **38 passed** | Synthetic loopback; successful rerun after sandbox bind restrictions |
| Foundation libraries | **372 passed** | api-keys 17, config 127, engines 52, guardrails 58, local-models 88, secret-scanner 30; 4 intentionally ignored |
| Initial MCP/context/tools libraries | **800 passed** | Nine crates, zero failures, 3 intentionally ignored; initial Rust 1.98.1 build; see appendix filters |
| API/router/providers test compilation | **PASSED** | Three library targets compiled on stable 1.99.0; no tests automatically executed by this build |
| API/router/providers selected tests | **97 passed** | 55 provider pure/temp, 13 router, 12 server, 17 synthetic streaming fixtures; exact filters in appendix |
| Final shared libraries | **297 passed** | local-models 88, marketplace 29, monitor 33, monitoring 45, proxy 73, types 29; includes final lock/index/trace changes |
| Marketplace fixture lint correction rerun | **29 passed** | Same behavior after nonempty request-read assertion and initializer cleanup |
| Compression protection | **26 passed** | Standalone pure module, no model/GPU linkage or loading |
| Launcher/config helpers | **37 passed** | Exact production source included in isolated harness; temporary files and pure parsing |
| Updater pure policy | **11 passed** | Production enums/function and unchanged tests extracted without app/timers |
| Tauri UI pure helpers | **8 passed** | Six path/approval/frontmatter cases plus two retarget cases; earlier 3 path cases are included in the six |
| Frontend Node regressions | **17 passed** | Final run 15.8s, two workers, no browser/app; pure functions, temporary files and mocked SDK boundaries |
| Packaging regressions | **2 passed** | Two methods; retention test also covers four invalid values with repository preservation |
| Desktop frontend production build | **PASSED** | TypeScript and Vite; 4,071 modules |
| Website production build | **PASSED** | TypeScript and Vite; 6,662 modules |
| Final desktop/website TypeScript | **PASSED** | Both final checks exit 0, including inline approval errors |
| Workspace Clippy all targets | **PASSED** | Exit 0 after correcting new test-fixture lints; compiles native application and test targets without executing them |
| Rust formatting | **PASSED** | Whole workspace stable formatting check |
| Syntax/structure checks | **PASSED** | 4 workflow YAML files, 10 shell scripts, 3 parsed Python ML diagnostics, browser-example JS, Tauri JSON; diagnostic models not executed |
| Metrics query correctness/performance | **PASSED** | Exact production SQL: 3 correctness fixtures; 10k minute rows ~0.0022s, with 174 overlapping rollups ~0.0262s; illustrative local timing |
| Reverse-proxy TCP integration | **6 passed** | Six localhost dummy-upstream tests, including forged-hop reset and correlation |
| Catalog preservation | **PASSED** | Pre-existing user file matches recorded SHA-256 and is excluded from staging |

Existing non-failing warnings remain: upstream `block 0.1.6` future incompatibility, existing bundle chunk-size warnings and Browserslist freshness. Cache-only catalog messages are expected. No warning-free third-party dependency claim is made.

### Reproduction commands and local evidence

- **Core initial library run:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-types -p lr-utils -p lr-json-repair -p lr-secret-scanner -p lr-monitor --lib`. Evidence: `/private/tmp/localrouter-review-core-tests.log`.
- **OAuth callbacks and library:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-oauth --lib`. Evidence: `/private/tmp/localrouter-review-auth-loopback-tests.log`.
- **Foundation libraries:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-api-keys -p lr-config -p lr-guardrails -p lr-secret-scanner -p lr-engines -p lr-local-models --lib`. Evidence: `/private/tmp/localrouter-review-foundation-tests.log`.
- **Initial MCP/context/tools libraries:** `Inspected library test binaries, --test-threads=4; exact crate ledger in MCP appendix`. Evidence: `/private/tmp/localrouter-review-mcp-tests/*.log`.
- **API/router/providers test compilation:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-providers -p lr-router -p lr-server --lib --no-run`. Evidence: `/private/tmp/localrouter-review-api-build.log`.
- **API/router/providers selected tests:** `Direct execution of compiled test binaries with inspected filters and --test-threads=4`. Evidence: `See API appendix`.
- **Final shared libraries:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-types -p lr-monitoring -p lr-proxy -p lr-monitor -p lr-marketplace -p lr-local-models --lib`. Evidence: `/private/tmp/localrouter-review-final-shared-tests.log`.
- **Marketplace fixture lint correction rerun:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-marketplace --lib`. Evidence: `/private/tmp/localrouter-review-marketplace-final-tests.log`.
- **Compression protection:** `rustup run stable rustc --edition 2021 --test crates/lr-compression/src/protection.rs -o /private/tmp/localrouter-review-protection-tests; execute produced binary`. Evidence: `/private/tmp/localrouter-review-protection-tests.log`.
- **Launcher/config helpers:** `Stable cargo test --offline --manifest-path /private/tmp/localrouter-review-runtime-harness/Cargo.toml --lib`. Evidence: `/private/tmp/localrouter-review-runtime-tests.log`.
- **Updater pure policy:** `Same isolated harness, updater_decision::tests filter`. Evidence: `/private/tmp/localrouter-review-updater-tests.log`.
- **Tauri UI pure helpers:** `Source-backed isolated Rust helper harnesses; exact commands in UI appendix`. Evidence: `See UI appendix`.
- **Frontend Node regressions:** `npm run test:unit`. Evidence: `See frontend appendix`.
- **Packaging regressions:** `python3 -B -m unittest discover -s tests/scripts -v`. Evidence: `Recorded in coordinator report`.
- **Desktop frontend production build:** `npm run build`. Evidence: `See frontend appendix`.
- **Website production build:** `npm run build:check (working directory website)`. Evidence: `See frontend appendix`.
- **Final desktop/website TypeScript:** `npx --no-install tsc --noEmit; npx --no-install tsc --noEmit -p website/tsconfig.json`. Evidence: `See frontend appendix`.
- **Workspace Clippy all targets:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo clippy --offline --workspace --all-targets -- -D warnings`. Evidence: `/private/tmp/localrouter-review-clippy-final.log`.
- **Rust formatting:** `rustup run stable cargo fmt --all -- --check`. Evidence: `/private/tmp/localrouter-review-fmt-final.log`.
- **Syntax/structure checks:** `YAML parser, bash -n, Python ast.parse, Node vm.Script/JSON.parse and installed TypeScript parser`. Evidence: `Recorded in section reports`.
- **Metrics query correctness/performance:** `In-memory Python SQLite extraction of production SQL and index`. Evidence: `See foundations appendix`.
- **Reverse-proxy TCP integration:** `env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-proxy --test reverse_e2e`. Evidence: `/private/tmp/localrouter-review-reverse-proxy-tests.log`.
- **Catalog preservation:** `SHA-256 comparison against catalog-preservation.json`. Evidence: `plan/review-2026-10-01/catalog-preservation.json`.


### Why whole-workspace tests were not executed

The project guide normally requests `cargo test --workspace`. Inspection found tests/examples that can load or download actual models, including behavior not uniformly marked ignored. The user's explicit no-GPU constraint and the review's model-free execution scope take precedence. Instead, the review compiled selected test binaries, executed inspected CPU-only packages/filters, compiled all workspace targets through Clippy, and used isolated harnesses for pure Tauri helpers. This preserves broad useful validation without silently invoking model workloads.

Some sandboxed tests initially failed to bind localhost. Those were rerun with permission for temporary loopback fixtures, using synthetic data. These environment failures and their successful reruns are distinguished in the detailed reports. An initial frontend path assertion was corrected to compare canonical paths on macOS. Intermediate failures are not concealed or presented as final passing checks.

## Repository coverage map

| Section | Review and implemented outcome | Runtime evidence boundary |
|---|---|---|
| lr-api-keys | Private transactional persistence, cache-operation ordering, opt-in real keychain tests | Temporary/mock credentials only |
| lr-oauth | Flow-bound validated error callbacks, HTML escaping, prompt error delivery | Synthetic localhost callback tests |
| lr-config | Deduplicated missing-strategy healing, exclusive private temporary writes | Save/load/migration/config fixtures |
| lr-clients | CRUD/token lifecycle and failure sequencing inspection | No real secret rotation/account actions |
| lr-types | Trace trust boundary and task-local propagation | Pure trace and forged-boundary regressions |
| lr-utils | Paths, crypto, discovery/timeout contracts | CPU utility tests; platform-specific sampling |
| lr-catalog | Cache switch, generation/matching/pricing fallback boundaries | Cache-only build; snapshot preserved |
| lr-providers | Stream framing, OAuth storage, numeric validation, provider call sites | Pure and synthetic localhost fixtures; no upstreams |
| lr-router | Budget metrics, multiple windows, history ordering, cache expiry | Focused rate/cache tests |
| lr-server | Host/auth readiness, private request logging, shutdown tasks, ports, ownership, Unicode | Focused host/trace/audio/kill-switch tests |
| lr-monitoring | Deduplicated rollups with missing-interval handling | SQLite fixtures and library tests |
| lr-monitor | Reentrant emitter callbacks and event resource/lifecycle review | CPU event-store tests |
| lr-mcp | Transport authentication/concurrency/cancellation, broadcast errors, logging | Mock/local transport and gateway tests |
| lr-mcp-via-llm | Atomic mixed-tool claims and retained cancellation ownership | Mock orchestration tests |
| lr-context | Byte-bounded truncation, overflow-safe reads, linear long-line indexing | Pure/SQLite context tests |
| lr-memory | Session creation/expiry atomicity, nontruncating transcripts, Unicode IDs | Temporary memory/session fixtures |
| lr-skills | Resolved resource confinement, bounded output, effective timeout | Temp files and a synthetic sleep process |
| lr-marketplace | Shared validated/bounded download path and destination checks | Local HTTP/temp-filesystem fixtures |
| lr-coding-agents | Missed wakeups, newest-first listing, lock lifetimes | Process-free session tests |
| lr-proxy | Private CA key creation, boundary trace enforcement and accounting | Synthetic keys/local protocol fixtures |
| lr-responses-sessions | Owner-scoped previous response lookup | In-memory SQLite ownership tests |
| lr-secret-scanner | Overlapping detector keywords, Unicode masking | Synthetic secret strings only |
| lr-guardrails | Visible rejected-model configuration errors | Constructor/mock tests; no moderation model |
| lr-engines | Chained archive-symlink write protection | Tiny archives/fake engines; no real installation |
| lr-local-models | Atomic job admission; synchronized library validation/deletion | Tiny generated GGUF headers/loopback fixtures |
| lr-compression | Fenced-code protection parity and lifecycle review | Standalone pure protection module only |
| lr-embeddings | Downloader/model lifetime/tokenization/pooling inspection | Static and compilation; no model quality claim |
| lr-routellm | Initialization/prediction/unload/downloader/test safety review | Static and compilation; no prediction benchmark |
| lr-json-repair | Parser/stream state structural inspection | Existing pure repair tests |
| Tauri runtime | Safe backup/config parsing, shared marketplace callback, updater intervals | Source-backed isolated pure helpers and compilation |
| Tauri UI/capabilities | Managed skill paths, approval validation, YAML quoting, URL retarget, audio CSP | Pure helpers/JSON checks/type/build; no GUI |
| Desktop frontend | Client ownership, async ordering, event/listener/resource cleanup, SSE | Node-only mocked/pure regression suite and production build |
| Website | Theme/demo event lifecycle, URL handling, icon path boundary | Node fixtures, TypeScript and production bundle |
| CI/release/packaging | Reproducible tested website revision, frontend CI, safe retention, unsigned Flatpak | YAML/shell parsing and fake-tool packaging fixtures |
| Docs/examples/scripts/assets | Setup corrections, archival guidance, safer legacy DOM construction; inventory | Syntax/static checks; no historical compatibility claim |

## Prioritized remaining work

These are bounded follow-up observations, not claims that the corresponding problems were fully resolved in this pass. Detailed section reports provide context and additional smaller items.

| Priority | Follow-up | Evidence needed before implementation/release |
|---|---|---|
| High | Design authenticated, request-bound multi-hop provenance | Threat model for replay and confused-deputy behavior; end-to-end multi-hop tests |
| High | Add protocol-aware limits for streaming frames, long-lived maps and approval histories | Legitimate large tool payload fixtures, cancellation/resource stress tests |
| High | Make marketplace/model multi-file installs transactional | Staging/promotion manifest, failed/cancelled download fixtures, rollback behavior |
| High | Reserve concurrent budget usage at admission | Clear estimated/actual usage contract, concurrent streaming/refund tests |
| Medium | Stronger filesystem race resistance and expanded-archive quotas | Directory-relative/no-follow traversal design, hostile mutation/large archive fixtures |
| Medium | Serialize overlapping server/proxy lifecycle transitions | Start/stop/rebind/restart stress fixtures and shutdown ownership contract |
| Medium | Improve model worker scheduling and cancellation | Explicitly authorized model runs and CPU/GPU performance measurements |
| Medium | Validate mounted UI/Tauri workflows across platforms | Interactive/browser/native tests for approval focus, dynamic identity changes, media playback, window lifecycle |
| Medium | Harmonize OAuth token endpoint encoding and resource limits | Form-encoded interoperability and bounded attempt-map tests |
| Medium | Strengthen metrics backfill and partial-window reporting | Downtime/retry fixtures and explicit retained-bucket semantics |
| Medium | Clarify catalog matching ambiguity and dependency/advisory health | Deterministic provider fixtures and a separately authorized online dependency assessment |
| Medium | Validate platform packaging, native keychain and real provider contracts | Isolated Windows/Linux/macOS environments and opt-in live fixtures |

No dependency versions, live pricing/catalog data or public software advisories were refreshed. This was a local source and behavior review; it is not a third-party dependency audit or a claim of vulnerability-free software.

## Supporting artifacts

- `plan/2026-10-01-CODEBASE_REVIEW.md`: working plan and completion checklist.
- `plan/review-2026-10-01/repository-inventory.json`: baseline tracked files grouped into sections.
- `plan/review-2026-10-01/coverage-inventory.json`: tracked-file coverage classifications and sizes.
- `plan/review-2026-10-01/frontend-inventory.json`: TypeScript parser and structural frontend inventory.
- `plan/review-2026-10-01/catalog-preservation.json`: excluded user change and preservation hash.
- `plan/review-2026-10-01/change-manifest.json`: changed/new review files, excluding the pre-existing catalog edit.
- `plan/review-2026-10-01/validation-results.json`: final machine-readable validation summary.
- Seven detailed section reports, incorporated in full below so this file can be read independently.

Raw local build logs live under `/private/tmp/localrouter-review-*` as recorded in the validation evidence. Temporary logs and build artifacts are not committed. The report and machine-readable summaries preserve their essential outcomes after those temporary files disappear.

## Detailed section reports

The following appendices reproduce the section reports with their own inventories and detailed findings. The final validation table above is authoritative for later integration reruns; an explicitly historical first-wave result below remains labeled as such.


---

## Appendix 1: Configuration, credentials, local models, scanning, build and packaging review

Source report: `plan/review-2026-10-01/security-storage-models-build.md`.


### Scope and method

Coordinator-owned review of `lr-api-keys`, `lr-oauth`, `lr-config`, `lr-secret-scanner`, `lr-guardrails`, `lr-engines`, `lr-local-models`, `lr-compression`, `lr-embeddings`, `lr-routellm`, build workflows, packaging, scripts, examples and root documentation. Initial supporting-library inspection covered `lr-types`, `lr-utils`, `lr-json-repair`, `lr-monitor`, `lr-monitoring`, `lr-clients` and `lr-catalog`; the MCP reviewer performed the second wave for these foundations.

The review combined manifests and source inventories, risk-oriented searches, detailed reads of persistence/authentication/download/validation paths, existing test inspection, targeted fixes and focused regressions. Generated model data, binary assets, generated Tauri schemas and historical plans were inventoried rather than exhaustively hand-audited. This is not a claim that every line of this approximately 434,000-line repository was deeply reviewed.

No GPU device was initialized, no inference model was loaded, no real model or inference-engine download was performed, and the desktop application was not launched. Download tests use tiny fixtures served over loopback. The compression protection tests compile the standalone pure Rust module without linking Candle or GPU libraries.

### Implemented changes

#### 1. Private, atomic file-keychain persistence

**Files:** `crates/lr-api-keys/src/keychain_trait.rs`.

Before: file-keychain writes used ordinary `fs::write` on a predictable temporary name, inherited ambient file permissions, and explicitly removed the destination on Windows before replacement. Secrets could be readable beyond the file owner; failure after removal could lose the previous file. Mutation was published in memory before persistence succeeded.

After: same-directory `NamedTempFile` provides exclusive creation and private Unix permissions. Contents are written, synced and atomically persisted. A complete candidate map is persisted while holding the storage mutex, and only then becomes the live map. A failed store/delete leaves the previous in-memory state intact. Temporary files are cleaned up on failure.

Regression coverage verifies failed replacement preserves existing secrets, failed insert does not create a visible secret, failed delete retains the secret, no temporary secret file remains, and both fresh and replacement files have mode `0600` on Unix.

#### 2. Serialize underlying keychain operations with cache publication

**Files:** `crates/lr-api-keys/src/keychain_trait.rs`.

Before: a slow cache miss could fetch an old value, race a store/delete, and repopulate the cache after the newer operation completed. Concurrent misses could also repeat operating-system credential prompts.

After: a shared operation mutex orders cache misses, writes, deletes and explicit invalidation. Cache hits retain the fast read path. A miss rechecks the cache after acquiring the operation mutex. Eight simultaneous synthetic readers verify one underlying lookup and consistent results.

#### 3. Make real credential-store tests opt-in

**Files:** `crates/lr-api-keys/src/keychain.rs`.

Three tests directly create/delete entries in the real operating-system credential store. They now have explicit ignore reasons and require an intentional `--ignored` invocation. File-backed and mock-keychain tests remain automatic. No real credential-store tests were executed during this review; an initial broad build was interrupted before test execution when these tests were identified.

#### 4. OAuth error callbacks require a matching flow and escape HTML

**Files:** `crates/lr-oauth/src/browser/callback_server.rs`.

Before: the error path returned before validating `state`, interpolated provider-controlled error strings directly into HTML, and did not notify the pending flow. A browser could display injected markup while the application kept waiting for timeout.

After: both success and error callbacks pass state and issuer checks. A validated provider error completes the matching flow with an error, and HTML interpolation escapes all special characters. Invalid states leave the legitimate pending flow intact. Cancellation documentation now matches the implemented server shutdown.

Regression coverage uses a temporary local callback listener and synthetic query strings to verify invalid-state isolation, escaped script/image markup and immediate error delivery. All OAuth library tests pass when localhost binding is available.

#### 5. Heal a shared missing strategy only once

**Files:** `crates/lr-config/src/storage.rs`.

Before: the known-strategy set was built once and never updated during healing. Two clients referencing the same absent strategy caused two duplicate strategy records to be created, after which validation rejected recovery.

After: newly encountered IDs are inserted into the set during traversal, so each absent strategy is created once. A full save/load fixture confirms both client references survive and the resulting configuration validates.

#### 6. Create configuration temporary files privately and exclusively

**Files:** `crates/lr-config/src/storage.rs`.

Before: temporary YAML files were created with normal file permissions, and restrictive permissions were applied only after publication. Creation also allowed an existing temporary path to be truncated.

After: `OpenOptions::create_new(true)` creates each temporary file exclusively; Unix mode `0600` applies from creation. Existing final-file restrictive permissions remain. This improves the secret-bearing configuration write path without changing its schema.

#### 7. Prevent chained-symlink writes during engine archive extraction

**Files:** `crates/lr-engines/src/download.rs`.

Before: lexical checks rejected direct `../` paths and direct escaping symlinks but could miss chains of individually permitted symlinks. For example, `a -> .`, `b -> a/..`, then `b/escaped` can resolve outside the extraction directory.

After: ZIP and tar.zst extraction reject members whose destination components traverse existing symlinks, including symlinks from earlier entries or an extra archive. Ordinary library symlink entries remain supported; writing through them is rejected. Existing target symlinks cannot redirect a file overwrite.

Regressions cover the chained attack in both ZIP and tar.zst and preservation of a pre-existing file outside the destination. These operate only on tiny generated archives and temporary files.

#### 8. Expose guardrail model configuration failures

**Files:** `crates/lr-guardrails/src/engine.rs`.

Before: missing providers, incomplete model configuration and unknown model types were only logged; the advertised `load_errors` list stayed empty. The Tauri UI already consumed this list to emit model-load failures, so configured guards could silently disappear from that reporting path.

After: each rejected model populates `load_errors` with its model ID and actionable reason. A constructor-only test exercises all three cases without executing any moderation model or HTTP request. Runtime allow/ask/deny policy is otherwise unchanged.

#### 9. Atomically deduplicate local-model download jobs

**Files:** `crates/lr-local-models/src/download.rs`.

Before: overlap checking and job insertion occurred under separate lock acquisitions. Concurrent starts could both pass the check and write the same partial and final files.

After: overlap/deduplication checks and insertion share one critical section. Existing jobs resume after releasing the lock. Repeated selected paths are deduplicated once before admission. A loopback fixture launches 16 concurrent starts and verifies one ID and one job; no real model is downloaded.

#### 10. Preserve model files shared through equivalent paths

**Files:** `crates/lr-local-models/src/library.rs`.

Before: deletion used a stale snapshot of remaining entries after releasing the index lock, and sharing comparisons used raw paths. Concurrent additions or equivalent paths containing `.` could leave an active entry pointing at a deleted file.

After: the index lock spans the ownership check and deletion. Import and downloaded-entry insertion acquire the same lock before validating files, so a validated candidate cannot wait for a deletion and then publish a missing path. The latter gap was found and closed during independent review. Sharing comparisons use canonicalized paths. A fixture verifies a downloaded file remains when an imported entry references the same file through an equivalent path.

#### 11. Detect secrets with overlapping keyword prefixes

**Files:** `crates/lr-secret-scanner/src/regex_engine.rs`.

Before: non-overlapping Aho-Corasick iteration could find the short `sk-` keyword and omit the longer `sk-proj-`, `sk-ant-` or `sk-ant-api03-` rules. Specific modern key formats could pass without their intended detector running.

After: overlapping keyword iteration considers every matching rule prefix. Regressions verify all three formats using synthetic high-entropy strings.

#### 12. Mask Unicode secret previews without panicking

**Files:** `crates/lr-secret-scanner/src/regex_engine.rs`.

Before: preview generation sliced six prefix bytes and four suffix bytes directly. Generic rules can match non-ASCII passwords, making those byte positions invalid UTF-8 boundaries.

After: preview masking uses character counts and character boundaries. Regressions cover short Unicode values, mixed-width masked content and long emoji strings. ASCII masking semantics are retained.

#### 13. End a self-contained fenced-code region correctly

**Files:** `crates/lr-compression/src/protection.rs`.

Before: a word containing both opening and closing fences toggled fenced state only once, unnecessarily protecting all later prose from compression.

After: the number of fence delimiters determines the state transition. A regression checks prose before and after a self-contained fenced word. All 26 pure protection tests pass without loading a model or linking the GPU runtime.

#### 14. Add frontend, website and script validation to CI

**Files:** `.github/workflows/ci.yml`.

A separate CPU validation job installs pinned dependency sets, executes Node-only frontend regressions, executes packaging script regressions, builds the desktop frontend and typechecks/builds the website. The workflow now declares read-only repository permissions and cancels superseded runs for the same ref. Existing Rust checks remain.

#### 15. Build and deploy the tested website revision reproducibly

**Files:** `.github/workflows/deploy-website.yml`.

Automatic deployment follows successful push CI runs, checks out the triggering commit SHA, uses the repository Node version and both lockfile caches, installs with `npm ci`, and uses `build:check`. Previously it checked out the current default branch, used Node 18 and unpinned installs, and omitted TypeScript validation. Manual deployment remains available. No deployment was executed.

#### 16. Reject destructive packaging retention settings before mutation

**Files:** `packaging/linux-repo/build-linux-repo.sh`, `tests/scripts/test_packaging.py`.

Before: `--keep 0` produced an empty retention set and could delete every release, including the release just staged. Arbitrary version input also reached filenames and pruning logic.

After: the script validates a positive integer retention count and the expected bare semantic version before staging/pruning. Regressions verify `0`, negative, fractional and textual retention inputs fail while a temporary repository remains unchanged.

#### 17. Finish unsigned Flatpak metadata generation

**Files:** `packaging/linux-repo/build-flatpak-repo.sh`, `tests/scripts/test_packaging.py`.

Before: an optional GPG-key test was the last command of a redirected command group. With `set -e`, the unsigned case returned failure and stopped before generating the complete install metadata.

After: explicit `if` blocks make the optional signing fields safe. A temporary repository with fake `ostree`/`flatpak` commands verifies both `.flatpakrepo` and `.flatpakref`, their expected URLs and `.nojekyll`. No repository was published or signed.

#### 18. Refresh development setup and identify archival examples

**Files:** `README.md`, `docs/MCP_STREAMING_CLIENT.md`, `examples/streaming-client-example.ts`, `examples/streaming-client-browser.html`.

Development instructions now use stable Rust, the Node version declared in `.nvmrc`, `npm ci`, and the project's preferred `--no-watch` development command. The old `/gateway/stream` example and guide now identify themselves as archival and point to the maintained MCP client/API documentation; the example imports a client that no longer exists and should not be mistaken for current instructions.

The historical browser example now creates server tags with DOM elements and `textContent` instead of interpolating server names into `innerHTML`. Its JavaScript was syntax-checked without launching a browser.

### Other reviewed areas and limits

- **Local-model metadata and transport:** inspected redirect/token-origin gating, typed error mapping, path validation, GGUF classification, memory-fit estimation, library staging and tests. Model metadata/fit computations do not establish actual hardware inference performance.
- **Local engine lifecycle:** reviewed recipes, managed pointers, archive extraction, process supervision, cancellation and fake-engine tests. Actual installation recipes, vendor releases and GPU executables were not run.
- **Embeddings:** inspected model lifecycle, serialization locks, tokenization/pooling and the downloader. No numerical embedding-quality validation was attempted.
- **RouteLLM:** inspected initialization, prediction, idle unload, downloader verification and test annotations. GPU examples and ignored model benchmarks were excluded.
- **Compression:** inspected protection, model scoring/window boundaries, service batching and download status; tested the pure protection layer. Numeric compression quality remains unvalidated.
- **Guardrails:** reviewed model/executor orchestration, failure reporting and confidence filtering. Provider protocol support and model efficacy require provider-specific fixtures or explicit live tests.
- **Catalog:** inspected cache-only build behavior and manifests. The pre-existing `modelsdev_raw.json` modification was preserved, and catalog fetches were disabled for all review builds.
- **Packaging/release:** read CI/deployment/release structure, Docker defaults, package-manager templates, publishing/pruning scripts, and shell/Python syntax. No installer image, container, package manager publication or external release action was executed.
- **Documentation:** current README, project guidance and selected recent architecture plans were consulted. Historical docs/plans and generated assets are explicitly classified in the inventory.

### Follow-up observations requiring separate validation/design

1. Embedding/compression downloaders copy cached artifacts directly to their final paths and use presence-based downloaded checks. A staged multi-file installation/manifest would give stronger interruption and consistency guarantees; real model downloads were intentionally excluded here.
2. Model forward passes occur inside async service methods while holding serialization locks. Moving them to dedicated blocking workers needs ownership/cancellation design and performance validation with actual models.
3. Model performance, quantization fit, native provider behavior, macOS GPU execution and Windows/Linux installer behavior remain platform/live-test work.
4. The configuration loader's recovery and watcher pathways deserve additional stress tests around overlapping external edits and atomic file replacement; this review fixes the demonstrated shared-orphan recovery bug, not every possible watcher race.
5. The historical streaming examples remain archival; no compatibility endpoint or retired client was reintroduced.
6. This review did not refresh dependency versions or perform an internet advisory assessment. Build/lint/test results establish only the checked local behavior and do not amount to a vulnerability-free certification.

### Validation record

The final consolidated summary records the exact final commands/results. Initial CPU core tests passed 183 tests. OAuth passed 38 tests after rerunning with localhost binding permitted. The stable Rust 1.99.0 foundation rerun passed 372 tests with 4 ignored: API keys 17 (3 ignored), configuration 127, engines 52 (1 ignored), guardrails 58, local models 88 and secret scanner 30. This includes the final keychain cache-concurrency changes; the later library validation-lock refinement is covered by the final shared-library rerun in the consolidated report. Pure compression protection passed 26 tests. Packaging passed 2 test methods, including four destructive-retention subcases. Workflow YAML, 10 shell scripts, the three Python diagnostic sources and browser-example JavaScript passed syntax checks; Python ML diagnostics were parsed, not executed.

---

## Appendix 2: API, routing, and providers review — 2026-10-01

Source report: `plan/review-2026-10-01/api-routing-providers.md`.


### Scope and method

Owned: `crates/lr-server`, `crates/lr-router`, `crates/lr-providers`. Read the repository's CLAUDE.md conventions. Preserved the pre-existing edit to `crates/lr-catalog/catalog/modelsdev_raw.json`. No commits. No application launch, real provider requests, credentials access, model download, model loading, inference, or GPU execution.

Coverage is deliberately distinguished: every Rust source file below received an inventory and structural risk scan (production panic/slicing sites, asynchronous synchronization, stream framing, secret handling, request surfaces, and test side effects). High-risk implementations received targeted substantive reads, and changed functions/tests received full review. This is **not a line-by-line proof of all source code**, nor a claim of complete dynamic coverage. The provider catalog/factory tables, giant route orchestration modules, and embedded engine implementations were inspected selectively; embedded serving behavior remains static-only.

### Implemented improvements

#### Provider streaming correctness

1. Added shared `sse_lines::line_batches` built on the byte-preserving SSE/NDJSON framer; existing `lines` behavior is retained. Complete lines decode only after all UTF-8 bytes arrive. CRLF is normalized and an unterminated final line is emitted at EOF. Errors preserve stream ordering.
2. Migrated 13 completion adapters: Anthropic, Cerebras, DeepInfra, Gemini, Groq, Mistral, Ollama, OpenAI, generic OpenAI-compatible, OpenRouter, Perplexity, TogetherAI, xAI. Previously each independently decoded arbitrary HTTP chunks into lossy UTF-8 strings before buffering, corrupting non-ASCII text split across reads. They also dropped unterminated last lines. Removed the redundant per-adapter shared string/mutex buffer; retained provider-specific event conversion and usage handling.
3. Responses SSE now decodes through the common byte-safe line framer. Its frame-boundary routine chooses whichever LF/CRLF delimiter occurs first, preventing concatenation of independent mixed-ending events.
4. Ollama pull-progress decoding now consumes complete NDJSON lines, preserves every progress record, ignores blank lines, flushes the final record without newline, and reports malformed/provider-error records instead of fabricating empty-chunk failures. Tests feed synthetic bytes only: no model pull was performed.
5. Added exhaustive single-boundary and one-byte fragmentation fixtures containing accented text, emoji, CJK, CRLF, a data sentinel, and an unterminated tail. Responses fixtures verify wire order; Ollama fixtures verify all progress records.

#### OAuth credential durability and confidentiality

`lr-providers/src/oauth/storage.rs` now serializes read-modify-save transactions, writes into a same-directory private NamedTempFile, syncs contents, and atomically replaces the destination. Unix files are 0600 before any token bytes are written, avoiding the old chmod-after-write exposure. Parallel writers cannot truncate/interleave the JSON or publish stale snapshots. A failed save/delete leaves the in-memory credentials unchanged. The blocking write task owns the cache guard until both disk and cache commit, including when the async caller is cancelled. Existing destination symlinks are replaced rather than followed. Tests use only disposable directories and synthetic credentials. Updated the module description to accurately describe the owner-private JSON store instead of claiming it is encrypted.

#### Rate limiting and concurrency

- All configured metric types now participate in admission checks. Previously token/cost usage was recorded but never consulted when allowing subsequent requests, so exhausted budgets did not block requests.
- Multiple windows for one metric now charge each request once. They retain history for the longest configured window, so inspecting/checking a minute window cannot erase events still needed by an hourly window.
- Record paths prune old events as well as check paths; timestamp insertion preserves ordering despite concurrent completion order, keeping front-based expiry valid.
- DashMap shard guards are released before awaiting per-state locks when taking persistence snapshots or reading usage.
- Total token arithmetic saturates instead of overflowing; retry delays round up rather than advertising zero seconds for a remaining fractional second.
- The negative endpoint-capability cache removes expired entries conditionally, avoiding deleting a newly refreshed entry after releasing its read guard.

These fixes do not implement atomic admission reservations: simultaneous in-flight requests can still overshoot a limit because actual usage is charged after completion. The current API also reports only the first configured window for a metric in get_*_usage. Both are explicit follow-ups rather than hidden guarantees.

#### HTTP server robustness, privacy, and lifecycle

- Host-header protection parses HTTP authority correctly: IPv6 loopback works; non-ASCII/malformed hosts, invalid ports, userinfo and external domains are rejected; DNS localhost matching is case-insensitive. Existing support for a missing Host header is retained.
- Request/trace logs include only the URI path, preventing MCP `?token=` credentials and other query data from entering persistent logs.
- Tower authentication now calls the service instance that was polled for readiness, preserving concurrency-limit/readiness permits instead of calling a fresh clone.
- Port search stops at 65535 instead of overflowing; binding port zero returns/reports the OS-assigned port.
- Periodic server session/token cleanup tasks observe server cancellation rather than retaining old server state after every restart.
- Transcription/translation monitor previews truncate on a character boundary, preventing panics when byte 200 lies inside a Unicode transcript character.
- `/responses` history retrieval is scoped to the authenticated client through the new `get_active_for_client` storage API supplied by the MCP/context review agent. Foreign response IDs cannot import another client's messages/tools; foreign/missing/expired references retain the existing start-fresh behavior. Owner-scope regression coverage is in lr-responses-sessions.

#### Numeric validation

- Anthropic thinking budgets reject integers that cannot fit the supported type instead of wrapping into an apparently valid small budget.
- Logprob counts reject negative, fractional, textual and oversized values before narrowing; omitted counts retain the existing zero default.
- System One probability normalization scales by the maximum finite weight before summing. Large finite weights such as `[f64::MAX, f64::MAX]` now normalize to `[0.5, 0.5]` instead of all zeros from an infinite sum.

### Validation

Compiled all three library test targets without executing any tests:

```sh
env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-providers -p lr-router -p lr-server --lib --no-run
```

**Compilation passed in 15m09s** on stable Rust 1.99. Build log: `/private/tmp/localrouter-review-api-build.log`. The only reported build caveat was an upstream `block 0.1.6` future-incompatibility warning. Compilation of model dependencies does not load a model or execute GPU work.

To release the Cargo lock for the other reviewers, executed inspected filters directly against the resulting test binaries under `/private/tmp/localrouter-review-target/debug/deps`:

| Binary / filters | Result | Side effects |
|---|---:|---|
| `lr_providers-05b81bf276645437`: `sse_lines::tests`, `openai_responses::stream::tests`, `oauth::storage::tests`, `features::anthropic_thinking::tests`, `features::logprobs::tests`, `systemone::types::tests`, `ollama::tests::pull_progress_` | **55 passed, 0 failed** | Synthetic byte/string fixtures; temporary OAuth files only |
| `lr_router-a524b103bc205375`: `rate_limit::tests`, `endpoint_cache::tests` | **13 passed, 0 failed** | In-memory state, temporary persistence and short expiry waits |
| `lr_server-a932ff08da3f7e2c`: `host_validation_tests`, `trace_middleware_tests`, `transcript_previews_preserve_multibyte_characters`, `kill_switch_tests` | **12 passed, 0 failed** | In-memory Axum requests/streams; no app/server startup |
| Provider binary: the 17 `stream_*` usage fixtures enumerated below | **17 passed, 0 failed** | Temporary localhost HTTP fixture servers and synthetic keys |

**Total: 97 passed, 0 failed.** Tests ran with `--test-threads=4`. The localhost-fixture run used approved sandbox escalation because loopback binds are restricted; no test contacted a real provider or used actual credentials. Exact fixture filters: `anthropic::tests::stream_`, `cerebras::tests::stream_reports_upstream_usage`, `deepinfra::tests::stream_reports_upstream_usage`, `gemini::tests::stream_`, `groq::tests::stream_reports_upstream_usage`, `mistral::tests::stream_reports_upstream_usage`, `ollama::tests::stream_reports_final_eval_counts`, `openai::tests::stream_reports_upstream_usage`, `openai_compatible::tests::stream_`, `openrouter::tests::stream_reports_upstream_usage`, `perplexity::tests::stream_reports_upstream_usage`, `togetherai::tests::stream_reports_upstream_usage`, `xai::tests::stream_reports_upstream_usage`.

Changed Rust sources were formatted and `git diff --check` passed for owned files. Full workspace lint/build results are recorded by the root reviewer; the above tests intentionally exclude embedded model execution, GPU/device selection, real provider integration and external OAuth flows.

### Important follow-ups and review limitations

- Cross-hop trust issue was fixed jointly: arbitrary inbound trace headers previously skipped scans/approvals/accounting and the proxy firewall. The MCP agent changed central `RequestTrace::outbound_for` to preserve correlation IDs but reset the enforcement hop to 1; server tests now assert a forged header cannot set duplicate state or suppress rate-limit charges. All production inbound parse paths were scanned: the server and proxy both use the central helper. Intentional consequence: multi-hop transformations/accounting can repeat until an authenticated, request-bound handoff protocol is implemented. A simple ID registry or an unbound signature would still allow replay.
- Stream line/frame accumulation remains unbounded for a provider that never supplies a delimiter. A protocol-aware maximum and typed error need coordinated design (large tool arguments are legitimate).
- Server lifecycle `start`/`stop` and reverse-proxy startup have separate read/write phases and deserve a serialized lifecycle transaction under concurrent UI calls.
- Token endpoint documentation describes form-encoded OAuth requests but the handler still extracts JSON. Its per-client-ID attempt map also has no global cardinality cap. These are pre-existing compatibility/resource issues outside the implemented focused changes.
- Full production payload logging remains intentionally present in monitor/error paths. This review fixed definite query credential leakage, not every product-level logging retention/privacy policy.
- External provider protocol compatibility, real OAuth exchanges, embedded inference, model engines, OS dialogs/keychain access, and platform-specific runtime behavior were not exercised.
- No defects were established in the sampled existing byte-safe Cohere parser, shared typed OpenAI error classification, health cache aggregate calculation, feature registry wiring, provider model catalog fallback logic, and System One request cardinality validation. This is scoped evidence, not a blanket guarantee.

### Exact file coverage inventory

**D**: changed or targeted substantive inspection of relevant behavior; **S**: structural scan/inventory plus selected declarations/call paths. D does not imply every line in a large file was deeply reviewed.

| File | Lines at report generation | Coverage |
|---|---:|---|
| `crates/lr-server/src/lib.rs` | 911 | D |
| `crates/lr-server/src/manager.rs` | 156 | D |
| `crates/lr-server/src/middleware/auth_layer.rs` | 302 | D |
| `crates/lr-server/src/middleware/client_auth.rs` | 270 | D |
| `crates/lr-server/src/middleware/error.rs` | 218 | S |
| `crates/lr-server/src/middleware/mod.rs` | 5 | S |
| `crates/lr-server/src/openapi/extensions.rs` | 325 | S |
| `crates/lr-server/src/openapi/mod.rs` | 333 | S |
| `crates/lr-server/src/routes/audio.rs` | 1621 | D |
| `crates/lr-server/src/routes/chat.rs` | 2843 | S |
| `crates/lr-server/src/routes/completions.rs` | 1691 | S |
| `crates/lr-server/src/routes/embeddings.rs` | 492 | S |
| `crates/lr-server/src/routes/finalize.rs` | 473 | S |
| `crates/lr-server/src/routes/generation.rs` | 48 | S |
| `crates/lr-server/src/routes/helpers.rs` | 357 | D |
| `crates/lr-server/src/routes/images.rs` | 663 | S |
| `crates/lr-server/src/routes/mcp.rs` | 1797 | S |
| `crates/lr-server/src/routes/mcp_ws.rs` | 319 | S |
| `crates/lr-server/src/routes/mod.rs` | 37 | S |
| `crates/lr-server/src/routes/models.rs` | 398 | S |
| `crates/lr-server/src/routes/moderations.rs` | 397 | S |
| `crates/lr-server/src/routes/monitor_helpers.rs` | 927 | S |
| `crates/lr-server/src/routes/oauth.rs` | 443 | D |
| `crates/lr-server/src/routes/pipeline.rs` | 2802 | D |
| `crates/lr-server/src/routes/responses.rs` | 1523 | D |
| `crates/lr-server/src/routes/stream_usage.rs` | 466 | D |
| `crates/lr-server/src/routes/systemone.rs` | 671 | S |
| `crates/lr-server/src/state.rs` | 1816 | S |
| `crates/lr-server/src/types.rs` | 1155 | S |
| `crates/lr-router/src/endpoint_cache.rs` | 120 | D |
| `crates/lr-router/src/free_tier.rs` | 2644 | D |
| `crates/lr-router/src/lib.rs` | 3516 | S |
| `crates/lr-router/src/rate_limit.rs` | 1048 | D |
| `crates/lr-router/src/systemone.rs` | 1097 | S |
| `crates/lr-providers/src/anthropic.rs` | 2050 | D |
| `crates/lr-providers/src/cerebras.rs` | 442 | D |
| `crates/lr-providers/src/cohere.rs` | 1508 | D |
| `crates/lr-providers/src/deepinfra.rs` | 731 | D |
| `crates/lr-providers/src/embedded/decider.rs` | 565 | S |
| `crates/lr-providers/src/embedded/kev.rs` | 606 | S |
| `crates/lr-providers/src/embedded/laya.rs` | 623 | S |
| `crates/lr-providers/src/embedded/llamacpp.rs` | 744 | S |
| `crates/lr-providers/src/embedded/mod.rs` | 645 | S |
| `crates/lr-providers/src/embedded/ollaya.rs` | 1363 | S |
| `crates/lr-providers/src/embedded/sdcpp.rs` | 883 | S |
| `crates/lr-providers/src/embedded/von.rs` | 552 | S |
| `crates/lr-providers/src/factory.rs` | 4366 | S |
| `crates/lr-providers/src/features/anthropic_thinking.rs` | 302 | D |
| `crates/lr-providers/src/features/gemini_thinking.rs` | 240 | S |
| `crates/lr-providers/src/features/json_mode.rs` | 544 | S |
| `crates/lr-providers/src/features/logprobs.rs` | 553 | D |
| `crates/lr-providers/src/features/mod.rs` | 196 | S |
| `crates/lr-providers/src/features/openai_reasoning.rs` | 215 | S |
| `crates/lr-providers/src/features/prompt_caching.rs` | 691 | S |
| `crates/lr-providers/src/features/structured_outputs.rs` | 682 | S |
| `crates/lr-providers/src/gemini.rs` | 1721 | D |
| `crates/lr-providers/src/gpt4all.rs` | 554 | S |
| `crates/lr-providers/src/groq.rs` | 745 | D |
| `crates/lr-providers/src/health.rs` | 405 | D |
| `crates/lr-providers/src/health_cache.rs` | 595 | D |
| `crates/lr-providers/src/http_client.rs` | 369 | D |
| `crates/lr-providers/src/jan.rs` | 554 | S |
| `crates/lr-providers/src/key_storage.rs` | 314 | D |
| `crates/lr-providers/src/lib.rs` | 3149 | S |
| `crates/lr-providers/src/llamacpp.rs` | 584 | S |
| `crates/lr-providers/src/lmstudio.rs` | 745 | S |
| `crates/lr-providers/src/localai.rs` | 717 | S |
| `crates/lr-providers/src/mistral.rs` | 659 | D |
| `crates/lr-providers/src/oauth/anthropic_claude.rs` | 237 | S |
| `crates/lr-providers/src/oauth/github_copilot.rs` | 556 | S |
| `crates/lr-providers/src/oauth/mod.rs` | 295 | D |
| `crates/lr-providers/src/oauth/openai_codex.rs` | 357 | S |
| `crates/lr-providers/src/oauth/storage.rs` | 297 | D |
| `crates/lr-providers/src/oauth/token_source.rs` | 530 | D |
| `crates/lr-providers/src/ollama.rs` | 1474 | D |
| `crates/lr-providers/src/openai.rs` | 2006 | D |
| `crates/lr-providers/src/openai_compatible/stream_usage.rs` | 591 | D |
| `crates/lr-providers/src/openai_compatible.rs` | 1040 | D |
| `crates/lr-providers/src/openai_responses/emit.rs` | 479 | S |
| `crates/lr-providers/src/openai_responses/http.rs` | 131 | S |
| `crates/lr-providers/src/openai_responses/mod.rs` | 30 | S |
| `crates/lr-providers/src/openai_responses/request.rs` | 492 | S |
| `crates/lr-providers/src/openai_responses/response.rs` | 208 | S |
| `crates/lr-providers/src/openai_responses/stream.rs` | 776 | D |
| `crates/lr-providers/src/openai_responses/types.rs` | 368 | S |
| `crates/lr-providers/src/openrouter.rs` | 872 | D |
| `crates/lr-providers/src/perplexity.rs` | 456 | D |
| `crates/lr-providers/src/registry.rs` | 1757 | D |
| `crates/lr-providers/src/sse_lines.rs` | 249 | D |
| `crates/lr-providers/src/systemone/emulation.rs` | 581 | S |
| `crates/lr-providers/src/systemone/gateway.rs` | 865 | S |
| `crates/lr-providers/src/systemone/mod.rs` | 20 | S |
| `crates/lr-providers/src/systemone/provider.rs` | 619 | S |
| `crates/lr-providers/src/systemone/types.rs` | 639 | D |
| `crates/lr-providers/src/togetherai.rs` | 916 | D |
| `crates/lr-providers/src/xai.rs` | 472 | D |

---

## Appendix 3: MCP, tools, context and local sessions review — 2026-10-01

Source report: `plan/review-2026-10-01/mcp-tools-context.md`.


### Progress and approach

- [x] Read the project guide; record the initial inventory and preserve unrelated edits.
- [x] Structurally inspect every assigned first-party Rust module and crate manifest.
- [x] Deep-review trust boundaries, session lifecycle, concurrency and resource handling.
- [x] Implement concrete improvements with focused local regressions.
- [x] Review implementation against findings and test coverage; perform a fresh bug hunt.
- [x] Coordinate CPU-only offline validation with the root agent and record exact initial outcomes; final stable reruns belong to the consolidated report.
- [x] Complete the detailed findings, coverage and remaining limitations below.

No commits are made during this shared review, as instructed by the coordinating agent. No GPU execution, model inference/download, external provider calls, credential access or application launch is permitted.

### Inventory at the start of review

| Crate | Rust files | Rust lines (including tests) |
|---|---:|---:|
| lr-mcp | 35 | 29,707 |
| lr-mcp-via-llm | 8 | 8,357 |
| lr-context | 8 | 5,442 |
| lr-memory | 5 | 2,918 |
| lr-skills | 8 | 2,520 |
| lr-marketplace | 7 | 3,542 |
| lr-coding-agents | 6 | 3,844 |
| lr-proxy | 21 | 7,551 |
| lr-responses-sessions | 1 | 369 |

The inventory totals 99 Rust files and 64,250 lines. Structural coverage and deep inspection are distinct; this review does not claim a line-by-line proof of correctness across that volume.

### Findings and changes

Implemented changes and per-section findings are detailed below.

### Validation

Initial compilation and 800 CPU/local tests passed. The coordinator subsequently ran the final shared-boundary suites on stable Rust 1.99.0: all 297 tests passed across six crates; see the final validation note below.

### Implemented corrections (source review complete; validation tracked separately)

#### 1. Response-history isolation between authenticated clients

`lr-responses-sessions/src/lib.rs` now exposes `get_active_for_client(id, client_id, retention)`. The SQL query filters ownership before decoding the row. A foreign response ID and a missing response ID both return `None`; lookups do not refresh last activity. Existing unrestricted `get_active` remains for administrative/internal uses. A new regression covers own, foreign and missing response IDs and verifies the original timestamp stays unchanged.

The review found the actual leak in `lr-server/src/routes/responses.rs`: a supplied `previous_response_id` was loaded without comparing its owner with the authenticated client. The API agent migrated that caller to the owner-scoped method. This is a coordinated fix spanning the assigned store and another agent's server code; the server agent owns its endpoint behavior and endpoint tests.

#### 2. Concurrent memory session creation

`lr-memory/src/session_manager.rs` uses a DashMap entry lock through lookup and replacement. Previously two requests could both see no session, generate separate transcript paths, and overwrite the map entry. The new regression synchronizes 16 threads for each of 16 independent clients and verifies that exactly one request creates the shared session.

#### 3. Memory expiry racing with renewed activity

The same manager now uses conditional removal under the DashMap shard lock. The previous sequence removed the session, checked its age, and reinserted it if refreshed. That exposed a gap in which another request could create a new session that was subsequently overwritten. Existing expiry/max-duration/touch tests exercise ordinary behavior; the lock-level reasoning covers the previously exposed removal gap.

#### 4. Transcript initialization no longer truncates concurrent data

`lr-memory/src/transcript.rs` now creates/opens the transcript in append mode. An already-started request can append an exchange before another request's initialization finishes; `fs::write(path, "")` previously erased that exchange. A regression writes an exchange first, initializes afterward, and verifies byte-for-byte preservation.

Memory short display IDs and expiry logging also use character-safe shortening instead of slicing an arbitrary UTF-8 string at byte 8. A Unicode regression covers both timestamp-prefixed and legacy-style names.

#### 5. Skill ReadFile confines resolved symlinks

`lr-skills/src/mcp_tools.rs` routes reads through `SkillManager::get_resource`, reusing its canonical path containment check. Discovery includes symlinked files, so membership in the discovered file list did not prove containment. The Unix regression creates a permitted reference and a symlink to a sibling private file: the permitted read succeeds and the symlink escape fails.

This does not claim protection against a hostile local process swapping filesystem entries between validation and open; fully race-resistant traversal would require directory-relative no-follow operations.

#### 6. Marketplace downloads validate destinations and payloads

`lr-marketplace/src/skill_sources.rs` validates portable relative manifest paths before download, rejecting absolute paths, parent traversal, Windows drive/alternate-stream syntax, backslashes and NULs. Additional files cannot replace the authoritative `SKILL.md`. It checks existing path components for symlinks before creating directories or writing. The implementation rejects unsuccessful HTTP responses rather than saving an HTML/error response as an installed skill.

Downloads are streamed into a bounded buffer: at most 16 MiB per file and 64 MiB per installation. Limits apply to actual received chunks as well as advertised Content-Length, so omitted/misleading length headers do not disable the limit. Four synthetic loopback HTTP responses test unsuccessful status, excessive Content-Length, an excessive chunked body with no Content-Length, and a successful file that consumes the aggregate budget.

`lr-marketplace/src/lib.rs` now exposes `MarketplaceService::download_skill(&listing) -> Result<PathBuf, MarketplaceError>`, including validation that `source_label` and `name` are each a single directory component. Other agents migrated the real Tauri direct-install path and launcher callback to this API, removing duplicated unsafe download code rather than leaving this correction unused.

Additional regressions cover portable path rules, nested directory creation, symlinked parent directories, symlinked manifest files and preservation of outside content. A late regression verifies reserved manifest names, including normalized `SKILL.md/` and `SKILL.md/.`, and traversal paths are rejected before filesystem/network work in the download client. Installation is still not transactional: a later download failure can leave a partial directory; see follow-ups.

#### 7. Script output stays bounded and async timeouts actually terminate

`lr-skills/src/executor.rs` drains stdout/stderr while retaining only the latest 1 MiB per stream. Both sync and async execution use the bounded reader, so a noisy script cannot grow an unbounded Vec for the entire timeout. Pipes continue draining after the capture cap, avoiding an artificial child-process deadlock.

The async path previously awaited stdout/stderr EOF before killing a timed-out child, so the child could keep those pipes open indefinitely and defeat its timeout. It now kills first, then allows at most one second for pipe draining, aborting unfinished drain tasks if descendants keep descriptors open. The background-owned child uses kill-on-drop for cancellation/runtime shutdown. Captured byte tails are decoded lossily so a cut through a Unicode character does not erase all output.

Regressions verify the bounded capture retains the suffix and run only a synthetic local `/bin/sh` script containing `exec sleep 10` with a zero-second timeout; completion must be reported promptly. No coding agent, model, package manager or GPU program is executed.

#### 8. Context truncation honors its byte limit

`lr-context/src/truncate.rs` no longer unconditionally includes the first/last line when either line alone exceeds its budget. That previously allowed a huge leading/trailing line to defeat a supposedly bounded tool response. It falls back to character-safe truncation whenever line-aligned edges cannot fit. The regression covers oversized first and last lines independently and together, Unicode content, and budgets from zero through 1000 bytes.

#### 9. Context reads avoid overflow and quadratic long-line indexing

`lr-context/src/lib.rs` uses saturating addition for offset plus requested limit. A read beginning at line 2 with `usize::MAX` previously overflowed in a debug build (or wrapped in an optimized build). A regression now verifies the correct remaining lines.

Long-line subdivision computes UTF-8 byte offsets once instead of repeatedly summing all prior character widths for every slice. This changes that indexing step from quadratic growth to a linear pass while retaining existing numbered sub-line behavior and UTF-8-safe boundaries. Existing long-line, sub-offset, Unicode and large-document tests cover the behavior.

#### 10. MCP WebSocket authentication headers reach the handshake

`lr-mcp/src/transport/websocket.rs` constructs the handshake request and copies the configured headers into it with validated header names/values. Previously headers were stored in the struct but `connect_async(&url)` never sent them, breaking authenticated WebSocket MCP servers. A loopback server verifies a synthetic `x-test-auth` header is received.

#### 11. Concurrent/cancelled MCP WebSocket writes retain the connection

The WebSocket sink is now held in a Tokio mutex and borrowed while awaiting a send. Previously each sender took the sink out of an Option, causing overlapping requests to report “write handle not available”; cancellation during the await could permanently drop the only write handle. The local regression cancels a request waiting on the write lock, then successfully submits 16 concurrent JSON-RPC requests with 256 KiB synthetic payloads and checks correlation of restored IDs.

#### 12. Cancelled MCP requests clean up pending state

`lr-mcp/src/transport/mod.rs` defines a scoped pending-request guard used by stdio, SSE and WebSocket sends. Dropping a caller's future now removes its response sender even if cancellation occurs during serialization/write/HTTP send/response wait. Existing explicit error cleanup remains harmless and idempotent. The WebSocket cancellation regression checks the pending map returns to empty.

#### 13. MCP broadcast preserves backend errors

`lr-mcp/src/gateway/router.rs` now converts a JSON-RPC error response into a broadcast failure rather than treating it as a successful null result. A mock transport regression verifies an unsupported-method response is present in failures and absent from successes. Retry backoff also clamps the shift before arithmetic, preventing overflow for high configured retry counts while preserving the 10-second cap.

#### 14. MCP transport logs do not print configured header values

SSE diagnostic messages log header names rather than entire request/response header maps. In particular this removes configured Authorization/custom-token values from ordinary debug logging. Upstream error bodies and URLs still have broader diagnostic privacy considerations; the change is intentionally specific to the confirmed header disclosure.

#### 15. Mixed tool execution is claimed atomically and stays cancellable

`lr-mcp-via-llm/src/manager.rs` matches client tool-call results and removes the matching pending execution under one DashMap entry lock. It can no longer match one execution then remove a replacement inserted before its separate removal. A concurrent-consumer regression verifies a pending execution is claimed once.

`lr-mcp-via-llm/src/orchestrator.rs` awaits background tool handles while they remain owned by PendingMixedExecution. Previously taking the handles out before awaiting disabled the Drop implementation's ability to abort unfinished tools if the resume request was cancelled. Existing pending-execution Drop tests cover abort ownership; full mocked orchestrator integration tests provide CPU-only behavior coverage.

#### 16. Coding-agent session status and listing correctness

`lr-coding-agents/src/manager.rs` subscribes to completion notifications before checking current state, closing a missed-wakeup window that could delay a completed session until the full wait timeout. It releases DashMap guards before asynchronously waiting for per-session locks during interruption. Session lists now sort the complete matching set before truncation; the previous arbitrary DashMap iteration limit could omit the newest sessions. Limit zero returns no sessions.

Two process-free regressions cover completion notification and newest-first limiting, including zero limit, wrong client and agent-type filtering. Real coding agents and their permission flows were not launched.

#### 17. Proxy CA key permissions are restrictive before bytes are written

`lr-proxy/src/cert.rs` opens new secret files with Unix mode 0600 and tightens an existing open file's mode before writing bytes. Previously it wrote the key with default permissions and only afterward changed permissions. The existing root-key permission test and a new pre-existing-mode-0644 synthetic fixture cover the final modes/content. No real CA key or credential file was read during review.

#### 18. Untrusted request trace headers cannot disable enforcement/accounting

This coordinated second-wave fix is implemented centrally in `lr-types/src/trace.rs` and documented in `foundations.md`. `RequestTrace::outbound_for` preserves the wire trace ID for correlation but resets its hop to 1. A public `X-LocalRouter-Trace: forged;hop=99` no longer grants “already checked” privileges. Proxy boundary tests verify the forged header still reaches the firewall and increments metrics; reverse proxy integration expectations retain correlation but reject duplicate privileges. The API agent owns equivalent server ingress tests.

Compatibility change: unauthenticated wire-based multi-hop deduplication is disabled; real multi-hop routes can now count/prompt more than once. Restoring that optimization requires authenticated, request-bound provenance. Internal deliberately trusted traces retain their existing behavior.

### Review coverage and retained behavior

This was a risk-driven review of all nine crate inventories, manifests, public interfaces, stateful operations, I/O boundaries and relevant test entry points. Deep review concentrated on the paths above. The following untouched areas were reviewed structurally and sampled semantically; “no additional fix” is not a correctness guarantee.

| Section | Inspected contracts and behavior | Additional result |
|---|---|---|
| MCP protocol and merger | JSON-RPC request/notification/response types, error constructors, capability/version types, namespace and catalog merge interfaces | No additional verified issue isolated in the sampled pure merge/protocol paths; no live protocol-conformance claim |
| Gateway access control | Permission hierarchy, Allow/Ask/Off resolution, session approval/denial precedence | Existing explicit denial/permission branching retained |
| Gateway tools/resources/prompts | Namespaced mappings, resource URI fallback, per-session transport extraction, virtual server dispatch and permission entry points | Follow-up on concurrent session refresh and live permission changes remains warranted |
| Approval managers | Pending-map lifecycle, timeout/response/cancellation handling, elicitation schema validation | Existing explicit timeout cleanup present; future-drop cleanup is less consistent than transport cleanup |
| Virtual servers | Skills, memory, marketplace, coding-agent and context tool ownership/configuration interfaces | Verified dependency fixes flow through shared implementations; no external tool execution performed |
| MCP manager/bridge/OAuth | Transport lifecycle surfaces, stdio cwd validation, stream relay signatures, test keychain injection and ignored package-download tests | Live process fleets, OAuth browser consent/redirects and upstream integration intentionally untested |
| MCP via LLM | Session hash matching, explicit key fast path, hidden-history reconstruction, guardrail gates, mixed/client tool dispatch, background-handle ownership | Mock provider/MCP integration only; heuristic session matching policy retained |
| Context | FTS query parameterization/sanitization, transactional replacement, search fallback, reads, chunking, truncation, hybrid rank fusion | No embedding model or vector benchmark run; concurrency of optional vector reindexing merits separate work |
| Memory | Client stores, transcript capture, session grouping, archives/compaction callback, search/reindex interfaces | Compaction uses mock callbacks in tests; no real summarization call |
| Skills | Discovery/frontmatter, archive entry confinement, manager snapshots, filesystem watcher, file reads, script executor | Archive expansion limits and watcher backpressure remain follow-ups |
| Marketplace | Registry requests, source cache, listing transforms, install config, popup approvals, callback boundaries, downloads | Existing registry status checks retained; download/install trust boundary improved as above |
| Coding agents | Ownership checks, start/say/resume/status/list/end lifecycle, discovery interfaces, command argument generation, approval service, bounded output ring | Agent binaries, account state and live work directories not exercised |
| Proxy | Authentication entry, host policy, CONNECT/plain HTTP/TLS/reverse interfaces, bounded captures, HTTP/WebSocket framing, wire metadata, active/passive interceptors | No real upstreams, root CA installation, system trust mutation, app listener or user traffic |
| Responses sessions | SQLite schema, persistence, active-window filtering, retention sweep, serialization, owner lookup | CPU in-memory tests; extreme retention values and storage quotas remain follow-ups |

### Remaining limitations and specific follow-ups

- **Coverage:** structural review covered the listed Rust modules; this is not a line-by-line proof of 64k lines. Generated assets, embedded models and third-party dependencies were not audited as first-party source.
- **Skill ZIPs:** enclosed entry names block lexical traversal, but extraction lacks an explicit total expanded-byte/file-count budget and an atomic completion marker. Malformed/huge archive fixtures should precede a broader extractor redesign.
- **Filesystem race resistance:** canonical/no-symlink checks prevent ordinary supplied-path escapes, but do not eliminate races against a hostile local process replacing directories during async I/O. Directory-relative handles/no-follow writes are the stronger boundary.
- **Transactional installs:** a network/error/limit failure can leave a partial installation directory. A staging directory plus atomic promotion would also improve recovery and cancellation behavior.
- **Script lifecycle:** output is now bounded and timeouts effective, but process-group termination/descendant guarantees remain platform-specific; asynchronous output files still update on completed capture rather than live streaming. PID-based temp paths and cleanup deserve a separate lifecycle design.
- **Long-lived maps:** per-client orchestration histories, seen-tool sets and approval stores need a unified memory quota/eviction policy; expiry alone is not a strict live-allocation cap.
- **MCP transport bounds:** line/event buffers and response bodies should receive protocol-appropriate frame limits. SSE multiline data and CRLF framing deserve dedicated interoperability fixtures. No sweeping protocol rewrite was attempted in this pass.
- **Gateway shutdown/reconnect:** SessionTransportSet snapshot/clear/close behavior, revision metadata cleanup and cancellation of approval futures warrant stress testing with real mocked connections and concurrent session resets.
- **Memory archival:** session rotation can leave expired transcript files for later maintenance; conversation detection versus forced close/compaction is not made fully transactional by the map-entry correction.
- **Catalog/embedding paths:** no GPU, model inference, downloads or benchmark execution was used. Optional vector indexing quality and consistency under concurrent reindexing remain outside runtime validation.
- **Coding-agent process policy:** concurrency reservation across simultaneous starts/resumes and authorization changes between requests need broader integration fixtures with a dedicated fake agent protocol before lifecycle changes.
- **Retention/storage:** persistent transcript/session stores lack comprehensive quotas; extreme signed retention settings can still merit input validation before arithmetic.
- **Trace deduplication:** correlation is preserved, but repeated multi-hop accounting/prompts are an intentional security tradeoff until provenance is authenticated.

### Validation evidence

Initial offline build command (no test execution during compilation):

```sh
RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 \
CARGO_TARGET_DIR=/private/tmp/localrouter-review-target \
cargo test --offline -p lr-context -p lr-memory -p lr-skills \
  -p lr-marketplace -p lr-mcp -p lr-mcp-via-llm \
  -p lr-coding-agents -p lr-proxy -p lr-responses-sessions --lib --no-run
```

The ambient toolchain was Homebrew Rust 1.98.1; compilation succeeded in 11m25s. The root agent uses rustup stable 1.99.0 for final CI-parity checks. This toolchain difference was identified and communicated; no claim is made that the initial build itself used stable 1.99.0.

The compiled test binaries then ran directly in three parallel processes, four test threads each, with 180-second process limits. Escalation was needed because the sandbox blocks loopback socket binds. Only reviewed synthetic/local fixtures ran: no GPU work, model downloads, inference, live provider calls or application launch. The three ignored MCP tests require package managers/downloads and stayed ignored.

| Crate | Passed | Failed | Ignored | Initial binary coverage |
|---|---:|---:|---:|---|
| lr-context | 140 | 0 | 0 | Latest code |
| lr-memory | 44 | 0 | 0 | Latest code |
| lr-skills | 34 | 0 | 0 | Latest code, including bounded output and async timeout regressions |
| lr-marketplace | 28 | 0 | 0 | Main download changes; later normalized reserved-name hardening needs rerun |
| lr-mcp | 318 | 0 | 3 | Latest code, including authenticated/concurrent/cancelled loopback WebSocket fixture |
| lr-mcp-via-llm | 101 | 0 | 0 | Latest code and mocked orchestrator tests |
| lr-coding-agents | 58 | 0 | 0 | Latest code, process-free manager tests |
| lr-proxy | 72 | 0 | 0 | CA key fix; late trace boundary/tests require rebuilt binary |
| lr-responses-sessions | 5 | 0 | 0 | Latest owner-isolation API/test |
| **Total** | **800** | **0** | **3** | Initial direct-binary pass |

Logs: `/private/tmp/localrouter-review-mcp-tests/*.log`. Late trace/types, marketplace and foundational corrections subsequently passed the coordinator's final stable rerun; see the final validation note. `rustup run stable rustfmt --edition 2021 --check` passed for all then-modified Rust files. Final normalization changes were formatted with stable rustfmt afterward.

### Full primary Rust file inventory

Every path below was included in the structural inventory/search pass. “Changed / focused review” identifies implemented paths and their close regression files. “Structural / sampled” explicitly avoids claiming line-by-line deep review of an entire large module. Tests/benchmarks listed as structural were inspected for their execution requirements; vector benchmarks were not run.

| File | Review depth |
|---|---|
| `crates/lr-mcp/src/bridge/mod.rs` | Structural / sampled |
| `crates/lr-mcp/src/bridge/stdio_bridge.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/access_control.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/coding_agent_approval.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/elicitation.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/firewall.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_prompts.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_resources.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/merger.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/mod.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/router.rs` | Changed / focused review |
| `crates/lr-mcp/src/gateway/sampling.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/sampling_approval.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/session.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/streaming_notifications.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/tests.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/types.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_marketplace.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_memory.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_server.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_skills.rs` | Structural / sampled |
| `crates/lr-mcp/src/lib.rs` | Structural / sampled |
| `crates/lr-mcp/src/manager.rs` | Structural / sampled |
| `crates/lr-mcp/src/oauth.rs` | Structural / sampled |
| `crates/lr-mcp/src/oauth_browser.rs` | Structural / sampled |
| `crates/lr-mcp/src/protocol.rs` | Structural / sampled |
| `crates/lr-mcp/src/transport/mod.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/session_transport_set.rs` | Structural / sampled |
| `crates/lr-mcp/src/transport/sse.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/stdio.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/websocket.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/integration_tests.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/lib.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/manager.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/session.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/tests.rs` | Changed / focused review |
| `crates/lr-context/benches/vector_search.rs` | Structural / sampled |
| `crates/lr-context/src/chunk.rs` | Structural / sampled |
| `crates/lr-context/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-context/src/hybrid.rs` | Structural / sampled |
| `crates/lr-context/src/lib.rs` | Changed / focused review |
| `crates/lr-context/src/search.rs` | Structural / sampled |
| `crates/lr-context/src/truncate.rs` | Changed / focused review |
| `crates/lr-context/src/types.rs` | Structural / sampled |
| `crates/lr-memory/src/compaction.rs` | Structural / sampled |
| `crates/lr-memory/src/lib.rs` | Structural / sampled |
| `crates/lr-memory/src/session_manager.rs` | Changed / focused review |
| `crates/lr-memory/src/tests.rs` | Changed / focused review |
| `crates/lr-memory/src/transcript.rs` | Changed / focused review |
| `crates/lr-skills/src/discovery.rs` | Structural / sampled |
| `crates/lr-skills/src/executor.rs` | Changed / focused review |
| `crates/lr-skills/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-skills/src/lib.rs` | Structural / sampled |
| `crates/lr-skills/src/manager.rs` | Structural / sampled |
| `crates/lr-skills/src/mcp_tools.rs` | Changed / focused review |
| `crates/lr-skills/src/types.rs` | Structural / sampled |
| `crates/lr-skills/src/watcher.rs` | Structural / sampled |
| `crates/lr-marketplace/src/install.rs` | Structural / sampled |
| `crates/lr-marketplace/src/install_popup.rs` | Structural / sampled |
| `crates/lr-marketplace/src/lib.rs` | Changed / focused review |
| `crates/lr-marketplace/src/registry.rs` | Structural / sampled |
| `crates/lr-marketplace/src/skill_sources.rs` | Changed / focused review |
| `crates/lr-marketplace/src/tools.rs` | Structural / sampled |
| `crates/lr-marketplace/src/types.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/approval.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/discovery.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/lib.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/manager.rs` | Changed / focused review |
| `crates/lr-coding-agents/src/mcp_tools.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/types.rs` | Structural / sampled |
| `crates/lr-proxy/src/active.rs` | Changed / focused review |
| `crates/lr-proxy/src/anthropic.rs` | Structural / sampled |
| `crates/lr-proxy/src/cert.rs` | Changed / focused review |
| `crates/lr-proxy/src/error.rs` | Structural / sampled |
| `crates/lr-proxy/src/interceptor.rs` | Structural / sampled |
| `crates/lr-proxy/src/lib.rs` | Changed / focused review |
| `crates/lr-proxy/src/manager.rs` | Structural / sampled |
| `crates/lr-proxy/src/ollama.rs` | Structural / sampled |
| `crates/lr-proxy/src/openai.rs` | Structural / sampled |
| `crates/lr-proxy/src/passive.rs` | Changed / focused review |
| `crates/lr-proxy/src/resolver.rs` | Structural / sampled |
| `crates/lr-proxy/src/reverse.rs` | Structural / sampled |
| `crates/lr-proxy/src/systemone.rs` | Structural / sampled |
| `crates/lr-proxy/src/tap.rs` | Structural / sampled |
| `crates/lr-proxy/src/tls.rs` | Structural / sampled |
| `crates/lr-proxy/src/transport.rs` | Structural / sampled |
| `crates/lr-proxy/src/websocket.rs` | Structural / sampled |
| `crates/lr-proxy/src/wire.rs` | Structural / sampled |
| `crates/lr-proxy/tests/mitm_e2e.rs` | Structural / sampled |
| `crates/lr-proxy/tests/passthrough_e2e.rs` | Structural / sampled |
| `crates/lr-proxy/tests/reverse_e2e.rs` | Changed / focused review |
| `crates/lr-responses-sessions/src/lib.rs` | Changed / focused review |

### Final coordinator validation

Stable Rust 1.99.0 final shared-library run passed **297 tests, zero failures**: lr-local-models 88, lr-marketplace 29, lr-monitor 33, lr-monitoring 45, lr-proxy 73 and lr-types 29. This includes the late marketplace normalization, trace, metrics query/index and monitor callback changes. Both the model-library lock refinement and indexed metrics source predate their rebuilt test binaries. A subsequent marketplace-only rerun after lint-only fixture corrections passed all 29 tests. Log: `/private/tmp/localrouter-review-final-shared-tests.log`; rerun: `/private/tmp/localrouter-review-marketplace-final-tests.log`. Whole-workspace all-target Clippy results and the reverse-proxy integration run are recorded in the consolidated report.

Clippy prompted a meaningful nonempty-read assertion and direct struct initialization in the marketplace HTTP fixture. The WebSocket fixture has a narrowly scoped large-error allowance because the third-party handshake callback requires an HTTP response error type; production error handling is unchanged.

The stable reverse-proxy TCP integration suite also passed all **6 tests**, including forged-hop reset, correlation, forwarding, streaming and unreachable-upstream behavior. These use ephemeral loopback listeners and dummy upstreams. Workspace `cargo clippy --offline --workspace --all-targets -- -D warnings` passed after the fixture lint corrections.

---

## Appendix 4: Foundational libraries second-wave review — 2026-10-01

Source report: `plan/review-2026-10-01/foundations.md`.


- [x] Inventory the additional assigned modules.
- [x] Inspect client/token management, metrics persistence, event buffers, catalog matching and utility contracts.
- [x] Implement independently verified improvements and local regressions.
- [x] Review the final changes and coverage; run focused CPU-only checks and hand final stable compilation to the coordinating root agent.
- [x] Record precise validation, coverage boundaries and remaining follow-ups.

Scope: lr-monitoring, lr-monitor, lr-json-repair, lr-types, lr-utils, lr-clients and lr-catalog. The catalog snapshot is pre-existing user work and must remain untouched. Root previously tested types, utils, json-repair and monitor; this wave supplements that work.

### Implemented fixes

#### Public trace headers no longer grant enforcement/accounting bypass

Root identified that the deduplication trace was trusted across an HTTP boundary. A caller could send `X-LocalRouter-Trace: anything;hop=99`; code treating hop > 1 as already handled could skip policy checks and usage accounting.

`lr-types/src/trace.rs::RequestTrace::outbound_for` now keeps the trace ID for correlation while resetting its hop to 1. Module/API documentation explains that a wire header is not proof of prior authorization. `next_hop` and task-local trusted internal traces remain available, but all inspected HTTP ingress call sites use `outbound_for` via the server trace middleware or proxy `stamp_trace` helper.

Regressions cover claimed hops 1, 2, 99 and u32::MAX, including propagation into spawned tasks. Proxy tests verify a forged trace still runs the firewall and counts the request, and reverse-proxy integration expectations now preserve the ID while treating the request as a fresh enforcement/accounting hop. The API agent owns the equivalent server boundary fixture.

Compatibility effect: real multi-hop wire requests can now prompt/count more than once. Authenticated request-bound provenance is required before wire-triggered deduplication can safely be restored. This deliberate tradeoff was coordinated with the root agent before editing.

#### Metrics aggregation no longer counts the same request multiple times

`lr-monitoring/src/storage.rs::get_aggregated_usage` summed minute, hourly and daily copies of the same traffic. It is used by recent strategy usage, pre-estimates and feature totals, so existing rollups could inflate limits and displayed savings. It now delegates to the shared non-overlapping totals query.

The previous supposedly deduplicated `get_usage_for_type` implementation had its own gap bug: it excluded all fine rows before the newest coarse row, even if earlier hours had never been aggregated. It could also count minute rows again when a daily row existed without hourly rows. The new query excludes a fine row only when a selected, coarser row for the same metric type covers that exact interval. A coarse row outside the requested time window does not hide fine rows inside the window.

Regressions now assert the same correct totals from both public aggregation methods and exercise an unaggregated old hour, a later hourly rollup, a daily rollup after hourly rows are removed, and a partial-window query. The production SQL was also extracted directly from the Rust source and executed against in-memory Python SQLite fixtures: all three gap/daily/partial-window assertions passed.

The final performance check caught an initial correlated-query regression on 10,000 minute-only rows, which was interrupted after three seconds. A partial covering index containing only hourly/daily rows and a bounded coverage lookup now avoid repeatedly scanning minute history or unrelated older rollups. Repeating the exact production SQL on the same synthetic fixture returned the correct totals in 0.0022 seconds with minute-only data, and 0.0262 seconds with 167 hourly and seven daily rollups also present. These are local smoke measurements, not a portable performance guarantee. SQLite's query plan confirmed use of the new partial index.

This keeps the existing bucket timestamp interpretation of query windows. It does not reconstruct intra-bucket usage once finer history has been deleted; exact partial-hour/day historical accounting would require a different retention contract.

#### Monitor callbacks can safely replace the emitter

`lr-monitor/src/store.rs` invoked create/update notification callbacks while holding the emitter read lock. A callback that called `set_emitter` would wait forever for the same lock. Both paths now clone the callback and release the lock before invocation. The regression replaces the callback from both create and update notifications; it checks lock availability first so a regression fails without hanging the test process.

### Supplemental review coverage

The root had already reviewed and tested several foundational crates. This wave is an additional structural and selected semantic pass, not a duplicate full audit.

| Crate | Supplemental inspection | Outcome |
|---|---|---|
| lr-types | Error/MCP/common helper surfaces; request trace parse/serialization/outbound/task-local propagation | Confirmed and fixed wire-header trust issue; other pure helpers retained |
| lr-monitoring | SQLite schema and rollup queries, successful/failed recordings, metrics consumers, scheduling interfaces, logger/MCP graph module inventory | Fixed double counting and gap handling; other areas require deeper persistence/rotation performance work |
| lr-monitor | Event count/byte budget, event size and summary interfaces, push/update/list, completion guard and truncation contracts | Fixed emitter callback reentrancy deadlock. Existing guard marks abandoned events failed. Single oversized newest event remains intentionally allowed by documented policy |
| lr-clients | Client CRUD/secret-store sequencing, enabled checks, synchronization interfaces and in-memory token generation/expiry/revocation | No real secrets/keychain calls performed. Token store uses cryptographic random keys and expiry checks; further atomicity/error-recovery review remains useful |
| lr-utils | Crypto key generation, paths, install-source detection, host invocation, shell-path timeout and binary-discovery interfaces | No additional concrete correction isolated beyond root's work; platform-specific helpers not exercised live |
| lr-json-repair | Public repair options, streaming frame/state interfaces, finish/action handling and pure test structure | No model-dependent behavior; root's CPU suite supplies runtime coverage. Exhaustive schema/fuzz testing not performed |
| lr-catalog | Build-time cache switch, fetch/generation interfaces, model matcher exact/alias/prefix/date-suffix stages, generated catalog boundary | No source/snapshot changes. Prefix/name matches can still be ambiguous across providers; deterministic matching policy deserves separate design and pricing fixtures |

### Validation and boundaries

- No GPU, inference, model download, provider call, app launch, real credential inspection or live account mutation.
- First-wave library binaries passed 800 tests; those results are recorded in `mcp-tools-context.md`.
- Stable rustfmt check passed after the trace and metrics edits.
- Extracted production SQL passed three in-memory SQLite cases for rollup gaps/daily-only/partial-window selection.
- Extracted production SQL also returned correct totals for 10,000 minute-only rows and the same rows with overlapping hourly/daily rollups; query-plan inspection confirmed indexed coverage lookup.
- Final stable Rust 1.99.0 tests passed: lr-types 29, lr-monitoring 45, lr-monitor 33. These include the final trace, indexed metrics query and emitter regressions. The shared run also passed lr-proxy 73, lr-marketplace 29 and lr-local-models 88: 297 total, zero failures. Workspace Clippy is recorded in the consolidated report.
- No commits were made; the pre-existing catalog JSON modification was preserved.

### Follow-ups not represented as completed work

- Authenticated cross-process trace provenance before re-enabling wire-based enforcement/accounting deduplication.
- Rollup backfill after long application downtime; failed aggregation retries; exact reporting for windows whose leading/trailing buckets have lost fine detail.
- Client deletion/secret rotation recovery if secret-store operations fail, and capacity control for generated tokens.
- Deterministic catalog prefix matching and ambiguity handling, with provider-specific pricing expectations. No live pricing assertions were made.
- Exhaustive repair-parser/schema fuzzing and cross-platform binary-discovery/install-source testing remain separate work.

### Supplemental source inventory

| Crate | First-party Rust files | Rust lines at end of this wave |
|---|---:|---:|
| lr-monitoring | 9 | 4,961 |
| lr-monitor | 7 | 2,897 |
| lr-json-repair | 3 | 2,020 |
| lr-types | 5 | 688 |
| lr-utils | 7 | 1,768 |
| lr-clients | 3 | 1,165 |
| lr-catalog | 8 | 1,712 |

Generated `lr-catalog/catalog/catalog.rs` and the raw catalog snapshot are excluded from first-party review counts.

### Supplemental file inventory

These paths were included in the structural pass. Only the three changed modules received the detailed fixes described above; other modules were sampled at their interfaces and selected stateful operations.

| File | Review depth |
|---|---|
| `crates/lr-monitoring/src/aggregation_task.rs` | Structural / sampled |
| `crates/lr-monitoring/src/graphs.rs` | Structural / sampled |
| `crates/lr-monitoring/src/lib.rs` | Structural / sampled |
| `crates/lr-monitoring/src/logger.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_graphs.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_logger.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_metrics.rs` | Structural / sampled |
| `crates/lr-monitoring/src/metrics.rs` | Structural / sampled |
| `crates/lr-monitoring/src/storage.rs` | Changed / focused review |
| `crates/lr-monitor/src/guard.rs` | Structural / sampled |
| `crates/lr-monitor/src/lib.rs` | Structural / sampled |
| `crates/lr-monitor/src/size.rs` | Structural / sampled |
| `crates/lr-monitor/src/store.rs` | Changed / focused review |
| `crates/lr-monitor/src/summary.rs` | Structural / sampled |
| `crates/lr-monitor/src/truncate.rs` | Structural / sampled |
| `crates/lr-monitor/src/types.rs` | Structural / sampled |
| `crates/lr-json-repair/src/lib.rs` | Structural / sampled |
| `crates/lr-json-repair/src/streaming.rs` | Structural / sampled |
| `crates/lr-json-repair/src/types.rs` | Structural / sampled |
| `crates/lr-types/src/errors.rs` | Structural / sampled |
| `crates/lr-types/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-types/src/lib.rs` | Structural / sampled |
| `crates/lr-types/src/mcp_types.rs` | Structural / sampled |
| `crates/lr-types/src/trace.rs` | Changed / focused review |
| `crates/lr-utils/src/binary.rs` | Structural / sampled |
| `crates/lr-utils/src/crypto.rs` | Structural / sampled |
| `crates/lr-utils/src/install_source.rs` | Structural / sampled |
| `crates/lr-utils/src/lib.rs` | Structural / sampled |
| `crates/lr-utils/src/paths.rs` | Structural / sampled |
| `crates/lr-utils/src/sandbox.rs` | Structural / sampled |
| `crates/lr-utils/src/test_mode.rs` | Structural / sampled |
| `crates/lr-clients/src/lib.rs` | Structural / sampled |
| `crates/lr-clients/src/manager.rs` | Structural / sampled |
| `crates/lr-clients/src/token_store.rs` | Structural / sampled |
| `crates/lr-catalog/build.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/codegen.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/mod.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/models.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/scraper.rs` | Structural / sampled |
| `crates/lr-catalog/src/lib.rs` | Structural / sampled |
| `crates/lr-catalog/src/matcher.rs` | Structural / sampled |
| `crates/lr-catalog/src/types.rs` | Structural / sampled |

---

## Appendix 5: Desktop frontend and website review — 2026-10-01

Source report: `plan/review-2026-10-01/frontend-website.md`.


### Scope, constraints, and coverage

Owned areas: `src/`, `website/`, root frontend build configuration and `package.json`, and `tests/e2e/`. Read `CLAUDE.md` first. The parent agent owns the repository-wide plan and final aggregation. No commits, dependency installation, model downloads, external requests, inference, browser launches, desktop launches, or GPU execution were performed by this reviewer. CPU-only TypeScript compilation, bundling, Node tests, source parsing, and temporary filesystem fixtures were used.

The machine-readable inventory is `frontend-inventory.json`. It records **268 TypeScript/TSX files, 79,230 lines, 254 useEffect calls and 482 direct invoke calls** at inventory time. Every listed file was parsed with the installed TypeScript parser and included in a repository-wide pattern audit for asynchronous work, events/listeners, storage, network calls, unsafe rendering/evaluation, external assets, URL handling, and resource ownership. The parser reported **zero syntax diagnostics**. Desktop and website baselines independently passed `tsc --noEmit`.

This is not a claim that every line received an equally deep manual review. Files marked **D** below received direct semantic review of the entire small module or the relevant behavior and its callers; **S** means structural/pattern/type/build review, with manual inspection of selected matched code where applicable. Large components, data catalogs and demo handlers have many untouched behaviors. Generated Windows XP bundles, sourcemaps, Playwright reports and dependency lockfiles were inventoried as artifacts, not audited as original implementation or edited.

### Implemented improvements

#### F01 — Credential isolation when switching Try It Out clients

Files: `src/views/try-it-out/llm-tab/index.tsx`, `src/views/try-it-out/mcp-tab/index.tsx`.

Previously the selected client changed before the asynchronous `get_client_value` response arrived, leaving the previous client's credential usable in the meantime. Responses could also arrive out of order and install a credential for the wrong selection. Credentials are now stored together with their owning client ID; only a matching credential can be used, pending lookups clear the state, and obsolete responses are ignored. The LLM default selection now chooses from enabled clients instead of selecting a disabled first entry.

#### F02 — Incremental model refresh ordering and cache races

File: `src/hooks/useIncrementalModels.ts`.

Refresh previously started before asynchronous event listener registration, so fast local/cache responses could disappear before listeners existed. Refresh now waits for registration promises. A delayed cached-model response no longer overwrites newer per-provider results; the merge preserves refreshed providers, including an intentionally empty refreshed list. Effect-local cancellation prevents an old StrictMode lifecycle from writing through a reused mounted ref. The effect now tracks its actual refresh dependencies.

#### F03 — Dynamic event names follow hook props

File: `src/hooks/useTauriListener.ts`.

The hook previously only depended on caller-supplied dependencies, leaving it subscribed to an old event name if the event prop changed. Event names now participate in the dependency list; existing safe listener teardown and latest-handler refs are retained.

#### F04 — Download startup failures and event handling

File: `src/hooks/useModelDownload.ts`.

An invoke rejection was ignored whenever a separate failure event was configured, even when startup failed before emitting any event. Invoke errors now transition to failed in either configuration, with duplicate failure callbacks suppressed. Event filters and progress normalizers use current refs rather than stale mount-time closures. Progress rejects non-finite values and clamps to 0–100. A successful completion clears stale error text. Lifecycle behavior across arbitrary model identity changes still deserves a mounted integration suite; no download was executed.

#### F05 — Monitor snapshot/live-event correctness

Files: `src/views/monitor/hooks/useMonitorEvents.ts`, new `src/views/monitor/monitor-events.ts`.

Initial/filter-change snapshots could overwrite live events, older filter queries could replace newer results, duplicate events could accumulate, and an update newly matching a status filter was never inserted. A bounded merge now gives live updates precedence, deduplicates by ID, applies the active predicate, sorts by sequence, and inserts newly matching updates. An in-flight query records intervening live updates and merges them into its snapshot. Superseded queries and queries invalidated by Clear cannot repopulate the list.

#### F06 — Monitor detail request races

File: `src/views/monitor/hooks/useMonitorEvents.ts`.

Selecting B while A was loading could show A's detail under B's highlighted row. Selection now updates its identity ref synchronously, clears the old detail immediately, and applies only the latest matching detail response. Update-driven detail refreshes use the same guard. Clearing/unmounting invalidates pending detail requests; a missing detail clears the panel instead of retaining another event.

#### F07 — MCP connection ownership and cleanup

File: `src/lib/mcp-client.ts`.

Connections now use attempt-local SDK client/transport references and a generation counter. Disconnect invalidates and detaches synchronously before waiting for close; a late handshake cannot reset or close a newer connection. Failed handshakes close their resources and expose the failure without leaving stale state. Transport closure resets connection metadata and subscriptions. Resource-read callbacks verify that both the connection and subscription are still current. The already-declared roots capability now has an explicit empty `roots/list` handler, avoiding a capability/handler mismatch.

#### F08 — Complete MCP pagination and subscription rollback

File: `src/lib/mcp-client.ts`.

Tools, resources and prompts previously returned only the first page. They now consume every cursor, including an empty-string cursor, and reject repeated cursors instead of looping. Subscription failure rolls back the locally installed callback; an unsubscribe only drops its callback after the server accepts it, avoiding a local/server mismatch on rejection.

#### F09 — Remove raw sampling/elicitation payload logging

File: `src/lib/mcp-client.ts`.

Sampling prompts and elicitation inputs no longer get copied wholesale into the developer console. Other existing diagnostic logs remain; this change is not a claim that all logging in the product has been redacted.

#### F10 — Responses SSE decoding and connection release

Files: new `src/lib/sse.ts`, `src/views/try-it-out/llm-tab/chat-panel.tsx`.

The handwritten Responses parser only recognized LF framing and retained its reader on consumer errors. A shared async generator now handles LF/CRLF boundaries split across arbitrary chunks, streaming UTF-8, multi-line data and significant whitespace. It ignores comment-only frames, cancels the stream and releases the reader when the consumer ends or throws. Unterminated final frames are not dispatched. CR-only line endings are not implemented.

#### F11 — Chat cancellation cannot clear a new request

File: `src/views/try-it-out/llm-tab/chat-panel.tsx`.

After Stop then Send, the old request's `finally` could null out the new abort controller and reset its loading indicator. Each request now retains its own controller and only clears the shared ref if it still owns it. A synchronous controller guard also blocks two sends before React updates loading state.

#### F12 — Image preview allocation and speech cleanup

Files: `src/views/try-it-out/llm-tab/images-panel.tsx`, `src/views/try-it-out/llm-tab/speech-panel.tsx`.

Image inputs allocated URLs before applying the 16-image limit, leaking discarded previews; allocations inside state updaters could also run twice under StrictMode. Allocation now happens outside updaters and only for retained files, with refs synchronized during additions/removals/mask replacement. Editing a result at the limit no longer allocates a discarded URL. Speech requests use an abort controller, stop on unmount and check cancellation before creating blob URLs, so a late response cannot create an unreachable URL after cleanup. Generated speech arrays remain user-session history rather than a new retention policy.

#### F13 — Theme resilience and system-theme reactivity

Files: `src/hooks/use-theme.ts`, `website/src/hooks/use-theme.ts`.

Unavailable localStorage and corrupted stored values could crash initialization or produce an invalid theme cycle. Both hooks validate the stored enum and tolerate unavailable reads/writes. System-theme changes now update React state as well as the root class, keeping theme-dependent controls consistent.

#### F14 — Key/value editor respects externally loaded data

File: `src/components/ui/KeyValueInput.tsx`.

Local rows were initialized once, so changing the controlled resource/config could leave stale keys and secrets displayed. External changes now refresh the rows while echoing the component's own unfinished rows does not erase them. Edits clone the changed row instead of mutating an existing state object. `Object.fromEntries` preserves literal keys such as `__proto__` as data. The inputs also receive accessible names.

#### F15 — Website development icon path confinement

Files: `website/vite.config.ts`, new `website/shared-icons.ts`.

The custom `/icons` middleware joined an arbitrary request path onto the public directory and could expose sibling files through traversal. It now accepts flat image filenames, rejects malformed escapes/separators/nulls, resolves real paths and rejects symlinks escaping the icon directory. A disappearing/unreadable file stream ends with an error response rather than emitting an unhandled stream error. Supported MIME mappings now include JPEG and WebP.

#### F16 — Website mock event lifecycle

File: `website/src/stubs/tauri-api-event.ts`.

An event queued before `unlisten` still called the removed handler, and two already queued events both called a once-listener. Delivery now verifies that the exact listener entry remains registered. Empty event sets are removed, and cleanup of an old listener does not remove a replacement registration.

#### F17 — External-link scheme checks and opener isolation

Files: `src/components/shared/FirewallApprovalCard.tsx`, `website/src/stubs/tauri-plugin-shell.ts`.

The marketplace approval source link was an unvalidated external string passed to window.open, unlike the already validated marketplace browse links. It now uses the existing HTTP/HTTPS validator and isolates the opener. The website shell stub applies the same scheme restriction and opener isolation. URLs are still opened only by the corresponding user action.

#### F18 — Standalone CPU regression entry point

Files: `package.json`, new `tests/e2e/unit.config.ts`, `tests/e2e/unit/*.spec.ts`.

Added `npm run test:unit` using the repository's existing Playwright dependency, with a separate configuration that has no app/global setup, browser fixtures or browser install requirement. Tests use pure functions, temporary files, in-memory event delivery, mocked MCP SDK boundaries and Web Streams. The parent agent wired this command into CI.

#### F19 — Fixed Vite port matches Tauri's configured dev URL

File: `vite.config.ts`.

Tauri always opens port 1420, but Vite previously selected another port if 1420 was occupied. `strictPort: true` now fails clearly instead of starting a frontend at a URL Tauri will not load. The associated Tauri media CSP and approval-denial changes are detailed in `tauri-ui.md`.

### Verification ledger

- Baseline `npx --no-install tsc --noEmit`: exit 0, no diagnostics.
- Baseline `npx --no-install tsc --noEmit -p website/tsconfig.json`: exit 0, no diagnostics.
- Structural parser inventory: 268 files, 79,230 lines, zero syntax diagnostics.
- Initial 14-test run: 13 passed, one test expectation failed because macOS canonicalizes `/var` to `/private/var`. The production path resolver correctly returned a canonical path. The test now compares real paths.
- Second 14-test run: **14 passed (958ms)**.
- Expanded suite `npm run test:unit`: **17 passed (21.3s)**, with 2 workers and no browser/app setup.
- Intermediate production builds caught an accidental out-of-scope abortController reference in Stop; corrected before final validation.
- Final passing production build results are recorded below.
- `git diff --check` has passed during implementation; final verification repeated before handoff.

The 17 behavioral cases cover monitor status transitions/filtering/order/deduplication/snapshot merge, icon traversal and symlink escapes, mock once/unlisten semantics, URL scheme rejection, MCP complete pagination/repeated cursors/handshake cleanup/reconnect ownership/transport closure, and SSE framing/UTF-8/multiline/reader cleanup. React mounted behavior, actual native-window interactions, provider requests, UI screenshots and GPU behavior were not exercised.

### Areas inspected without a separate patch

- Existing URL helper already correctly rejected non-HTTP(S) schemes; its established validated callers were retained.
- OpenAI wrapper confines configuration to supplied base URL/key and intentionally permits browser context for desktop/local use; no secret is introduced into static website assets by this wrapper itself.
- Radix modal/switch wrappers and information tooltip already clean up tooltip timers and delegate dialog/switch interaction behavior to installed primitives.
- Connection graph activity expiration deletes aged entries and clears its timer; no new timer leak was found in the inspected implementation.
- Static docs/research content uses react-markdown without raw HTML execution. Pattern matches for `eval` and `innerHTML` were mock code samples, not executing calls. No live source `dangerouslySetInnerHTML`, `document.write`, `eval`, `new Function`, or remotely loaded script/image/style literals were found by the selected audit expressions.
- Permission state controls, wizard creation flow, rate-limit editor and server settings received selected semantic review. Full accessibility audits and every rapid-toggle/autosave interleaving remain outside demonstrated coverage.
- Existing desktop E2E setup builds/launches a Tauri application and creates an Ollama provider; it was deliberately not run under the no-GPU/no-app verification constraints. Its use of a fixed home-directory test config and legacy fixture shape remains a candidate for a future isolated E2E overhaul.

### Remaining risks and follow-up evidence needed

1. Several large resource/client/settings components still have independent asynchronous loads and debounced saves. A consistent request-generation pattern plus a mounted test harness would provide broader race coverage than the focused fixes here.
2. Model-download identity changes and events from externally initiated/retried jobs have no explicit job ID in this generic hook; the new error handling cannot prove cross-job ordering without backend contracts and mounted integration tests.
3. MCP WebSocket support exists in the wrapper but the active UI selects SSE. Browser WebSocket authentication behavior and real SSE reconnection were not exercised.
4. Some frontend error branches log/return generic errors; all backend string-error conversions were not rewritten merely for consistency.
5. Persisted monitor filters are parsed as typed JSON without a full runtime enum schema. Malformed manually edited storage deserves additional mounted coverage.
6. Speech/history memory, very large image attachments, huge event payloads and endless SSE streams need a separate product decision on limits; the fixes do not silently impose new user-visible quotas.
7. Generated Windows XP assets are sourced from another repository. Their original source and browser behavior were not reviewed or rebuilt.
8. Browser-only keyboard/focus/accessibility behavior and native OAuth/popup workflows need a future explicitly permitted interactive run.

### File inventory

Legend: **D** = direct semantic review of the relevant implementation/callers; **S** = structural/pattern/type/build coverage. Counts below are the captured inventory, not a claim of every-line manual inspection.

| File | Lines | Coverage |
|---|---:|---|
| `src/lib/sse.ts` | 26 | D |
| `src/views/monitor/monitor-events.ts` | 27 | D |
| `tests/e2e/unit.config.ts` | 11 | D |
| `tests/e2e/unit/frontend-regressions.spec.ts` | 83 | D |
| `tests/e2e/unit/mcp-client.spec.ts` | 98 | D |
| `tests/e2e/unit/sse.spec.ts` | 39 | D |
| `website/shared-icons.ts` | 18 | D |
| `src/App.tsx` | 401 | S |
| `src/components/Logo.tsx` | 29 | S |
| `src/components/McpServerIcon.tsx` | 15 | S |
| `src/components/OAuthSettingsControls.tsx` | 318 | S |
| `src/components/ProviderForm.tsx` | 565 | S |
| `src/components/ProviderIcon.tsx` | 15 | S |
| `src/components/ServiceIcon.tsx` | 366 | S |
| `src/components/add-resource/DisabledOverlay.tsx` | 40 | S |
| `src/components/add-resource/MarketplaceSearchPanel.tsx` | 452 | S |
| `src/components/add-resource/index.ts` | 9 | S |
| `src/components/client/ClientModeSelector.tsx` | 310 | S |
| `src/components/client/ClientTemplates.tsx` | 766 | S |
| `src/components/client/HowToConnect.tsx` | 1897 | S |
| `src/components/client/ProxyAllowedModels.tsx` | 129 | S |
| `src/components/compression/types.ts` | 37 | S |
| `src/components/connection-graph/ConnectionGraph.tsx` | 207 | S |
| `src/components/connection-graph/hooks/useGraphData.ts` | 160 | D |
| `src/components/connection-graph/index.ts` | 4 | S |
| `src/components/connection-graph/nodes/AccessKeyNode.tsx` | 58 | S |
| `src/components/connection-graph/nodes/CodingAgentNode.tsx` | 41 | S |
| `src/components/connection-graph/nodes/EndpointNode.tsx` | 59 | S |
| `src/components/connection-graph/nodes/MarketplaceNode.tsx` | 41 | S |
| `src/components/connection-graph/nodes/McpServerNode.tsx` | 67 | S |
| `src/components/connection-graph/nodes/ProviderNode.tsx` | 67 | S |
| `src/components/connection-graph/nodes/RouterGroupNode.tsx` | 15 | S |
| `src/components/connection-graph/nodes/SkillNode.tsx` | 41 | S |
| `src/components/connection-graph/types.ts` | 171 | S |
| `src/components/connection-graph/utils/buildGraph.ts` | 810 | S |
| `src/components/guardrails/SafetyModelPicker.tsx` | 276 | S |
| `src/components/icons/category-icons.tsx` | 91 | S |
| `src/components/layout/BugReportDialog.tsx` | 174 | S |
| `src/components/layout/app-shell.tsx` | 209 | S |
| `src/components/layout/command-palette.tsx` | 381 | S |
| `src/components/layout/header.tsx` | 102 | S |
| `src/components/layout/index.tsx` | 4 | S |
| `src/components/layout/sidebar.tsx` | 903 | S |
| `src/components/mcp/McpOAuthModal.tsx` | 246 | D |
| `src/components/mcp/McpServerTemplates.tsx` | 530 | S |
| `src/components/permissions/CategoryActionButton.tsx` | 94 | S |
| `src/components/permissions/ClientToolsIndexingTree.tsx` | 159 | S |
| `src/components/permissions/GatewayIndexingTree.tsx` | 162 | S |
| `src/components/permissions/IndexingStateButton.tsx` | 163 | S |
| `src/components/permissions/McpPermissionTree.tsx` | 273 | S |
| `src/components/permissions/ModelsPermissionTree.tsx` | 157 | S |
| `src/components/permissions/PermissionStateButton.tsx` | 96 | D |
| `src/components/permissions/PermissionTreeSelector.tsx` | 284 | S |
| `src/components/permissions/SkillsPermissionTree.tsx` | 203 | S |
| `src/components/permissions/VirtualMcpIndexingTree.tsx` | 125 | S |
| `src/components/permissions/index.ts` | 14 | S |
| `src/components/permissions/types.ts` | 44 | D |
| `src/components/providers/EmbeddedEngineTab.tsx` | 594 | S |
| `src/components/providers/EngineModelsTab.tsx` | 321 | S |
| `src/components/providers/HuggingFaceAccountCard.tsx` | 259 | S |
| `src/components/providers/LocalModelsTab.tsx` | 1101 | S |
| `src/components/routellm/ThresholdSelector.tsx` | 291 | S |
| `src/components/routellm/types.ts` | 40 | S |
| `src/components/shared/ContentStorePreview.tsx` | 384 | D |
| `src/components/shared/ExperimentalBadge.tsx` | 20 | S |
| `src/components/shared/FeatureClientsCard.tsx` | 116 | S |
| `src/components/shared/FirewallApprovalCard.tsx` | 761 | D |
| `src/components/shared/McpToolDisplay.tsx` | 192 | S |
| `src/components/shared/ModelDownloadCard.tsx` | 108 | S |
| `src/components/shared/RefreshModelsButton.tsx` | 70 | S |
| `src/components/shared/SamplePopupButton.tsx` | 32 | S |
| `src/components/shared/SystemOneAnswers.tsx` | 268 | S |
| `src/components/shared/feature-support-matrix.tsx` | 132 | S |
| `src/components/shared/metrics-chart.tsx` | 509 | S |
| `src/components/shared/model-pricing-badge.tsx` | 187 | S |
| `src/components/shared/stats-card.tsx` | 138 | S |
| `src/components/shared/support-level-badge.tsx` | 107 | S |
| `src/components/strategies/RateLimitEditor.tsx` | 248 | D |
| `src/components/strategy/AllowedModelsSelector.tsx` | 357 | S |
| `src/components/strategy/DragThresholdModelSelector.tsx` | 812 | S |
| `src/components/strategy/PrioritizedModelSelector.tsx` | 346 | S |
| `src/components/strategy/StrategyModelConfiguration.tsx` | 793 | S |
| `src/components/strategy/ThreeZoneModelSelector.tsx` | 1254 | S |
| `src/components/strategy/UnifiedModelsSelector.tsx` | 524 | S |
| `src/components/strategy/index.ts` | 21 | S |
| `src/components/ui/Badge.tsx` | 48 | S |
| `src/components/ui/Button.tsx` | 62 | S |
| `src/components/ui/Card.tsx` | 82 | S |
| `src/components/ui/Input.tsx` | 49 | S |
| `src/components/ui/KeyValueInput.tsx` | 108 | D |
| `src/components/ui/Modal.tsx` | 145 | D |
| `src/components/ui/PresetSlider.tsx` | 101 | S |
| `src/components/ui/Select.tsx` | 200 | S |
| `src/components/ui/Slider.tsx` | 25 | S |
| `src/components/ui/Toggle.tsx` | 49 | D |
| `src/components/ui/TriStateButton.tsx` | 85 | S |
| `src/components/ui/alert-dialog.tsx` | 139 | S |
| `src/components/ui/alert.tsx` | 60 | S |
| `src/components/ui/checkbox.tsx` | 36 | S |
| `src/components/ui/collapsible.tsx` | 9 | S |
| `src/components/ui/command.tsx` | 157 | S |
| `src/components/ui/dialog.tsx` | 119 | S |
| `src/components/ui/dropdown-menu.tsx` | 198 | S |
| `src/components/ui/info-tooltip.tsx` | 92 | D |
| `src/components/ui/label.tsx` | 24 | S |
| `src/components/ui/popover.tsx` | 29 | S |
| `src/components/ui/progress.tsx` | 25 | S |
| `src/components/ui/radio-group.tsx` | 42 | S |
| `src/components/ui/resizable.tsx` | 74 | S |
| `src/components/ui/scroll-area.tsx` | 46 | S |
| `src/components/ui/separator.tsx` | 29 | S |
| `src/components/ui/skeleton.tsx` | 15 | S |
| `src/components/ui/sonner.tsx` | 48 | S |
| `src/components/ui/switch.tsx` | 31 | S |
| `src/components/ui/table.tsx` | 117 | S |
| `src/components/ui/tabs.tsx` | 53 | S |
| `src/components/ui/textarea.tsx` | 24 | S |
| `src/components/ui/tooltip.tsx` | 28 | S |
| `src/components/wizard/ClientCreationWizard.tsx` | 289 | D |
| `src/components/wizard/steps/StepNameAndMode.tsx` | 78 | S |
| `src/components/wizard/steps/StepTemplate.tsx` | 44 | S |
| `src/components/wizard/steps/StepWelcome.tsx` | 64 | S |
| `src/constants/features.ts` | 44 | S |
| `src/constants/safety-model-variants.ts` | 57 | S |
| `src/constants/tab-icons.ts` | 45 | S |
| `src/hooks/use-theme.ts` | 82 | D |
| `src/hooks/useIncrementalModels.ts` | 106 | D |
| `src/hooks/useMetricsSubscription.ts` | 25 | D |
| `src/hooks/useModelDownload.ts` | 164 | D |
| `src/hooks/useTauriListener.ts` | 99 | D |
| `src/lib/mcp-client.ts` | 543 | D |
| `src/lib/openai-client.ts` | 19 | D |
| `src/lib/utils.ts` | 6 | S |
| `src/main.tsx` | 26 | S |
| `src/types/systemone.ts` | 93 | D |
| `src/types/tauri-commands.ts` | 4383 | S |
| `src/utils/errors.ts` | 31 | D |
| `src/utils/url.ts` | 16 | D |
| `src/views/catalog-compression/index.tsx` | 1109 | S |
| `src/views/clients/client-detail.tsx` | 296 | S |
| `src/views/clients/index.tsx` | 320 | S |
| `src/views/clients/tabs/coding-agents-tab.tsx` | 191 | S |
| `src/views/clients/tabs/compression-tab.tsx` | 124 | S |
| `src/views/clients/tabs/config-tab.tsx` | 80 | S |
| `src/views/clients/tabs/context-tab.tsx` | 199 | S |
| `src/views/clients/tabs/guardrails-tab.tsx` | 238 | S |
| `src/views/clients/tabs/info-tab.tsx` | 321 | S |
| `src/views/clients/tabs/json-repair-tab.tsx` | 131 | S |
| `src/views/clients/tabs/llm-optimize-tab.tsx` | 37 | S |
| `src/views/clients/tabs/marketplace-tab.tsx` | 82 | S |
| `src/views/clients/tabs/mcp-tab.tsx` | 66 | S |
| `src/views/clients/tabs/memory-tab.tsx` | 174 | S |
| `src/views/clients/tabs/models-tab-legacy.tsx` | 439 | S |
| `src/views/clients/tabs/secret-scanning-tab.tsx` | 274 | S |
| `src/views/clients/tabs/settings-tab.tsx` | 326 | D |
| `src/views/clients/tabs/skills-tab.tsx` | 106 | S |
| `src/views/clients/tabs/unified-models-tab.tsx` | 982 | S |
| `src/views/coding-agents/index.tsx` | 1175 | S |
| `src/views/compression/index.tsx` | 842 | S |
| `src/views/dashboard/index.tsx` | 617 | S |
| `src/views/debug/index.tsx` | 301 | S |
| `src/views/elicitation-form.tsx` | 271 | D |
| `src/views/firewall-approval.tsx` | 978 | D |
| `src/views/guardrails/guardrails-panel.tsx` | 344 | S |
| `src/views/guardrails/index.tsx` | 650 | S |
| `src/views/json-repair/index.tsx` | 516 | S |
| `src/views/marketplace/index.tsx` | 1242 | S |
| `src/views/mcp-servers/index.tsx` | 181 | S |
| `src/views/mcp-servers/mcp-settings-panel.tsx` | 158 | S |
| `src/views/memory/index.tsx` | 586 | S |
| `src/views/memory/sessions-tab.tsx` | 812 | S |
| `src/views/monitor/event-detail.tsx` | 1782 | S |
| `src/views/monitor/event-filters.tsx` | 329 | D |
| `src/views/monitor/event-list.tsx` | 166 | S |
| `src/views/monitor/hooks/useMonitorEvents.ts` | 124 | D |
| `src/views/monitor/index.tsx` | 164 | D |
| `src/views/monitor/try-it-out-panel.tsx` | 125 | S |
| `src/views/optimize-overview/OptimizeDiagram.tsx` | 109 | S |
| `src/views/optimize-overview/index.tsx` | 346 | S |
| `src/views/resources/compatibility-panel.tsx` | 193 | S |
| `src/views/resources/index.tsx` | 176 | S |
| `src/views/resources/mcp-servers-panel.tsx` | 1637 | S |
| `src/views/resources/models-panel.tsx` | 427 | S |
| `src/views/resources/providers-panel.tsx` | 2073 | S |
| `src/views/response-rag/index.tsx` | 546 | S |
| `src/views/sampling-approval.tsx` | 198 | D |
| `src/views/secret-scanning/index.tsx` | 486 | S |
| `src/views/settings/appearance-tab.tsx` | 689 | S |
| `src/views/settings/general-tab.tsx` | 142 | S |
| `src/views/settings/health-checks-tab.tsx` | 66 | S |
| `src/views/settings/index.tsx` | 67 | S |
| `src/views/settings/licenses-tab.tsx` | 110 | S |
| `src/views/settings/logging-tab.tsx` | 722 | S |
| `src/views/settings/server-tab.tsx` | 370 | D |
| `src/views/settings/updates-tab.tsx` | 501 | S |
| `src/views/skills/index.tsx` | 1008 | S |
| `src/views/strong-weak/index.tsx` | 400 | S |
| `src/views/try-it-out/guardrails-tab/index.tsx` | 514 | S |
| `src/views/try-it-out/llm-tab/chat-panel.tsx` | 763 | D |
| `src/views/try-it-out/llm-tab/embeddings-panel.tsx` | 250 | S |
| `src/views/try-it-out/llm-tab/images-panel.tsx` | 560 | D |
| `src/views/try-it-out/llm-tab/index.tsx` | 1069 | D |
| `src/views/try-it-out/llm-tab/speech-panel.tsx` | 294 | D |
| `src/views/try-it-out/llm-tab/systemone-panel.tsx` | 723 | S |
| `src/views/try-it-out/llm-tab/transcribe-panel.tsx` | 393 | S |
| `src/views/try-it-out/mcp-tab/connection-info-panel.tsx` | 203 | S |
| `src/views/try-it-out/mcp-tab/elicitation-panel.tsx` | 486 | S |
| `src/views/try-it-out/mcp-tab/index.tsx` | 968 | D |
| `src/views/try-it-out/mcp-tab/prompts-panel.tsx` | 347 | S |
| `src/views/try-it-out/mcp-tab/resources-panel.tsx` | 379 | S |
| `src/views/try-it-out/mcp-tab/sampling-panel.tsx` | 531 | S |
| `src/views/try-it-out/mcp-tab/tools-panel.tsx` | 426 | S |
| `src/vite-env.d.ts` | 1 | S |
| `tests/e2e/fixtures/test-helpers.ts` | 169 | D |
| `tests/e2e/global-setup.ts` | 116 | D |
| `tests/e2e/global-teardown.ts` | 20 | D |
| `tests/e2e/playwright.config.ts` | 24 | D |
| `tests/e2e/specs/client-name-change.spec.ts` | 146 | S |
| `vite.config.ts` | 70 | D |
| `website/src/App.tsx` | 53 | S |
| `website/src/components/ArchitectureDiagram.tsx` | 442 | S |
| `website/src/components/ElicitationDemo.tsx` | 77 | S |
| `website/src/components/FirewallApprovalDemo.tsx` | 124 | S |
| `website/src/components/Footer.tsx` | 100 | S |
| `website/src/components/FreeTierFallbackDemo.tsx` | 25 | S |
| `website/src/components/GuardrailApprovalDemo.tsx` | 47 | S |
| `website/src/components/Logo.tsx` | 29 | S |
| `website/src/components/McpViaLlmDiagram.tsx` | 185 | S |
| `website/src/components/Navigation.tsx` | 148 | S |
| `website/src/components/SecretScanApprovalDemo.tsx` | 38 | S |
| `website/src/components/demo/DemoBanner.tsx` | 9 | S |
| `website/src/components/demo/LocalRouterDemo.tsx` | 36 | D |
| `website/src/components/demo/MacOSMenuBar.tsx` | 43 | S |
| `website/src/components/demo/MacOSTrayMenu.tsx` | 370 | S |
| `website/src/components/demo/MacOSWindow.tsx` | 38 | S |
| `website/src/components/demo/TauriMockSetup.ts` | 4779 | S |
| `website/src/components/demo/index.ts` | 5 | S |
| `website/src/components/demo/mockData.ts` | 1421 | S |
| `website/src/components/docs/DocEmbeds.tsx` | 59 | S |
| `website/src/components/docs/MarketplaceDemo.tsx` | 113 | S |
| `website/src/components/docs/MarketplaceInstallDemo.tsx` | 30 | S |
| `website/src/components/docs/MetricsDemo.tsx` | 173 | S |
| `website/src/components/docs/ModelRoutingDemo.tsx` | 55 | S |
| `website/src/components/ui/Badge.tsx` | 43 | S |
| `website/src/components/ui/Button.tsx` | 53 | S |
| `website/src/components/ui/Card.tsx` | 78 | S |
| `website/src/components/ui/dropdown-menu.tsx` | 198 | S |
| `website/src/hooks/use-theme.ts` | 82 | D |
| `website/src/lib/utils.ts` | 6 | S |
| `website/src/main.tsx` | 10 | S |
| `website/src/pages/Demo.tsx` | 40 | S |
| `website/src/pages/Docs.tsx` | 827 | S |
| `website/src/pages/Download.tsx` | 154 | D |
| `website/src/pages/Home.tsx` | 2072 | S |
| `website/src/pages/Research.tsx` | 520 | S |
| `website/src/pages/docs-content.ts` | 26 | S |
| `website/src/pages/research-content.ts` | 23 | S |
| `website/src/stubs/openai.ts` | 202 | S |
| `website/src/stubs/tauri-api-core.ts` | 14 | S |
| `website/src/stubs/tauri-api-event.ts` | 74 | D |
| `website/src/stubs/tauri-api-mocks.ts` | 12 | S |
| `website/src/stubs/tauri-api-webviewWindow.ts` | 7 | S |
| `website/src/stubs/tauri-plugin-dialog.ts` | 6 | S |
| `website/src/stubs/tauri-plugin-process.ts` | 3 | S |
| `website/src/stubs/tauri-plugin-shell.ts` | 19 | D |
| `website/src/stubs/tauri-plugin-updater.ts` | 8 | S |
| `website/src/vite-env.d.ts` | 1 | S |
| `website/vite.config.ts` | 147 | D |

#### Additional configuration/content/artifact coverage

Root `package.json`, `tsconfig.json`, `tsconfig.node.json`, `tailwind.config.js`, `postcss.config.js`, `index.html`; website package/config/HTML files; `src/index.css`, `website/src/index.css`; website documentation/research Markdown and public assets received configuration or structural/static-pattern review. Lockfiles and generated `website/public/winxp/**`/Playwright HTML reports were left intact. `CLAUDE.md` was read for conventions.

### Completed production builds

Both production builds exited 0:

```text
npm run build
> tsc && vite build
vite v6.4.1 building for production...
✓ 4071 modules transformed.
✓ built in 38.43s
```

Desktop's largest application chunk: `dist/assets/index-D_dHyGdP.js`, 1,217.36 kB / 292.91 kB gzip. Existing vendor chunking is retained.

```text
(cd website && npm run build:check)
> tsc && vite build
vite v6.4.1 building for production...
✓ 6662 modules transformed.
✓ built in 34.86s
```

Website's largest chunks: `App-Cm77ePKB.js`, 1,969.87 kB / 516.83 kB gzip; `index-OwvwanIY.js`, 1,519.29 kB / 442.30 kB gzip. Both builds warn about chunks above 500 kB and seven-month-old Browserslist data. Website also reports that FirewallApprovalDemo and GuardrailApprovalDemo are both static and dynamic imports, so their dynamic references do not isolate them into separate chunks. Those are recorded performance/dependency-maintenance opportunities, not suppressed warnings or new external package updates.

The final approval-window adjustment retains controls on submission errors and permits denial with invalid edits; see U03 in `tauri-ui.md`. A follow-up desktop typecheck is tracked separately after this adjustment.

Final approval-window follow-up `npx --no-install tsc --noEmit`: exit 0, no diagnostics. Final `git diff --check`: exit 0. Final review additionally preserves an existing resource-subscription callback if its replacement is rejected and prevents stale subscribe/unsubscribe responses from modifying a newer MCP connection.

Final regression rerun after these MCP changes: `npm run test:unit` — **17 passed (15.8s)** using 2 workers. Runtime warnings were Node's `module.register()` deprecation and the runner's `NO_COLOR`/`FORCE_COLOR` precedence notice; no test failed. Modified Rust files also passed `rustfmt --edition 2021 --check` in the same verification pass.

Final desktop and website checks after all frontend source edits also exited 0 with no diagnostics: `npx --no-install tsc --noEmit` and `npx --no-install tsc --noEmit -p website/tsconfig.json`.

---

## Appendix 6: Tauri UI, native configuration and capabilities — 2026-10-01

Source report: `plan/review-2026-10-01/tauri-ui.md`.


### Scope and method

Second-wave ownership from the parent reviewer covered all files under `src-tauri/src/ui/`, `src-tauri/capabilities/`, and `src-tauri/tauri.conf.json`. Command exports, dangerous filesystem/process operations, panic markers, validation boundaries, event payload contracts, and selected lifecycle code were inventoried. Deep manual review focused on skill installation/deletion, approval edits, provider-secret handling, incremental model refresh contracts, reverse-proxy URL retargeting, local-model input validators, monitor commands, and native capability/CSP configuration. The file table below distinguishes selected semantic review from structural review. Native app/runtime behavior was not launched or claimed verified.

No command signature or serialized response shape changed; no new Tauri command was registered. Existing TypeScript/demo IPC contracts remain applicable. The shared crate install method was coordinated directly with the MCP/marketplace reviewer.

### Implemented findings

#### U01 — Direct marketplace installs bypassed safe download handling

File: `src-tauri/src/ui/commands_marketplace.rs`.

`marketplace_install_skill_direct` previously built destination paths from untrusted source/name/file strings and duplicated raw reqwest downloads. It did not reject traversal, check unsuccessful HTTP status, or enforce body limits. The UI path now calls the marketplace reviewer's shared `MarketplaceService::download_skill(&listing)` API, which validates portable single-component labels, rejects symlink destinations, validates nested file paths, checks HTTP status and bounds downloads to 16 MiB per file / 64 MiB per skill. This removes the unsafe second implementation; crate-level tests belong to the marketplace report. No marketplace download was executed.

#### U02 — Managed skill deletion used lexical path prefixes

Files: new `src-tauri/src/ui/skill_paths.rs`, `commands.rs`, `commands_marketplace.rs`, `mod.rs`.

A lexical `starts_with` accepted paths such as `skills/../outside`; a parent symlink could also lead outside the intended directory. Passing the managed root itself could remove all skills. The new helper resolves the root and target, requires strict descendant containment, rejects a symbolic-link target and requires a directory containing `SKILL.md`. User-created and marketplace classification/deletion now use the same rule and delete the validated canonical target. A missing/non-skill path returns a clear error instead of allowing a broad deletion. Existing persistent config removal stays scoped to the requested marketplace path.

Tests cover ordinary user/marketplace paths, root rejection, parent traversal, missing/non-skill directories, leaf symlinks and escaping parent symlinks. This is a pre-operation confinement check rather than a claim of race-free hostile-filesystem protection: another local process able to replace directories during validation/removal can still require OS-specific descriptor-relative operations.

#### U03 — Invalid approval edits silently reverted to the original request

Files: new `src-tauri/src/ui/input_validation.rs`, `commands_clients.rs`, `mod.rs`, `src/views/firewall-approval.tsx`.

`serde_json::from_str(...).ok()` previously discarded malformed edited arguments and proceeded to allow the unedited payload. This is especially wrong when a user attempted to remove sensitive content. Allow actions now parse through a fallible helper before any permission/tracker changes; invalid edits return an error. Deny/block/disable actions ignore edits and remain available even when an editor contains malformed JSON. The frontend likewise only builds edit payloads for allow actions, avoiding an earlier local JSON.parse failure on Deny. Submission errors now appear inline while retaining approval/editor controls, allowing correction or denial instead of replacing the entire window with an error message.

Tests verify malformed/empty edits are rejected, absent edits remain absent, and valid modified arguments are preserved. Actual native approval windows and end-to-end backend waiting behavior were not exercised.

#### U04 — Skill names/descriptions could corrupt YAML frontmatter

Files: `src-tauri/src/ui/input_validation.rs`, `commands.rs`.

The create-skill command inserted user-provided names and descriptions between raw double quotes. Embedded quotes, backslashes or line breaks could create invalid frontmatter or inject extra keys. The document helper now serializes each field as a JSON-quoted YAML-compatible scalar and preserves the body. Tests parse the generated YAML and verify exact round trips for quotes, backslashes and a description containing apparent frontmatter delimiters/extra keys; blank descriptions are omitted.

#### U05 — Reverse-proxy retargeting corrupted IPv6 and URL suffixes

File: `src-tauri/src/ui/commands_reverse_proxy.rs`.

The old `rsplit_once(':')` logic treated the last colon of a bracketed IPv6 address or password as a port separator, and a URL with only query/fragment suffixes was parsed as part of its authority. Retargeting now isolates credentials, respects IPv6 brackets and splits suffixes at `/`, `?` or `#`. It retains the established behavior for empty/schemeless values and ordinary provider paths. Original and new regression tests passed as standalone extracted pure Rust functions. This function does not replace full upstream URL validation; malformed URL acceptance remains governed by existing provider/reverse-proxy validation.

#### U06 — Generated speech audio blocked by CSP

File: `src-tauri/tauri.conf.json`.

The speech panel creates `blob:` audio URLs, but the native CSP had no media directive and inherited `default-src 'self'`, which does not authorize blob playback. Added `media-src 'self' blob:`. The script and network policies are unchanged. Configuration JSON was parsed successfully; actual native audio playback was not launched.

#### U07 — Development server port could disagree with the native window

File: root `vite.config.ts` (also described in frontend report).

The Tauri development URL is fixed to `http://127.0.0.1:1420`. Vite now uses `strictPort: true`, so it reports a collision instead of selecting another port that the native window will not load.

### Verification and exact results

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

### Observations without additional changes

- Local-model path/model ID/import validators already reject parent components, nulls, relative imports and unsupported extensions as appropriate. Detailed remote/download implementation is owned by the models/engines reviewer; none was run.
- Provider secret-bearing `api_key`/`custom_headers` values are stripped before on-disk provider config is produced by the inspected helper; key migration/deletion errors are logged without printing the key itself.
- Monitor commands obtain running server state and return errors when it is unavailable. Event summary fields used by the frontend match the inspected Rust/TypeScript contracts.
- Incremental refresh emits started/provider/completed events and the frontend fix now registers before starting it. Failure events do not provide per-provider error details; broad event-contract redesign was avoided.
- Engine-install commands parse known recipe IDs and delegate supervision/installation to the engine subsystem. Capability files scope approval windows by their corresponding labels. Their larger default permission set was not narrowed speculatively without native interaction tests.
- Tray formatting already handles non-finite/negative values in compact output and has boundary tests. Tray graph/menu code received structural review and selected lifecycle reads rather than a native rendering audit.

### Remaining evidence gaps

- Native Tauri capability enforcement, real OAuth redirect/cancellation, focus timing, popup positioning, tray layout and actual media playback need an explicitly permitted interactive run.
- Rust command integration with the whole native crate is distinct from the isolated helper tests reported above.
- Managed deletion validation is not an OS-level atomic transaction; hostile concurrent filesystem replacement is outside demonstrated protection.
- Some command modules are thousands of lines long; structural inventory plus selected deep review is not exhaustive proof of every authorization/state transition.
- Provider-renaming secret migration is best-effort in existing code; stronger transactional rollback would require a coordinated registry/keychain/config redesign.
- Several version/probe commands launch subprocesses; the inspected coding-agent version command lacks a dedicated timeout. This was documented rather than broadened into a process-supervision rewrite.

### Per-file coverage

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

### Independent review of parent-owned fixes

At the parent's request, this reviewer then inspected the changed functions and relevant surrounding code in `crates/lr-api-keys/src/keychain_trait.rs`, `crates/lr-oauth/src/browser/callback_server.rs`, `crates/lr-engines/src/download.rs`, `crates/lr-local-models/src/download.rs`, `crates/lr-local-models/src/library.rs`, and `crates/lr-config/src/storage.rs`. This was read-only review; no edits or additional crate-test execution were performed by this reviewer.

- Found an incomplete concurrency fix in the library: although removal held the entries lock through deletion, import and completed-download insertion validated/read model files before acquiring that lock. They could validate, pause, then publish an entry after removal deleted the file. Reported this to the parent, who accepted it and is moving the lock before validation/candidate construction in both operations. Final implementation and regression results belong to the parent's core report.
- No newly introduced defect was identified in file-keychain persistence/publication order, cache-operation lock order, state/issuer validation before OAuth error delivery, orphan-strategy deduplication, owner-only exclusive config temp creation, or the local-model download check-and-insert critical section.
- Suggested a direct slow-cache-read versus store/delete/invalidate regression in addition to the existing concurrent-miss test. The locking implementation looked correct on inspection; this is a coverage suggestion rather than a demonstrated defect.
- The archive guard checks archive member components, with a fresh managed staging root assumed by its callers; it does not independently reject a symlink for the destination root itself. It also intentionally rejects duplicate archive writes to existing symlinks. Filesystem replacement races remain outside the demonstrated guarantee.
- Config write/sync failures can still leave their temporary file, an existing cleanup limitation; the new mode restricts that file to the owner on Unix.

After all frontend changes, the final CPU-only regression run passed **17 tests (15.8s)** and both desktop/website TypeScript checks exited 0. These results and all interactive/native verification limitations are retained in `frontend-website.md`.

Coordinator integration note: the independently identified model-library pre-lock validation race was resolved by acquiring the library index lock before import and downloaded-candidate validation. This joins validation, publication and deletion under the same operation lock.

---

## Appendix 7: Tauri runtime, launcher, updater, and integration-test review — 2026-10-01

Source report: `plan/review-2026-10-01/tauri-runtime.md`.


### Scope and operating constraints

Second-wave ownership assigned by the root reviewer: `src-tauri/src/launcher`, `src-tauri/src/updater`, `cli.rs`, `main.rs`, `lib.rs`, `src-tauri/build.rs`, plus static inspection of integration tests/examples. This excludes `src-tauri/src/ui`, which another agent owns. No app launch, external config edits, CA trust changes, proxy launch, real credential access, inference, engine process, GPU, network download, or provider call was performed.

Every listed source was inventoried and scanned for startup/process/file-write/security boundaries. Substantive reads focused on launcher configuration preservation, filesystem backups, JSONC decoding, update policy, proxy lifecycle, command construction, marketplace skill installation, and bridge/startup wiring. Large main.rs and integration suites were sampled, not exhaustively executed or proved. Tests known to invoke model initialization/downloads were identified and excluded.

### Implemented changes

#### Safe configuration updates and backups

`launcher/backup.rs` now creates both config replacement files and backups with owner-private Unix permissions from creation, writes/syncs before atomic persistence, and cleans temporary files on failure. Unique random backup suffixes prevent same-second overwrites of earlier recovery points, including files with identical basenames. A read error stops the edit rather than treating an unreadable file as absent. Identical-content writes return without replacing the inode. Pruning occurs after the config replacement succeeds.

The backup tests now inject a temporary backup directory, eliminating their previous write to the real user backup directory. Added regressions for successive versions, private permissions and refusal to overwrite unreadable destinations (represented by a directory). Promoted `tempfile` from dev-dependencies into normal dependencies in `src-tauri/Cargo.toml` because atomic replacement now uses it in production.

Eight launcher integrations previously parsed existing settings with `unwrap_or(empty settings)`, silently discarding user configuration when parsing failed. JSON and YAML parsing now uses checked shared helpers that require object/mapping roots; malformed/non-object input fails before a write. Updated Aider, Claude Code, Codex, Cursor, Droid, Goose, OpenClaw and OpenCode paths, including cleanup/undo paths. Codex/OpenCode config readers also propagate I/O errors. Comment-only YAML is accepted as an empty mapping. Pure fixtures cover malformed syntax, wrong root types, preserved unrelated settings, and empty YAML.

#### JSONC correctness

The JSONC reader now rejects unterminated block comments instead of accepting a valid prefix or an apparently empty document and rewriting it. Removed comments become whitespace rather than concatenating neighboring tokens: `1/*comment*/2` is rejected instead of silently becoming `12`. Existing string escaping, URLs, comments, CRLF behavior, and trailing-comma handling remain. Strengthened the previous unterminated-comment test from “does not panic” to actual rejection.

#### Marketplace callback confinement

Replaced the manual skill download/write callback in main.rs with `MarketplaceService::download_skill`, implemented by the MCP/marketplace agent. The callback now shares label/path validation, existing-symlink refusal, checked HTTP status, per-file and aggregate download limits. It returns the validated install directory for the existing config/rescan steps. This closes the duplicate unsafe installation path; tests of the shared path logic belong to the marketplace section. No skill was downloaded during this work.

#### Update interval robustness

The update policy uses checked signed-to-unsigned conversion rather than narrowing `u64` intervals to `i64`. Very large intervals cannot wrap negative and immediately trigger updates, and future last-check timestamps do not cause a zero-interval check after clock rollback. Existing install-manager and manual/automatic behavior is preserved.

### Validation

An isolated temporary Cargo harness includes the exact repository `backup.rs`, `config_parse.rs`, `jsonc.rs`, and `dotenv.rs` modules without Tauri startup. Command:

```sh
env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-runtime-target rustup run stable cargo test --offline --manifest-path /private/tmp/localrouter-review-runtime-harness/Cargo.toml --lib
```

**37 tests passed, 0 failed**. All filesystem tests use temporary directories; all remaining cases use in-memory strings. Log: `/private/tmp/localrouter-review-runtime-tests.log`.

A second harness pass extracted the exact production `UpdateMode` and `InstallSource` declarations, `InstallSource` implementation, `UpdateCheckDecision` and `should_check_for_updates` function, plus the unmodified updater test module. It excluded background timers, app handles, filesystem detection and startup. The same Cargo command with the additional filter `updater_decision::tests` produced **11 passed, 0 failed**, including the oversized interval and one-second-future timestamp regressions. Log: `/private/tmp/localrouter-review-updater-tests.log`.

**Total: 48 passed, 0 failed.** The harness and its build artifacts live outside the repository. Source extraction validates the pure policy rather than whole-Tauri wiring; workspace compilation/Clippy validation is coordinated by the root reviewer. Changed sources were formatted, and `git diff --check` passed for owned files.

### Follow-ups and unchanged areas

- Proxy/ReverseProxy startup still has check-then-bind state transitions that merit per-manager/per-client lifecycle serialization under simultaneous UI calls.
- Several shell command snippets interpolate environment values without shell quoting. Values generated from ordinary loopback URLs/UUID secrets are currently narrow, but CA paths with spaces and user-configured URLs deserve a shared quoting layer; platform differences make a broad untested replacement inappropriate here.
- External tool configuration schemas/CLI commands were not checked against live upstream documentation or installed apps. This review fixes local preservation/error-handling bugs rather than claiming latest third-party compatibility.
- CA trust operations and system/provider relocation commands were reviewed statically only and never executed.
- The launcher backup directory still enforces a global last-10 policy, per existing behavior. Per-target recovery retention could be a product improvement.
- main.rs startup remains a large cross-component integration module; no full Tauri startup or OS integration test ran.
- CLI Clap argument restrictions, updater externally-managed-install gating, pure proxy configuration merge tests, and deliberate Tauri crate re-exports showed no established issue in the paths read.

### Integration-test safety observations

`routellm_fixes_verification.rs` has tests calling actual downloader functions, model `predict`, and model initialization when local files exist; `routellm_improvements_tests.rs` has a real model download retry path. Their names alone do not imply CPU-only safety. These were not run. Provider/MCP suites frequently use local fixture servers and temporary databases, but some system/launcher paths use environment-owned resources; only explicitly inspected filters may be run. Build/compile checks do not execute those tests.

### Exact runtime coverage inventory

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

### Integration-test inventory (static inspection only unless final validation explicitly says otherwise)

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

### Example inventory

- `examples/feature_adapters.md` — static inventory; not executed.
- `examples/streaming-client-browser.html` — static inventory; not executed.
- `examples/streaming-client-example.ts` — static inventory; not executed.

---

## Appendix 8: Change manifest

The pre-existing catalog edit is excluded. Detailed source hunks are available in Git; this table lists the full review artifact and implementation scope.

| Path | Status | Category |
|---|---|---|
| `.github/workflows/ci.yml` | modified | source/test/config/documentation |
| `.github/workflows/deploy-website.yml` | modified | source/test/config/documentation |
| `CODEBASE_REVIEW_2026-10-01.md` | new | report/inventory |
| `README.md` | modified | source/test/config/documentation |
| `crates/lr-api-keys/src/keychain.rs` | modified | source/test/config/documentation |
| `crates/lr-api-keys/src/keychain_trait.rs` | modified | source/test/config/documentation |
| `crates/lr-coding-agents/src/manager.rs` | modified | source/test/config/documentation |
| `crates/lr-compression/src/protection.rs` | modified | source/test/config/documentation |
| `crates/lr-config/src/storage.rs` | modified | source/test/config/documentation |
| `crates/lr-context/src/lib.rs` | modified | source/test/config/documentation |
| `crates/lr-context/src/truncate.rs` | modified | source/test/config/documentation |
| `crates/lr-engines/src/download.rs` | modified | source/test/config/documentation |
| `crates/lr-guardrails/src/engine.rs` | modified | source/test/config/documentation |
| `crates/lr-local-models/src/download.rs` | modified | source/test/config/documentation |
| `crates/lr-local-models/src/library.rs` | modified | source/test/config/documentation |
| `crates/lr-marketplace/src/lib.rs` | modified | source/test/config/documentation |
| `crates/lr-marketplace/src/skill_sources.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp-via-llm/src/manager.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp-via-llm/src/tests.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp/src/gateway/router.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp/src/transport/mod.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp/src/transport/sse.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp/src/transport/stdio.rs` | modified | source/test/config/documentation |
| `crates/lr-mcp/src/transport/websocket.rs` | modified | source/test/config/documentation |
| `crates/lr-memory/src/session_manager.rs` | modified | source/test/config/documentation |
| `crates/lr-memory/src/tests.rs` | modified | source/test/config/documentation |
| `crates/lr-memory/src/transcript.rs` | modified | source/test/config/documentation |
| `crates/lr-monitor/src/store.rs` | modified | source/test/config/documentation |
| `crates/lr-monitoring/src/storage.rs` | modified | source/test/config/documentation |
| `crates/lr-oauth/src/browser/callback_server.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/anthropic.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/cerebras.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/deepinfra.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/features/anthropic_thinking.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/features/logprobs.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/gemini.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/groq.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/mistral.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/oauth/mod.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/oauth/storage.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/ollama.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/openai.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/openai_compatible.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/openai_responses/stream.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/openrouter.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/perplexity.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/sse_lines.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/systemone/types.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/togetherai.rs` | modified | source/test/config/documentation |
| `crates/lr-providers/src/xai.rs` | modified | source/test/config/documentation |
| `crates/lr-proxy/src/active.rs` | modified | source/test/config/documentation |
| `crates/lr-proxy/src/cert.rs` | modified | source/test/config/documentation |
| `crates/lr-proxy/src/lib.rs` | modified | source/test/config/documentation |
| `crates/lr-proxy/src/passive.rs` | modified | source/test/config/documentation |
| `crates/lr-proxy/tests/reverse_e2e.rs` | modified | source/test/config/documentation |
| `crates/lr-responses-sessions/src/lib.rs` | modified | source/test/config/documentation |
| `crates/lr-router/src/endpoint_cache.rs` | modified | source/test/config/documentation |
| `crates/lr-router/src/rate_limit.rs` | modified | source/test/config/documentation |
| `crates/lr-secret-scanner/src/regex_engine.rs` | modified | source/test/config/documentation |
| `crates/lr-server/src/lib.rs` | modified | source/test/config/documentation |
| `crates/lr-server/src/middleware/auth_layer.rs` | modified | source/test/config/documentation |
| `crates/lr-server/src/routes/audio.rs` | modified | source/test/config/documentation |
| `crates/lr-server/src/routes/responses.rs` | modified | source/test/config/documentation |
| `crates/lr-skills/src/executor.rs` | modified | source/test/config/documentation |
| `crates/lr-skills/src/mcp_tools.rs` | modified | source/test/config/documentation |
| `crates/lr-types/src/trace.rs` | modified | source/test/config/documentation |
| `docs/MCP_STREAMING_CLIENT.md` | modified | source/test/config/documentation |
| `examples/streaming-client-browser.html` | modified | source/test/config/documentation |
| `examples/streaming-client-example.ts` | modified | source/test/config/documentation |
| `package.json` | modified | source/test/config/documentation |
| `packaging/linux-repo/build-flatpak-repo.sh` | modified | source/test/config/documentation |
| `packaging/linux-repo/build-linux-repo.sh` | modified | source/test/config/documentation |
| `plan/2026-10-01-CODEBASE_REVIEW.md` | new | report/inventory |
| `plan/review-2026-10-01/api-routing-providers.md` | new | report/inventory |
| `plan/review-2026-10-01/catalog-preservation.json` | new | report/inventory |
| `plan/review-2026-10-01/change-manifest.json` | new | report/inventory |
| `plan/review-2026-10-01/coverage-inventory.json` | new | report/inventory |
| `plan/review-2026-10-01/foundations.md` | new | report/inventory |
| `plan/review-2026-10-01/frontend-inventory.json` | new | report/inventory |
| `plan/review-2026-10-01/frontend-website.md` | new | report/inventory |
| `plan/review-2026-10-01/mcp-tools-context.md` | new | report/inventory |
| `plan/review-2026-10-01/repository-inventory.json` | new | report/inventory |
| `plan/review-2026-10-01/security-storage-models-build.md` | new | report/inventory |
| `plan/review-2026-10-01/tauri-runtime.md` | new | report/inventory |
| `plan/review-2026-10-01/tauri-ui.md` | new | report/inventory |
| `plan/review-2026-10-01/validation-results.json` | new | report/inventory |
| `src-tauri/Cargo.toml` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/backup.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/aider.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/claude_code.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/codex.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/config_parse.rs` | new | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/cursor.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/droid.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/goose.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/jsonc.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/mod.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/openclaw.rs` | modified | source/test/config/documentation |
| `src-tauri/src/launcher/integrations/opencode.rs` | modified | source/test/config/documentation |
| `src-tauri/src/main.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/commands.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/commands_clients.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/commands_marketplace.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/commands_reverse_proxy.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/input_validation.rs` | new | source/test/config/documentation |
| `src-tauri/src/ui/mod.rs` | modified | source/test/config/documentation |
| `src-tauri/src/ui/skill_paths.rs` | new | source/test/config/documentation |
| `src-tauri/src/updater/mod.rs` | modified | source/test/config/documentation |
| `src-tauri/tauri.conf.json` | modified | source/test/config/documentation |
| `src/components/shared/FirewallApprovalCard.tsx` | modified | source/test/config/documentation |
| `src/components/ui/KeyValueInput.tsx` | modified | source/test/config/documentation |
| `src/hooks/use-theme.ts` | modified | source/test/config/documentation |
| `src/hooks/useIncrementalModels.ts` | modified | source/test/config/documentation |
| `src/hooks/useModelDownload.ts` | modified | source/test/config/documentation |
| `src/hooks/useTauriListener.ts` | modified | source/test/config/documentation |
| `src/lib/mcp-client.ts` | modified | source/test/config/documentation |
| `src/lib/sse.ts` | new | source/test/config/documentation |
| `src/views/firewall-approval.tsx` | modified | source/test/config/documentation |
| `src/views/monitor/hooks/useMonitorEvents.ts` | modified | source/test/config/documentation |
| `src/views/monitor/monitor-events.ts` | new | source/test/config/documentation |
| `src/views/try-it-out/llm-tab/chat-panel.tsx` | modified | source/test/config/documentation |
| `src/views/try-it-out/llm-tab/images-panel.tsx` | modified | source/test/config/documentation |
| `src/views/try-it-out/llm-tab/index.tsx` | modified | source/test/config/documentation |
| `src/views/try-it-out/llm-tab/speech-panel.tsx` | modified | source/test/config/documentation |
| `src/views/try-it-out/mcp-tab/index.tsx` | modified | source/test/config/documentation |
| `tests/e2e/unit.config.ts` | new | source/test/config/documentation |
| `tests/e2e/unit/frontend-regressions.spec.ts` | new | source/test/config/documentation |
| `tests/e2e/unit/mcp-client.spec.ts` | new | source/test/config/documentation |
| `tests/e2e/unit/sse.spec.ts` | new | source/test/config/documentation |
| `tests/scripts/test_packaging.py` | new | source/test/config/documentation |
| `vite.config.ts` | modified | source/test/config/documentation |
| `website/shared-icons.ts` | new | source/test/config/documentation |
| `website/src/hooks/use-theme.ts` | modified | source/test/config/documentation |
| `website/src/stubs/tauri-api-event.ts` | modified | source/test/config/documentation |
| `website/src/stubs/tauri-plugin-shell.ts` | modified | source/test/config/documentation |
| `website/vite.config.ts` | modified | source/test/config/documentation |
