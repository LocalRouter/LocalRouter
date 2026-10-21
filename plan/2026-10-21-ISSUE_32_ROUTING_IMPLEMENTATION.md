# Issue 32 follow-up: model selection and reusable routing profiles

## Todo and implementation sequence

- [x] Read the latest issue comment and audit the current selectors, strategy storage, routing fallback, connection URLs and named-route snapshots.
- [x] Save this proposed plan before implementation using copy-plan.sh. Implementation was authorized on 2026-10-21 and is isolated in the feature worktree.
- [x] Phase 1: replace policy model dropdowns with a searchable compact catalog picker; expose per-model free status and prices.
- [x] Phase 1: explain and display the running gateway URL, configured auto alias and routing/fallback behavior in client setup.
- [x] Phase 2: add reusable routing profiles with explicit client bindings, preserving client permissions, limits and paid-fallback consent.
- [x] Phase 2: add profile management and publication impact UI using existing routing snapshots and versioning machinery.
- [x] Plan Review: compare implementation against each behavior and acceptance criterion below; implement omissions.
- [x] Test Coverage Review: cover migration, picker metadata, shared-profile isolation, fallback and connection state changes.
- [x] Bug Hunt: review stale saves, permission expansion, aliases, cache/session invalidation, deletion and paid fallback.
- [x] Validate with current rustup stable: update stable, verify PATH/RUSTC, workspace Clippy with -D warnings, fmt check and workspace tests; run frontend type checks, focused tests and app/demo builds for implementation.
- [x] Commit and normal push completed, validated implementation and automatic catalog updates, preserving unrelated work.

## Request and recommendation

Source: https://github.com/LocalRouter/LocalRouter/issues/32#issuecomment-6309963617 (2026-10-21).
The original free-model matching request is closed and shipped. The latest comment asks for searchable, price-aware policy model selection, reusable Chatbot/Coding routing configurations, free-first with optional paid fallback, and clear connection/model-selection semantics.

Recommend two deliverable slices: picker/setup usability first; reusable routing profiles second. Keep one routing engine and reuse existing configuration/snapshot/versioning types. Do not simply expose shared Strategy IDs as the entire solution: Strategy includes model permissions, rate limits and free-tier spending controls, so sharing it changes more than model routing. No GitHub response or new issue is posted by this planning task.

## Verified current architecture

- src/components/strategy/RoutingPolicyEditor.tsx: ModelPicker is a plain Select; local Model drops pricing/free metadata. Both route destinations and decision-model selection use it. Saving through persistRoutingPolicy currently explicitly allows referenced models.
- src/components/shared/model-pricing-badge.tsx: reusable per-model free/pricing display exists. Fix partial-price presentation when reusing it: unknown input or output prices must not become $0.
- src/views/clients/tabs/unified-models-tab.tsx: client UI loads a strategy and edits model selection, free-tier controls and routing inline.
- crates/lr-config/src/types.rs: Client references strategy_id; Strategy contains permissions, limits, auto config and free-tier behavior. Default public auto alias is auto; legacy localrouter/auto is normalized internally.
- crates/lr-config/src/named_routes.rs: RouteSnapshot already excludes permissions and snapshots model alias, priority/available models, routing policy, adaptive and session settings. Published revisions, drafts and activation exist. Prefer factoring/reusing this machinery for shared routing content rather than maintaining parallel evaluators.
- crates/lr-router/src/lib.rs: selected policy route is combined with default-route models and the ordinary priority list; without an enabled policy, auto uses the priority list. Explicit model requests take the explicit-model path. Free-tier checks and Off/Ask/Allow paid fallback already exist.
- src/components/client/HowToConnect.tsx prefers actual_port to configured port. crates/lr-server/src/lib.rs increments the port on bind failure. This can explain 3625 versus 3627; the reporter's actual cause is unverified.
- plan/2026-10-15-VERSIONED_NAMED_ROUTES_RULES.md documents the existing versioned routing implementation and its isolation requirements.

## Phase 1: immediate improvements

