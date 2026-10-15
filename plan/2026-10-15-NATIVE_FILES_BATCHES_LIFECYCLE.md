# Native Files/Batches lifecycle (P2)

- [x] Review the March Files/Batches draft, current provider contracts and shared policy/accounting/storage infrastructure.
- [x] Save this plan before code. Define the verified transport matrix, ownership boundaries, durable states and concrete resource limits.
- [x] Add private, bounded, atomic per-client local file storage and authenticated upload/list/metadata/content/delete endpoints. Use opaque local IDs, reject unsafe paths/symlinks, enforce quotas/expiry and protect files referenced by active jobs. Uploading locally must not contact a provider.
- [ ] Add durable batch job records and owned SDK-compatible create/list/retrieve/cancel controls, output/error file aliases, explicit refresh/reconciliation and cleanup. Bind each job to client, provider instance/credential identity, endpoint/model, immutable input/policy snapshots and supported retention.
- [ ] Validate every JSONL row, unique custom ID, method/path, input limits and one native provider/model/endpoint before submission. Apply current permissions, approvals, secret scanning, guardrails, privacy restrictions and quotas to native jobs too; recheck policy before external upload/submission. Never use the old draft's native safety exemptions.
- [ ] Add native OpenAI API-key Files/Batches transport and a separate OpenRouter inline adapter, using currently verified endpoints. Subscription credentials and unsupported providers/operations must fail explicitly. Do not route provider batch requests through ordinary synchronous inference or describe local queues as discounted batches.
- [ ] Persist intent before each external mutation and remote IDs before advancing. Model never-submitted, uploading/submitting, known-submitted and unknown-outcome states separately. Restart, polling and cancellation must never automatically resubmit uncertain inference. Support client idempotency keys and safe read-only reconciliation where identity can be proven.
- [ ] Add durable pending quota/budget reservations and idempotent usage receipts so asynchronous work cannot bypass current client limits, overcommit concurrent jobs or charge twice on polling/restart. Preserve unknown/partial spend. Use actual native batch pricing/reported amounts; distinguish gateway fees, BYOK inference estimates and undiscounted extras.
- [ ] Preserve per-row successes/errors, unordered result identity and partial cancellation/expiry results. Keep control/retrieval authenticated and owner-scoped; never expose another LocalRouter client's provider-account jobs. Reject unowned remote file/container references and unsupported provider-side actions.
- [ ] Implement bounded user-triggered watches, output/error downloads and retention cleanup. Report unsupported cancellation separately from deleting terminal records. Remote cleanup failures remain visible/retryable without losing billing or ownership metadata.
- [ ] Add a Files/Batches management surface with explicit upload/validation/submission, real provider states, cancellation where supported, result inspection/download and deletion/retention controls. Show input counts, supported limits and known/unknown cost information needed to decide whether to submit. Synchronize capability reporting, OpenAPI, Tauri contracts and demo mocks.
- [ ] Cover cross-client/credential/model isolation, malformed JSONL, quota reservations, denied safety/privacy policy, partial results, cancellation races, credential rotation, idempotency, filesystem failures, retention and restart at every external mutation boundary with loopback fixtures. No paid external inference in tests.
- [ ] Plan Review: compare the implementation with every required behavior and finish omissions.
- [ ] Test Coverage Review: add meaningful uncovered edge/error/recovery tests.
- [ ] Bug Hunt: review duplicate submission, durable state transitions, ownership, cleanup, partial billing, reservation release, cancellation and unbounded retention.
- [ ] Update stable and pin actual compiler PATH/RUSTC; run exact workspace Clippy/fmt/tests, frontend types/unit/build checks and live API/OpenAPI fixtures.
- [ ] Commit and push completed validated changes and automatic catalog updates, preserving unrelated work.

This supersedes the unsafe native-policy exceptions and unverified provider assumptions in `2026-03-15-API-FILES-BATCHES.md`. Native batch payloads must pass LocalRouter policy before leaving the process. An asynchronous provider job is a distinct execution contract: automatic MCP loops, mutable route selection and privileged internal test tokens cannot silently bypass submission policy. Unsupported combinations are explicit capability errors.

