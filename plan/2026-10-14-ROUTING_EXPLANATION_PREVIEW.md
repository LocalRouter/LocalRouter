# Routing explanations and dry-run preview

- [ ] Inspect current policy preview, candidate filters, metadata and monitor UI.
- [ ] Add a shared cached-only candidate filter report used by chat execution and preview: permissions, disabled models/providers, known capabilities, gateway constraints, cooldowns and quotas; distinguish unknown information from executed decisions.
- [ ] Add deterministic policy preview that resolves explicit mode rules without classifiers and marks classifier-required routes unknown. Retain the existing classifier test as an explicit user action.
- [ ] Include policy/config fingerprints, candidate priority, estimated pricing, selected local instance, upstream attribution, attempts and classifier overhead in routing metadata. Preserve it across all supported LLM surfaces and errors.
- [ ] Extend the Tauri preview contract, TypeScript request/response types, demo handler and settings UI together; add a preview/test distinction.
- [ ] Add no-network and permission/filter parity tests, mode/default/unknown-policy cases and error explanations.
- [ ] Plan Review: compare requirements and implementation and finish missed behavior.
- [ ] Test Coverage Review: exercise safety, unknown metadata, all-excluded candidates and client isolation.
- [ ] Bug Hunt: inspect policy precedence, stale caches, dropped explanations and hidden preview IO.
- [ ] Update rustup stable; pin compiler PATH/RUSTC; run workspace Clippy/fmt/tests and frontend/demo checks.
- [ ] Commit and push validated changes and generated catalog updates while preserving unrelated work.

Approved roadmap item P1 follows gateway options/attribution. Existing preview_routing_policy currently runs the classifier and may spend money; retain this as a clearly labeled test and add a default no-network dry run. No prompt or classifier call is sent during a dry run, including model discovery, usage polling or pricing fetches. Unknown capability/pricing/quota/classifier information must stay unknown, and a predicted first candidate is conditional on future runtime checks.

Reuse routing_context and policy mode-rule semantics. Use model_has_capability and registry's cached metadata instead of list_provider_models_cached (which may fetch). Do not poll quotas in preview. Extract candidate exclusions from the live path into a shared helper so preview and execution use the same permission/capability/gateway checks. Dynamic runtime checks remain separately labeled. A full request preview accepts relevant messages, metadata and extensions; existing prompt-only policy testing stays compatible.

Acceptance: a provider mock that fails on any IO sees zero calls during preview; explicit mode selection matches live routing; classifier output is never invented; disabled and unauthorized candidates cannot appear eligible; new gateway restrictions participate; structured explanations survive no-route errors and streaming. Existing free-only and explicit-model behavior remains enforced. No telemetry or background probes are added.