1. Extract a compact accessible model picker with search by provider/name/id, provider and free-eligibility filters, and stable grouping. Reuse existing catalog loading and canonical free-model evaluation rather than inventing frontend matching. Keep decision/chat capability filtering and unavailable selected entries. Use virtualization if realistic catalog sizes require it.
2. Show provider, model, effective per-model free classification/reason and input/output price per million tokens, including price source. Distinguish local, subscription, limited free tier and paid; unknown prices remain unknown. Free eligibility is not a promise of remaining quota. Show badges on selected destination rows too. Metadata updates should not discard edits or reorder saved priority lists. Keep hosted decision-model cost visible even for otherwise free destinations.
3. Add a compact connection summary: actual OpenAI base URL ending in /v1, configured auto alias (default auto), and a copyable request using that alias. Use the existing credential reveal/copy conventions. Refresh on server/config/alias changes; a stopped server must not appear connected. Clearly distinguish gateway connections from inspection proxy mode.
4. Unify listening-port display across dashboard/settings/client instructions. If configured and bound ports differ, display both with the bind-fallback explanation. Do not hardcode 3627 as another service.
5. Explain model selection: permitted models are eligible for explicit requests; the auto priority list is also the policy's final fallback. For auto: chosen route -> default route -> priority list, with live permissions, capabilities, budget/free-tier checks and backoff applied. Out-of-rotation/available models are not automatically added to the fallback list. Explain that selecting a concrete model bypasses policy classification.
6. Keep existing free-tier behavior; relabel clearly as free-only / ask before paid fallback / allow paid fallback. Do not silently enable spending. Show the effective behavior alongside the policy, and explain decision-model charges separately.

Acceptance: a user can search a long model catalog, choose free models with pricing context, copy a working auto request using the running URL, and understand which fallback models may run without reading external docs.

## Phase 2: reusable profiles

1. Introduce a named routing profile resource (Chatbot, Coding) and optional binding from a client's existing strategy. Reuse RouteSnapshot and named-route revision logic; resolve an effective routing configuration before the existing router runs. Avoid duplicated routing algorithms. Existing inline strategies remain valid and unchanged when no binding exists.
2. Share routing content: classification question/method, decision model, route model lists, default route, ordinary fallback and routing tuning. Keep client alias/auto permission, model permissions, spending consent, rate limits and credentials local. Because RouteSnapshot currently includes the alias, separate binding-local alias from shared content explicitly rather than copying it blindly.
3. Keep free-tier/paid-fallback enforcement local to each binding. Offer clear profile-attachment defaults for free-first use, but require explicit selection for paid fallback; attaching/updating a profile never broadens existing client controls. Evaluate profile candidates against each client's permissions. Surface blocked/missing models with a repair action; never silently grant access through persistRoutingPolicy.
4. Add a Routing page listing profiles and affected clients; clients choose Custom or a shared profile and see a concise effective summary plus Edit profile and Make independent copy. Support create from current setup and duplicate. Keep advanced revision controls collapsed.
5. Use explicit draft -> publish/activate behavior, reusing current optimistic concurrency. Display affected clients and permission/missing-model issues before applying shared changes. Linked clients follow the active profile revision; users wanting a frozen configuration make an independent copy in v1. Do not introduce additional per-client revision-pin complexity initially.
6. Prevent deletion of an attached profile until clients are reassigned or explicitly detached to a copy. Validate references and migrations atomically. Existing shared strategies retain existing semantics; do not silently split shared budget/accounting state.
7. Include the effective profile identity/revision in routing explanations and cache/session fingerprints. Ensure previews, live dispatch, discovery and replay resolve the same effective configuration, including per-client restrictions. Handle revision changes between preflight and dispatch consistently with current stale-decision checks.
8. Update all changed Tauri commands, TypeScript response/parameter types, typed calls and website demo mocks. If endpoint wire schemas change, update OpenAPI annotations/registration/schema and verify the generated spec.

Acceptance: two clients can use Coding and follow a published change while keeping separate permissions and spending limits; Chatbot remains unaffected. Detaching creates a functional independent copy. Missing/forbidden models fail visibly without expanding permissions. Existing client configurations retain their behavior.

## Validation targets

- Picker search, capabilities, empty/error/loading state, missing selected models, unknown/partial prices and free rules refresh.
- Running vs configured ports, server restart/stopped state, custom alias and gateway versus inspection proxy instructions.
- Explicit model vs auto; selected/default/priority ordering, duplicates and empty lists; free exhaustion with Off/Ask/Allow, streaming and non-streaming, hosted decision cost disclosure.
- Shared profile migration and resolution, concurrent edits, publish/rollback, blocked deletion, detachment, two clients with different permissions/limits, missing providers, discovery and preview consistency, cache/session revision invalidation.
- Manual app/demo walkthrough using a large model catalog and two attached clients. No remote assets, telemetry or background inference.