## Initial contract matrix

OpenAI public API-key accounts use uploaded JSONL files followed by native batch creation; begin with validated text Chat, Responses and embeddings endpoints rather than claiming every newly advertised modality. The current API offers a 24-hour completion window, output expiry controls and genuine cancellation with partial results. File/model/endpoint support and pricing must remain verified; do not enable subscription backends by analogy.

OpenRouter currently uses **`/api/v1/batches`**, not the older beta URLs still visible on some model pages. It accepts inline requests, keeps model/endpoint/routing fields before `requests` in serialized JSON, returns accepted validation state, and later returns inline results. Its workspace-wide upstream listing is never a LocalRouter ownership list. Only terminal jobs can be deleted; deletion is not cancellation. Enable cancellation only if a current contract verifies it. Provider restrictions must map to documented batch controls or fail before IO.

A native discount is not a universal fixed reduction for every gateway, cached token or tool. Report provider/gateway amounts and estimates separately, and preserve unavailable prices. Unknown submission outcomes must retain evidence and reservations instead of being treated as failed/free/safe to retry.

## Persistence and recovery requirements

File and job IDs refer only to bounded local owned objects. Remote IDs and credential fingerprints are private bindings, never arbitrary client-selected proxy paths. Keep originals immutable, preserve output-affecting JSON ordering, and save the exact validated transformed snapshot/hash used for submission. Do not persist provider keys in job/file artifacts.

Journal submission intent before sending. A response timeout after a create call can mean the provider accepted it. A restart may poll a known remote ID or reconcile by a proven owned nonce; it must not replay the create operation just because an ID was not durably recorded. A second caller create is a distinct job unless its explicit idempotency key resolves to the same request fingerprint.

Quota reservations and accounting receipts need a durable source of truth shared with normal quota checks. Output ingestion, repeated polls and restart must not record usage twice. Releasing unknown reservations or deleting the last billing record must be deliberate and consistent with actual provider state.

Keep remote and local cleanup outcomes distinct. Active jobs may retain required inputs; expiry/deletion must not pretend to cancel running provider inference. Credential/base-URL changes cannot silently move a job to another account or host. Any unsupported operation remains visible in capabilities and the job UI.

## Primary sources reviewed

- https://developers.openai.com/api/docs/guides/batch
- https://developers.openai.com/api/reference/resources/batches/methods/create
- https://developers.openai.com/api/reference/resources/batches/methods/cancel
- https://developers.openai.com/api/reference/resources/files/methods/create
- https://openrouter.ai/docs/batch-quickstart
- https://openrouter.ai/blog/announcements/batch-api/

Remaining adapter details must be read from current primary references before enabling them. Local validation and loopback tests do not require live provider keys or paid calls.

## Concrete local storage contract

Use a new `lr-batches` crate with one private SQLite database for owned file blobs and, subsequently, job journals/reservations. It has no router/server dependency. Transactions make ownership, quotas and active input references consistent. Construction performs no IO; the first explicit file/job action creates the private directory/database. SQLite DELETE journals stay in the private directory; secure deletion clears freed payload pages. No filesystem path comes from an uploaded filename or ID.

Initial input limit is 16 MiB/file, 1 MiB/JSONL row and 50,000 rows. Local file quotas are 128 MiB/128 files per client and 512 MiB/512 files globally, with at most two concurrent multipart uploads. Output limits/reservations are enforced independently when batch ingestion is added; never silently truncate. Default retention is 30 days, configurable per upload from one hour to 30 days. Local Files initially accepts only `purpose=batch`; other purposes remain explicit unsupported capabilities. SDK-compatible file metadata/content/list/delete uses local opaque IDs and current real client authentication. Internal test identities cannot own persistent files. Active job references prevent removal. Expired unreferenced payloads are removed transactionally on explicit store access; no network cleanup is implied.

The first reviewable slice is local Files storage and its authenticated API, with store and loopback ownership/limit tests. Native batch submission remains unchecked until its entire policy/recovery/accounting lifecycle is implemented.

