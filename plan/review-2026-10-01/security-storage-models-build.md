# Configuration, credentials, local models, scanning, build and packaging review

## Scope and method

Coordinator-owned review of `lr-api-keys`, `lr-oauth`, `lr-config`, `lr-secret-scanner`, `lr-guardrails`, `lr-engines`, `lr-local-models`, `lr-compression`, `lr-embeddings`, `lr-routellm`, build workflows, packaging, scripts, examples and root documentation. Initial supporting-library inspection covered `lr-types`, `lr-utils`, `lr-json-repair`, `lr-monitor`, `lr-monitoring`, `lr-clients` and `lr-catalog`; the MCP reviewer performed the second wave for these foundations.

The review combined manifests and source inventories, risk-oriented searches, detailed reads of persistence/authentication/download/validation paths, existing test inspection, targeted fixes and focused regressions. Generated model data, binary assets, generated Tauri schemas and historical plans were inventoried rather than exhaustively hand-audited. This is not a claim that every line of this approximately 434,000-line repository was deeply reviewed.

No GPU device was initialized, no inference model was loaded, no real model or inference-engine download was performed, and the desktop application was not launched. Download tests use tiny fixtures served over loopback. The compression protection tests compile the standalone pure Rust module without linking Candle or GPU libraries.

## Implemented changes

### 1. Private, atomic file-keychain persistence

**Files:** `crates/lr-api-keys/src/keychain_trait.rs`.

Before: file-keychain writes used ordinary `fs::write` on a predictable temporary name, inherited ambient file permissions, and explicitly removed the destination on Windows before replacement. Secrets could be readable beyond the file owner; failure after removal could lose the previous file. Mutation was published in memory before persistence succeeded.

After: same-directory `NamedTempFile` provides exclusive creation and private Unix permissions. Contents are written, synced and atomically persisted. A complete candidate map is persisted while holding the storage mutex, and only then becomes the live map. A failed store/delete leaves the previous in-memory state intact. Temporary files are cleaned up on failure.

Regression coverage verifies failed replacement preserves existing secrets, failed insert does not create a visible secret, failed delete retains the secret, no temporary secret file remains, and both fresh and replacement files have mode `0600` on Unix.

### 2. Serialize underlying keychain operations with cache publication

**Files:** `crates/lr-api-keys/src/keychain_trait.rs`.

Before: a slow cache miss could fetch an old value, race a store/delete, and repopulate the cache after the newer operation completed. Concurrent misses could also repeat operating-system credential prompts.

After: a shared operation mutex orders cache misses, writes, deletes and explicit invalidation. Cache hits retain the fast read path. A miss rechecks the cache after acquiring the operation mutex. Eight simultaneous synthetic readers verify one underlying lookup and consistent results.

### 3. Make real credential-store tests opt-in

**Files:** `crates/lr-api-keys/src/keychain.rs`.

Three tests directly create/delete entries in the real operating-system credential store. They now have explicit ignore reasons and require an intentional `--ignored` invocation. File-backed and mock-keychain tests remain automatic. No real credential-store tests were executed during this review; an initial broad build was interrupted before test execution when these tests were identified.

### 4. OAuth error callbacks require a matching flow and escape HTML

**Files:** `crates/lr-oauth/src/browser/callback_server.rs`.

Before: the error path returned before validating `state`, interpolated provider-controlled error strings directly into HTML, and did not notify the pending flow. A browser could display injected markup while the application kept waiting for timeout.

After: both success and error callbacks pass state and issuer checks. A validated provider error completes the matching flow with an error, and HTML interpolation escapes all special characters. Invalid states leave the legitimate pending flow intact. Cancellation documentation now matches the implemented server shutdown.

Regression coverage uses a temporary local callback listener and synthetic query strings to verify invalid-state isolation, escaped script/image markup and immediate error delivery. All OAuth library tests pass when localhost binding is available.

### 5. Heal a shared missing strategy only once

**Files:** `crates/lr-config/src/storage.rs`.

Before: the known-strategy set was built once and never updated during healing. Two clients referencing the same absent strategy caused two duplicate strategy records to be created, after which validation rejected recovery.

After: newly encountered IDs are inserted into the set during traversal, so each absent strategy is created once. A full save/load fixture confirms both client references survive and the resulting configuration validates.

### 6. Create configuration temporary files privately and exclusively

**Files:** `crates/lr-config/src/storage.rs`.

Before: temporary YAML files were created with normal file permissions, and restrictive permissions were applied only after publication. Creation also allowed an existing temporary path to be truncated.

After: `OpenOptions::create_new(true)` creates each temporary file exclusively; Unix mode `0600` applies from creation. Existing final-file restrictive permissions remain. This improves the secret-bearing configuration write path without changing its schema.

