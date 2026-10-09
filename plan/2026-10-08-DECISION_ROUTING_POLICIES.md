# Configurable decision-model routing and published research

- [x] Inspect all RouteLLM integration points, provider decision APIs, configuration migrations, UI and website research conventions.
- [x] Replace strong/weak configuration with versioned routing policies: user questions/options/model lists, explicit mode rules, provider/model selector, conservative default and legacy migration.
- [x] Replace runtime classification with a bounded native System One call to the user's selected configured provider (hosted Jev and local Laya/Kev/etc supported); preserve role-aware context and routing metadata across APIs; enforce model permissions and capability eligibility.
- [x] Implement exact client-mode routing, semantic choices, failure/low-confidence fallback, routing preview and meaningful monitoring without recursive auto routing.
- [x] Replace frontend Strong/Weak controls with templates and editable question/options, provider decision-model selection, explicit-mode policy and fallback settings. Synchronize Tauri types/demo mocks/tray.
- [x] Remove RouteLLM runtime, downloader, dependencies and obsolete commands/tests/UI; retain legacy deserialization/migration and historical metrics compatibility only. Preserve research baseline provenance after removal.
- [x] Publish a new website research article with methods, local results, chart, limitations, sources and links to reproducible data; update product routing documentation and website examples.
- [x] Plan review: verify every promised behavior and edge case; implement missing items.
- [x] Test coverage review: add meaningful migration, metadata, routing/fallback/permission, provider error and UI contract tests.
- [x] Bug hunt: inspect request boundaries, cancellation/timeout, role confusion, unknown mode, option validation and recursive routing.
- [x] Run exact stable CI checks, frontend and website type/build checks, relevant UI verification; commit and push task changes plus any generated catalog update, preserving unrelated work.

User authorization: fully replace existing RouteLLM and allow users to choose their decision model from their configured providers, including hosted Jev and local Laya. This supersedes the research-only scope and its initial local-only classifier suggestion. No new telemetry; remote classifier calls occur only through user-selected providers.

Mode policy: exact `plan` routes to configured option; all other/missing modes use configured default. Client metadata key `localrouter.mode` is an explicit LocalRouter convention. Do not infer a trusted mode from arbitrary user prompt text. Semantic templates are opt-in interpretations of task intent. Model mappings are user preferences, not guarantees of answer quality.

Legacy migration preserves both model pools and enablement intent but does not reuse the old threshold or silently claim calibration. Without a configured decision provider, use the legacy strong/default pool and show setup status. Remove old binaries only on explicit user cleanup; do not delete cached weights implicitly.

Implementation review (2026-10-08): confirmed native-only classifier selection through existing providers; exact mode precedes inference; chat and Responses preserve mode through classification and consume it before upstream execution. Removed the RouteLLM crate, startup service, downloader and commands. Kept old log fields for historical readers and archived benchmark fixtures/source for reproducibility. Fixed policy-only auto model discovery/access (no ordinary priority list required), tray action dispatch, destination permission fallback, and stale preview responses. Client shortcuts open the LLM tab.

Validation so far: targeted router/config/server tests passed; 8 research checks, 33 frontend unit tests and 3 browser regressions passed. App and website production builds passed. Browser regressions cover saving a native provider/custom question/model mapping, contradictory/missing exact mode, and research figures on desktop/mobile. Full final stable checks recorded below on completion.

Final validation: updated stable to Rust 1.99.0 and explicitly pinned both the toolchain PATH and RUSTC after detecting Homebrew resolution through Cargo's wrapper. The final `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `cargo test --workspace` passed with that compiler: **3,752 passed, 65 ignored, zero failed**. Focused config/router/server run: 437 passed. Frontend: 33 unit tests, 3 browser regressions, desktop build and website type/build passed. Research: 8 checks; archived corpus preparation reproduces the exact frozen 232 cases. Packaging: 2 checks. Sitemap, bundled chart paths and staged diff whitespace validated. Existing bundle-size and upstream dependency notices remain.

Final delivery: commit and normal push to master's configured origin/master upstream. Website deployment follows the existing successful-CI workflow; the article also gets its own static SPA entry and sitemap URL. No unrelated working-tree changes or generated catalog update were present.
