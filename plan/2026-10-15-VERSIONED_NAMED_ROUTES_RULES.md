# Versioned named routes and deterministic rules

- [ ] Audit existing reusable strategies, custom auto-model aliases, route options, candidate filters, preview/session/cache fingerprints and settings editor; extend them rather than add a second routing engine.
- [ ] Add bounded, default-preserving deterministic route rules with trusted client identity, request-derived tools/images/context size and conservative known-cost conditions. Ordered first match selects an existing route; unknown facts never satisfy a constraint. Preserve the semantic/default route when no rule matches.
- [ ] Add optional weighted branches using deterministic client/session assignment. Keep experiments opt-in, no mirrored traffic, no extra inference, and preserve normal permissions/privacy/budgets on every selected candidate.
- [ ] Add per-strategy named-route drafts, immutable bounded published revisions and explicit activation/rollback. Reuse the existing custom auto-model alias and reusable strategy, keeping a stable client-facing alias across revisions. Snapshot routing configuration, not secrets or inferred permission grants.
- [ ] Reject stale edits using an expected configuration/revision fingerprint. Validate drafts independently without mutating the active route. Publish and rollback change the active snapshot atomically; existing candidate policy and live authorization remain authoritative.
- [ ] Show active revision, draft validation/preview, ordered conditions/branches and publication history in a list-based editor. Synchronize every new Rust Tauri command, TypeScript response/params, typed call and demo handler. Reuse existing read-only preview and model discovery aliases.
- [ ] Include chosen rule/branch and revision in routing explanations, cache/session identity and replay artifacts. Update HTTP/OpenAPI fixtures if wire types change; reject unsupported modality/continuation combinations explicitly.
- [ ] Add migration/bounds, deterministic assignment, unknown-cost, precedence, stale publish/rollback, shared-strategy isolation, privacy/permissions, cache invalidation and stream/fallback regressions.
- [ ] Plan Review: compare implementation with every promised behavior, finish omissions and document limits.
- [ ] Test Coverage Review: inspect changed paths and add meaningful error/boundary/isolation coverage.
- [ ] Bug Hunt: review privilege expansion, alias collisions, concurrent publication, floating-point/bucket boundaries, stale fingerprints and continuation behavior.
- [ ] Update/pin Rust stable and run exact workspace Clippy, fmt and tests, frontend types/unit regressions and app/website builds.
- [ ] Commit and normal push validated task changes and automatic catalog updates, preserving unrelated work. Continue search/server tools, documents/OCR, rerank, video and realtime from the approved roadmap.

The user authorized all roadmap features and moved local replay to P1. Replay and earlier slices are already delivered. Existing strategies are reusable, and AutoModelConfig.model_name already supplies stable virtual-model aliases. Versions should extend that mechanism; no new authorization surface or cloud control plane is needed. Draft publication must never reproduce the existing editor's convenience of granting model permissions implicitly.

Keep version history bounded (for example 32 published snapshots per strategy with explicit validation and configuration-size ceilings), retain the current active version, and never silently discard an unpublished draft. Rollback selects a previous snapshot through the same validation and current policy checks. Unsupported endpoint behavior must be explicit. Existing optional routing features remain disabled until the user configures them.