### 7. Prevent chained-symlink writes during engine archive extraction

**Files:** `crates/lr-engines/src/download.rs`.

Before: lexical checks rejected direct `../` paths and direct escaping symlinks but could miss chains of individually permitted symlinks. For example, `a -> .`, `b -> a/..`, then `b/escaped` can resolve outside the extraction directory.

After: ZIP and tar.zst extraction reject members whose destination components traverse existing symlinks, including symlinks from earlier entries or an extra archive. Ordinary library symlink entries remain supported; writing through them is rejected. Existing target symlinks cannot redirect a file overwrite.

Regressions cover the chained attack in both ZIP and tar.zst and preservation of a pre-existing file outside the destination. These operate only on tiny generated archives and temporary files.

### 8. Expose guardrail model configuration failures

**Files:** `crates/lr-guardrails/src/engine.rs`.

Before: missing providers, incomplete model configuration and unknown model types were only logged; the advertised `load_errors` list stayed empty. The Tauri UI already consumed this list to emit model-load failures, so configured guards could silently disappear from that reporting path.

After: each rejected model populates `load_errors` with its model ID and actionable reason. A constructor-only test exercises all three cases without executing any moderation model or HTTP request. Runtime allow/ask/deny policy is otherwise unchanged.

### 9. Atomically deduplicate local-model download jobs

**Files:** `crates/lr-local-models/src/download.rs`.

Before: overlap checking and job insertion occurred under separate lock acquisitions. Concurrent starts could both pass the check and write the same partial and final files.

After: overlap/deduplication checks and insertion share one critical section. Existing jobs resume after releasing the lock. Repeated selected paths are deduplicated once before admission. A loopback fixture launches 16 concurrent starts and verifies one ID and one job; no real model is downloaded.

### 10. Preserve model files shared through equivalent paths

**Files:** `crates/lr-local-models/src/library.rs`.

Before: deletion used a stale snapshot of remaining entries after releasing the index lock, and sharing comparisons used raw paths. Concurrent additions or equivalent paths containing `.` could leave an active entry pointing at a deleted file.

After: the index lock spans the ownership check and deletion. Import and downloaded-entry insertion acquire the same lock before validating files, so a validated candidate cannot wait for a deletion and then publish a missing path. The latter gap was found and closed during independent review. Sharing comparisons use canonicalized paths. A fixture verifies a downloaded file remains when an imported entry references the same file through an equivalent path.

### 11. Detect secrets with overlapping keyword prefixes

**Files:** `crates/lr-secret-scanner/src/regex_engine.rs`.

Before: non-overlapping Aho-Corasick iteration could find the short `sk-` keyword and omit the longer `sk-proj-`, `sk-ant-` or `sk-ant-api03-` rules. Specific modern key formats could pass without their intended detector running.

After: overlapping keyword iteration considers every matching rule prefix. Regressions verify all three formats using synthetic high-entropy strings.

### 12. Mask Unicode secret previews without panicking

**Files:** `crates/lr-secret-scanner/src/regex_engine.rs`.

Before: preview generation sliced six prefix bytes and four suffix bytes directly. Generic rules can match non-ASCII passwords, making those byte positions invalid UTF-8 boundaries.

After: preview masking uses character counts and character boundaries. Regressions cover short Unicode values, mixed-width masked content and long emoji strings. ASCII masking semantics are retained.

### 13. End a self-contained fenced-code region correctly

**Files:** `crates/lr-compression/src/protection.rs`.

Before: a word containing both opening and closing fences toggled fenced state only once, unnecessarily protecting all later prose from compression.

After: the number of fence delimiters determines the state transition. A regression checks prose before and after a self-contained fenced word. All 26 pure protection tests pass without loading a model or linking the GPU runtime.

### 14. Add frontend, website and script validation to CI

**Files:** `.github/workflows/ci.yml`.

A separate CPU validation job installs pinned dependency sets, executes Node-only frontend regressions, executes packaging script regressions, builds the desktop frontend and typechecks/builds the website. The workflow now declares read-only repository permissions and cancels superseded runs for the same ref. Existing Rust checks remain.

### 15. Build and deploy the tested website revision reproducibly

**Files:** `.github/workflows/deploy-website.yml`.

Automatic deployment follows successful push CI runs, checks out the triggering commit SHA, uses the repository Node version and both lockfile caches, installs with `npm ci`, and uses `build:check`. Previously it checked out the current default branch, used Node 18 and unpinned installs, and omitted TypeScript validation. Manual deployment remains available. No deployment was executed.

### 16. Reject destructive packaging retention settings before mutation

**Files:** `packaging/linux-repo/build-linux-repo.sh`, `tests/scripts/test_packaging.py`.

Before: `--keep 0` produced an empty retention set and could delete every release, including the release just staged. Arbitrary version input also reached filenames and pruning logic.