## Local Files slice review

The Files foundation is independently reviewable; the full native lifecycle remains pending above. Required local final steps:

- [x] Plan Review: verify upload/list/retrieve/content/delete, ownership, retention, quotas and UI behavior against the concrete local contract.
- [x] Test Coverage Review: store tests plus authenticated loopback fixtures for cross-client access, rotation, internal-token denial, malformed multipart/JSONL and live OpenAPI.
- [x] Bug Hunt: inspect transaction boundaries, cursor ownership, upload cancellation, private filesystem creation and UI client changes.
- [x] Run the exact stable workspace checks and frontend types/unit/production builds.
- [x] Commit and push the validated local Files slice, preserving unrelated changes; continue native batch submission next.

Local slice review completed: SQLite immediate transactions gate quotas/deletion; per-client cursors cannot reveal another client; real credential scopes exclude internal identities; slow-upload completion rechecks enabled clients; upload permits survive caller cancellation until blocking writes finish; names never select paths. Both metadata and content responses carry no-store. File UI works without a selected model and clears state on client changes. Six storage tests and two real HTTP fixtures pass, including local secret rotation and live OpenAPI. No Tauri command shape changed; the website OpenAPI demo is synchronized. Native submission, pricing and remote control remain pending rather than being inferred from local file support.

Local Files validation passed on rustup stable 1.99.0 with the actual compiler pinned: workspace Clippy with warnings denied, fmt and 111 test suites (4,338 passed, 69 ignored, zero failures), app TypeScript, all 72 frontend unit tests and app/website production builds. The two Files HTTP/OpenAPI fixtures also pass as part of the workspace suite. The following commit/push records this completed foundation; all native job tasks remain pending.

## Next slice: durable job journal and reservations

- [x] Add owner-scoped durable job records bound to immutable owned input and provider credential identity; explicit request idempotency is owner-scoped and rejects a changed fingerprint.
- [x] Add compare-and-swap revisions and mutation intents for upload, submit and cancel. Remote cleanup intents remain in the native adapter slice. A process crash leaves an uncertain operation with no automatic retry. Save known remote IDs in the same transaction that completes an intent.
- [x] Add bounded active/history job counts and transactional input references. Preserve minimal terminal ownership/accounting records; never expose remote account listing as local ownership.
- [x] Add durable pending reservations and per-row receipts with unique job/custom-ID identity. Unknown outcomes retain reservations; repeated result ingestion cannot double charge. The normal client quota gate now reads this same ledger; no synchronous usage is copied into the native ledger.
- [x] Review state transitions and coverage; test restart at mutation boundaries, concurrent idempotent creates, cross-client operations, quota overcommit and duplicate receipts.
- [x] Run exact stable CI and frontend checks; commit and push the completed journal foundation, then integrate native adapters and API submission.

Journal operations are primitives, not an enabled discounted endpoint. Public batches will use standard SDK statuses plus an explicit LocalRouter phase for uncertain transport outcomes. The journal never automatically sends a create. Bound jobs to 8 active per client/16 globally, 128 history records per client/512 globally; reserve storage before snapshots/results are added. Provider fingerprints must be salted durably and include configured origin and credential identity, never a raw key. Request idempotency compares the create request fingerprint rather than moving an existing job to a new provider/policy after rotation. One SQLite transaction owns job phase, input references, reservations and receipts.

Journal review: added an explicit draft-to-prepared boundary with an immutable SHA-256 snapshot, owned manifest/model/endpoint checks and duplicate-key rejection before a reservation or mutation can begin. The server must still perform all row policy checks before preparation and recheck the policy hash before external IO. Initial Unix directory/database entries are synced before durable intent is acknowledged. Bounded explicit cancellation retries target a known ID and never submit inference; terminal polls resolve outstanding cancel intents without stale status regression. Bulk receipt ingestion rolls back entirely on a conflicting/unowned result. Unknown cost remains represented explicitly rather than being released as free usage. Native cleanup, result persistence/retention and adapter transport remain unchecked in the full lifecycle.

