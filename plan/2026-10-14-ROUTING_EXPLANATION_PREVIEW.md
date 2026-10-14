# Routing explanations and dry-run preview

- [x] Inspect current policy preview, candidate filters, metadata and monitor UI.
- [x] Add a shared cached-only candidate filter report used by chat execution and preview: permissions, disabled models/providers, known capabilities, gateway constraints, cooldowns and quotas; distinguish unknown information from executed decisions.
- [x] Add deterministic policy preview that resolves explicit mode rules without classifiers and marks classifier-required routes unknown. Retain the existing classifier test as an explicit user action.
- [x] Include policy/config fingerprints, candidate priority, estimated pricing, selected local instance, upstream attribution, attempts and classifier overhead in routing metadata. Preserve it across all supported LLM surfaces and errors.
- [x] Extend the Tauri preview contract, TypeScript request/response types, demo handler and settings UI together; add a preview/test distinction.
- [x] Add no-network and permission/filter parity tests, mode/default/unknown-policy cases and error explanations.
- [x] Plan Review: compare requirements and implementation and finish missed behavior.
- [x] Test Coverage Review: exercise safety, unknown metadata, all-excluded candidates and client isolation.
- [x] Bug Hunt: inspect policy precedence, stale caches, dropped explanations and hidden preview IO.
- [x] Update rustup stable; pin compiler PATH/RUSTC; run workspace Clippy/fmt/tests and frontend/demo checks.
- [x] Commit and push validated changes and generated catalog updates while preserving unrelated work.

Approved roadmap item P1 follows gateway options/attribution. Existing preview_routing_policy currently runs the classifier and may spend money; retain this as a clearly labeled test and add a default no-network dry run. No prompt or classifier call is sent during a dry run, including model discovery, usage polling or pricing fetches. Unknown capability/pricing/quota/classifier information must stay unknown, and a predicted first candidate is conditional on future runtime checks.

Reuse routing_context and policy mode-rule semantics. Use model_has_capability and registry's cached metadata instead of list_provider_models_cached (which may fetch). Do not poll quotas in preview. Extract candidate exclusions from the live path into a shared helper so preview and execution use the same permission/capability/gateway checks. Dynamic runtime checks remain separately labeled. A full request preview accepts relevant messages, metadata and extensions; existing prompt-only policy testing stays compatible.

Acceptance: a provider mock that fails on any IO sees zero calls during preview; explicit mode selection matches live routing; classifier output is never invented; disabled and unauthorized candidates cannot appear eligible; new gateway restrictions participate; structured explanations survive no-route errors and streaming. Existing free-only and explicit-model behavior remains enforced. No telemetry or background probes are added.

Review completed: cached eligibility is shared by chat and native decision routing; exact client rules are resolved without IO while semantic verdicts remain unknown. Native Decisions now follow the same route/default/baseline chain. Classifier overhead is separate, and safe attempts plus policy/config fingerprints survive normal, SSE, no-route errors, Responses finalization, MCP errors and monitor events. Preview defaults to no classifier and the paid test is explicitly labeled. Types and demo handlers match the command.

Bug hunt fixed reserved mode metadata restoration by native adapters, dropped streaming Responses explanations, HTTP fixture authentication, structured errors lost in MCP/Responses wrappers, and post-request quota state changing the explanation of an already successful attempt. Candidate facts are snapshotted before inference. Failed attempt details expose outcome codes rather than raw upstream errors. Runtime quotas/readiness, missing capabilities/pricing and native transport discovery remain explicit unknowns. Existing free-tier-specialized errors preserve their contract.

Coverage includes canonical fingerprints, no-network/no-classifier preview, unchanged semaphore capacity, permission/filter parity, zero-dispatch no-route errors, native exact mode selection, sanitized native metadata, normal/streaming HTTP explanations on all three chat surfaces and live OpenAPI schemas. App/demo types, all 62 frontend tests and both production builds passed. Final stable CI parity is running after the last snapshot fix.

Final error-path review also preserved original HTTP statuses for permanent failures while carrying the routing trace, recorded previously missing failed streaming attempts, and kept classifier overhead on native failures. Regression fixtures verify 400/403 and existing payment-status preservation, six permanent normal/streaming HTTP failures with no fallback dispatch, and matching preview/live unavailable-classifier reasons. Existing integration expectations now assert the structured exclusions rather than the old generic error variant.

Final validation passed on rustup stable 1.99.0 with actual compiler PATH/RUSTC pinned: workspace Clippy with warnings denied, fmt check and all workspace tests (4,182 passed, 69 ignored, zero failures). All 23 live API/OpenAPI fixtures, 62 frontend unit regressions, app/demo type checks and both production builds passed. Plan Review, Test Coverage Review and Bug Hunt are complete.
