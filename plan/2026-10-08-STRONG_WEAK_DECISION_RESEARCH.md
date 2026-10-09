# Strong/weak routing with local decision models

- [x] Inspect the current classifier, routing semantics, repository examples and local model inventory.
- [x] Research current routing-specific models and System One alternatives using primary sources.
- [x] Save a reproducible benchmark using existing examples plus clearly separated stress cases; run local decision models and the existing classifier where feasible.
- [x] Record raw results, latency, classification behavior, limitations and a concrete replacement design. Research only: do not change production routing in this task.
- [x] Extend the research for the user's clarified direction: user-defined routing questions and model options, with useful default policy templates.
- [x] Benchmark explicit policy routing (planning/implementation and specialist/custom policies), including ambiguous and multi-turn requests; distinguish intent accuracy from actual model suitability.
- [x] Verify plan/edit mode availability in OpenAI-compatible requests and Codex; test routing with explicit mode metadata and document how LocalRouter would preserve it.
- [x] Plan review: check requested research, experiments and migration design against deliverables.
- [x] Test coverage review: verify benchmark metrics, failures and case provenance; add checks where necessary.
- [x] Bug hunt: inspect experiment setup for score inversion, contamination, cache/warmup effects and misleading quality claims.
- [x] Run required stable Rust CI checks, then commit and push the validated research artifacts and any automatic catalog update, preserving unrelated work.

The examples' easy/hard labels measure agreement with a routing policy, not actual strong-versus-weak answer quality. Keep that distinction explicit. Record models, versions, hardware, prompts and all failures. Avoid cloud inference and private user prompts. Benchmark engines should use isolated loopback ports and existing local weights where possible.


## Deliverables and validation

- Research and original strong/weak replay: `research/strong-weak-2026-10-08/README.md`.
- Revised recommendation, policy results, API-mode investigation, supplied-research verification and migration design: `research/strong-weak-2026-10-08/POLICY_ROUTING.md`.
- Reproducible harnesses, frozen inputs, all raw results/errors, summaries and a verified exportable chart are checked in. The Rust example runs the actual existing classifier. Production routing/configuration was not changed.
- Plan review: all requested investigations completed; distinguish policy compliance from answer-quality prediction. Explicit mode routing tested on synthetic normalized metadata; live client mode extraction remains a future integration test, not a measured claim.
- Coverage and bug review: eight Python checks pass, covering missing/duplicate results, error denominators, weak-only wins, threshold boundaries, option IDs and metadata precedence. All raw artifacts parse; policy corpus hashes match every run; local report links resolve. Exploratory strong/weak prompt tuning is explicitly identified.
- `rustup update stable`: passed; stable rustc 1.99.0 (`b940084d7`, 2026-09-28).
- `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `rustup run stable cargo fmt --all -- --check`: passed.
- `rustup run stable cargo test --workspace`: passed, 3,764 tests passed, 94 ignored, zero failures across 110 reported test suites.
- Final Rust builds used `CARGO_TARGET_DIR=/private/tmp/localrouter-routing-validation` after a shared target-cache removal interrupted the initial build. The isolated run completed successfully.
- Commit/push scope: research directory, benchmark example and this plan only; preserve other work. No automatic catalog change was present at final review. The commit and normal upstream push are the final task actions.