Journal final review: Plan Review, Test Coverage Review and Bug Hunt completed for the foundation. Focused tests pass (20 store/journal tests, 13 router quota tests); workspace CI is running against frozen source. The ordinary synchronous admission model is preserved: pending native reservations/receipts are included, while synchronous requests do not yet reserve their unknown final usage. Do not claim stronger universal concurrent admission semantics.

## Native adapter/API integration decisions for the next slice

- Add a provider-owned ephemeral native batch transport for direct OpenAI API-key accounts and OpenRouter. Subscription/OAuth and generic compatible providers remain unsupported unless verified. Use redirects-disabled HTTP clients, bounded bodies/IDs, configured credentials only, current-origin/credential fingerprint verification and real wire schemas. Never persist keys or retry a create after an uncertain result.
- OpenRouter's current documentation explicitly returns inline results only for completed jobs; failed/expired/cancelled jobs return `results=null`. Do not promise unavailable partial outputs or assume they were free. Its completed job can report aggregate usage/cost without distributing usage per row: extend the journal with verified aggregate accounting and cost provenance instead of inventing per-row amounts. BYOK gateway fees and inference estimates stay separate. OpenAI downloads known owned output/error files and preserves partial terminal outputs.
- Add internal bounded result-file creation separately from public `purpose=batch` upload (input remains 16 MiB; output default 32 MiB with reserved capacity). Local SDK output/error IDs must be durable aliases; no arbitrary provider file passthrough. Retention/cleanup must preserve minimal ownership and usage evidence, including unknown outcomes, and cannot delete the last record needed for current quota windows.
- Native SDK create should persist an owned draft and return promptly while an explicitly triggered supervised local worker validates all rows sequentially/bounded, seals the transformed snapshot, reserves quotas, uploads and submits. Track local cancellation/shutdown between every stage; never auto-resubmit pending mutations on restart. Use standard SDK statuses plus explicit LocalRouter phases/errors. Provide an explicit resume of never-submitted validation and read-only refresh/reconciliation, not implicit paid work from GET/list/startup.
- Resolve one explicit provider/model/endpoint from immutable JSONL before draft creation. Reject automatic model movement, streaming, unsupported fields/modalities, unowned file/container references, server-side actions and automatic MCP loops. Every accepted row must use the sequential shared policy/safety pipeline before any external upload; apply supported edits to the exact provider snapshot and fail rather than ignore unsupported edits. Native batches must reject duplicate-hop/internal-test bypass. Recheck current client/model/credentials/policy after validation and immediately before external mutations.
- OpenAI metadata can carry a reserved private job nonce and snapshot hash (reduce the user metadata allowance accordingly) for bounded read-only reconciliation. An upstream workspace-wide listing is never returned to a LocalRouter client. OpenRouter has no documented nonce/idempotency guarantee for an unknown create: preserve uncertainty and reservations rather than matching by coincidental model/custom IDs or resubmitting.
- Cancellation targets a known job and differs from deletion. OpenRouter cancellation remains unsupported absent a verified contract; terminal DELETE must expose upstream cleanup outcomes. OpenAI job metadata deletion is unsupported even when its owned remote files can be deleted. Credential rotation/removal must not silently move controls to another account.
- Add SDK API/OpenAPI, real local job UI, capability/type/mock synchronization, explicit bounded watches, source/retention/cost labels and loopback fixtures for all provider, policy, restart, partial/unknown billing, cancellation and cleanup paths. Mandatory final reviews, exact CI and normal commit/push remain required before marking the full Files/Batches task complete.

Journal foundation validation passed with rustup stable 1.99.0 and actual compiler PATH/RUSTC pinned: workspace Clippy (-D warnings), fmt and all 111 test suites (4,355 passed, 69 ignored, zero failures), app TypeScript, all 72 frontend unit tests and app/website production builds. The following commit/push records this completed primitive layer and shared quota gate. Native transport, SDK job API, aggregate pricing provenance, result storage/retention, remote cleanup and job UI remain pending; the master Files/Batches phase stays open.