After: the script validates a positive integer retention count and the expected bare semantic version before staging/pruning. Regressions verify `0`, negative, fractional and textual retention inputs fail while a temporary repository remains unchanged.

### 17. Finish unsigned Flatpak metadata generation

**Files:** `packaging/linux-repo/build-flatpak-repo.sh`, `tests/scripts/test_packaging.py`.

Before: an optional GPG-key test was the last command of a redirected command group. With `set -e`, the unsigned case returned failure and stopped before generating the complete install metadata.

After: explicit `if` blocks make the optional signing fields safe. A temporary repository with fake `ostree`/`flatpak` commands verifies both `.flatpakrepo` and `.flatpakref`, their expected URLs and `.nojekyll`. No repository was published or signed.

### 18. Refresh development setup and identify archival examples

**Files:** `README.md`, `docs/MCP_STREAMING_CLIENT.md`, `examples/streaming-client-example.ts`, `examples/streaming-client-browser.html`.

Development instructions now use stable Rust, the Node version declared in `.nvmrc`, `npm ci`, and the project's preferred `--no-watch` development command. The old `/gateway/stream` example and guide now identify themselves as archival and point to the maintained MCP client/API documentation; the example imports a client that no longer exists and should not be mistaken for current instructions.

The historical browser example now creates server tags with DOM elements and `textContent` instead of interpolating server names into `innerHTML`. Its JavaScript was syntax-checked without launching a browser.

## Other reviewed areas and limits

- **Local-model metadata and transport:** inspected redirect/token-origin gating, typed error mapping, path validation, GGUF classification, memory-fit estimation, library staging and tests. Model metadata/fit computations do not establish actual hardware inference performance.
- **Local engine lifecycle:** reviewed recipes, managed pointers, archive extraction, process supervision, cancellation and fake-engine tests. Actual installation recipes, vendor releases and GPU executables were not run.
- **Embeddings:** inspected model lifecycle, serialization locks, tokenization/pooling and the downloader. No numerical embedding-quality validation was attempted.
- **RouteLLM:** inspected initialization, prediction, idle unload, downloader verification and test annotations. GPU examples and ignored model benchmarks were excluded.
- **Compression:** inspected protection, model scoring/window boundaries, service batching and download status; tested the pure protection layer. Numeric compression quality remains unvalidated.
- **Guardrails:** reviewed model/executor orchestration, failure reporting and confidence filtering. Provider protocol support and model efficacy require provider-specific fixtures or explicit live tests.
- **Catalog:** inspected cache-only build behavior and manifests. The pre-existing `modelsdev_raw.json` modification was preserved, and catalog fetches were disabled for all review builds.
- **Packaging/release:** read CI/deployment/release structure, Docker defaults, package-manager templates, publishing/pruning scripts, and shell/Python syntax. No installer image, container, package manager publication or external release action was executed.
- **Documentation:** current README, project guidance and selected recent architecture plans were consulted. Historical docs/plans and generated assets are explicitly classified in the inventory.

## Follow-up observations requiring separate validation/design

1. Embedding/compression downloaders copy cached artifacts directly to their final paths and use presence-based downloaded checks. A staged multi-file installation/manifest would give stronger interruption and consistency guarantees; real model downloads were intentionally excluded here.
2. Model forward passes occur inside async service methods while holding serialization locks. Moving them to dedicated blocking workers needs ownership/cancellation design and performance validation with actual models.
3. Model performance, quantization fit, native provider behavior, macOS GPU execution and Windows/Linux installer behavior remain platform/live-test work.
4. The configuration loader's recovery and watcher pathways deserve additional stress tests around overlapping external edits and atomic file replacement; this review fixes the demonstrated shared-orphan recovery bug, not every possible watcher race.
5. The historical streaming examples remain archival; no compatibility endpoint or retired client was reintroduced.
6. This review did not refresh dependency versions or perform an internet advisory assessment. Build/lint/test results establish only the checked local behavior and do not amount to a vulnerability-free certification.

## Validation record

The final consolidated summary records the exact final commands/results. Initial CPU core tests passed 183 tests. OAuth passed 38 tests after rerunning with localhost binding permitted. The stable Rust 1.99.0 foundation rerun passed 372 tests with 4 ignored: API keys 17 (3 ignored), configuration 127, engines 52 (1 ignored), guardrails 58, local models 88 and secret scanner 30. This includes the final keychain cache-concurrency changes; the later library validation-lock refinement is covered by the final shared-library rerun in the consolidated report. Pure compression protection passed 26 tests. Packaging passed 2 test methods, including four destructive-retention subcases. Workflow YAML, 10 shell scripts, the three Python diagnostic sources and browser-example JavaScript passed syntax checks; Python ML diagnostics were parsed, not executed.