## Scope and open design decisions

This plan recommends true linked profiles, not copy-only templates. Copy is an explicit escape hatch. Final UI placement and exact schema names can follow repository conventions during implementation. Current backend strategy sharing is useful groundwork, but separating routing reuse from access/spending is the main engineering work. Keep the first release focused on the picker and connection explanation so that the user receives value before that refactor lands.

## Authorized implementation (2026-10-21)

- [x] Create isolated worktree /private/tmp/localrouter-routing-profiles on feat/routing-profiles.
- [x] Implement and validate both phases, then commit and push this feature branch without changing the user's main checkout.

Compatibility approach: keep each attached strategy's existing auto_config as a materialized effective snapshot. Binding, publication and rollback update linked routing snapshots atomically in ConfigManager.update; all existing runtime/discovery/preview/replay paths continue reading the same strategy shape. A new optional binding records profile identity/revision. Preserve the local alias, permission, model permissions, limits, cache settings and free/paid controls on every update. Existing configurations omit the additive empty profile fields. This also gives older readers a usable effective inline configuration without understanding shared profiles. Reuse bounded NamedRouteVersions and RouteSnapshot; profile snapshots use a canonical internal alias, never replacing client aliases. Validate binding consistency and reject inline routing edits while attached. Profile editorial state must not invalidate live routing caches.


## Implementation review and compatibility evidence

Both phases are implemented on feat/routing-profiles in /private/tmp/localrouter-routing-profiles. Existing inline configurations retain their serialized form when profiles are unused; no schema-version bump, automatic attachment, model-access grant, or spending-setting migration was introduced. Older readers can continue using each linked strategy's projected auto_config even if they do not understand the additive profile metadata. Existing shared strategies keep their existing shared permissions/accounting semantics; the binding UI explicitly lists every affected client.

The routing page reuses the existing Decision Routing navigation entry, now labeled Routing. Profiles reuse RouteSnapshot and NamedRouteVersions, including bounded drafts, publication, rollback and explicit inactive-history removal. Profile content is projected under the configuration write lock with validation and compare-and-swap tokens. Profile bindings preserve aliases/enablement, model permissions, rate limits, free-tier consent and response-cache settings. Draft/editorial changes do not change live routing identity. Profile identity/revision participates in routing scope, cache/session identity, replay/explanation metadata and preflight staleness checks.

Review fixes include preserving client aliases during profile draft preview, refreshing/invalidation when switching preview client/session/snapshot, retaining independent copies when deleting an unused profile after its clients retire, freezing reviewed publication tokens, handling missing models and unknown/partial prices, and moving previously available models out of the available pool when promoted into fallback priority. Client preview now passes the authenticated client ID rather than its public credential identifier. Selected model references and priority order survive catalog refreshes; model lists are searched/filtered and rendered in bounded batches.

Tests cover legacy JSON round trips with absent profile fields; two independently controlled strategies following publication/rollback while an unrelated strategy stays unchanged; deletion and stale/concurrent editing; authenticated HTTP model discovery, custom auto aliases, direct model requests, streaming, permission revocation, ordinary fallback and cache invalidation. Browser coverage exercises creation/publication/attachment/detachment, capability-separated search, free filtering, two linked clients with different permissions, isolation from another profile, and actual-port/custom-alias/stopped-state connection instructions. All browser traffic is restricted to the local demo test server. Visual artifacts are /tmp/localrouter-routing-profile.png, /tmp/localrouter-routing-client.png and /tmp/localrouter-policy-picker.png.

The frontend unit suite passes 86 tests, and all six profile/policy browser regressions pass. App and website production builds and TypeScript checks pass. Final exact Rust CI checks pass with stable Rust 1.99.1: workspace Clippy with warnings denied, fmt check, and all 111 Rust suites (4558 passed, 69 ignored, zero failures). Logs: /tmp/routing-profiles-final-{clippy,fmt,workspace}.log. Final browser, frontend-unit, type-check and production-build logs are in /tmp/routing-profiles-final-browser.log, /tmp/routing-profiles-frontend-tests2.log and /tmp/routing-profiles-final-*.log.
