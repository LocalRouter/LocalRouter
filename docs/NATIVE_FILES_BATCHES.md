# Files and native batches

## Local Files

`POST /v1/files` accepts multipart `file` and `purpose=batch`. Files remain on this machine, owned by the authenticated LocalRouter client. No provider upload or inference occurs. A local file marked `processed` has passed structural UTF-8 JSONL validation only. Submission must separately validate every row against current routing, safety, privacy and spending policy.

Use a normal client credential. Internal testing credentials cannot create or access persistent files. Ownership follows the client across local secret rotation; another client receives 404 for an unowned file ID, content, deletion or pagination cursor. An uploaded filename is display text, never a filesystem path.

Supported limits:

| Resource | Limit |
| --- | --- |
| Input file | 16 MiB |
| JSONL row | 1 MiB, JSON object |
| Rows | 50,000 |
| Concurrent uploads | 2 |
| Files per client / globally | 128 / 512 |
| Payload bytes per client / globally | 128 MiB / 512 MiB |
| Retention | Default 30 days; one hour to 30 days |
| List page | Default 20; maximum 100 |

Blank rows, invalid UTF-8/JSON, unsupported purposes, duplicate or unknown multipart fields, and oversized controls are rejected. Final line endings are optional. Multipart expiry fields must supply both `expires_after[anchor]=created_at` and integer `expires_after[seconds]`.

`GET /v1/files` lists only local owned files. Optional parameters: `purpose=batch`, `after=<local-file-id>`, `limit=1..100`, `order=asc|desc` (default `desc`). Results use the SDK list envelope and stable timestamp/ID ordering. Upstream account files are never implicitly listed.

`GET /v1/files/{id}` returns metadata; `GET /v1/files/{id}/content` downloads the exact original bytes. `DELETE /v1/files/{id}` removes local content and returns the SDK deletion envelope. Active job references prevent deletion with 409. Expired unreferenced payloads are removed on explicit store access. Local deletion/expiry does not imply cancellation or deletion of an upstream job.

The app's Try It Out → Files panel offers explicit upload, refresh, pagination, download and confirmed deletion in client mode. It does not need a model selection. Files are stored in `<configuration-name>.batch-store/batches.sqlite3` beside the configuration. Construction does no IO. On Unix, the directory and database use 0700/0600; symlinked roots, database files and sidecars are rejected. SQLite transactions enforce quotas and ownership; full synchronous commits and DELETE journaling protect atomicity, and secure deletion clears freed database payload pages. The database is local plaintext protected by OS permissions, not an encrypted vault.

## Native batch delivery status

Native batch submission is being implemented separately. Local file upload is available first; it does not provide batch execution, discounts, remote cancellation or provider file passthrough. The remaining policy, durable reservations, mutation journal, native adapters and result lifecycle are tracked in `plan/2026-10-15-NATIVE_FILES_BATCHES_LIFECYCLE.md`.

Current primary provider references: [OpenAI Files](https://developers.openai.com/api/reference/resources/files/methods/create), [OpenAI Batch](https://developers.openai.com/api/docs/guides/batch), [OpenRouter Batch](https://openrouter.ai/docs/batch-quickstart). LocalRouter's input/storage limits are intentionally smaller than some upstream limits. OpenRouter's current native API uses `/api/v1/batches` with inline requests, distinct from OpenAI's uploaded-file workflow.

## Durable journal foundation

The internal batch store now records draft jobs, immutable prepared snapshots, owned input references, provider/credential/policy fingerprints and a durable installation salt. Preparation verifies the exact custom-ID manifest, endpoint and model and seals a SHA-256 snapshot hash. Prepared JSONL must use compact reserialized JSON, preventing duplicate-key interpretations from changing policy after validation. Preparation is an explicit server step after safety checks; it does not replace them.

An owner-scoped idempotency key returns the original job for the same create fingerprint, including after input cleanup or key rotation. It never rebinds an existing job to a changed account. Compare-and-swap revisions and unique upload/submit intents prevent another create after a lost response. Pending intents can be read after restart for evidence-based reconciliation. Explicit cancellation retries target only the known remote job, are capped at eight, and cannot create inference. Stale polls and cancellation acknowledgements cannot regress terminal status.

Pending reservations remain charged across restart and past ordinary sliding-window boundaries until terminal accounting is proven. Per-row receipts must belong to the immutable manifest and carry a stable fingerprint. Repeated receipts are idempotent; conflicting results fail. Bulk ingestion is transactional. Partial results retain the full conservative reservation, using larger actual amounts when necessary, rather than adding partial spend to the reservation twice. Every row needs an explicit result, including verified unexecuted rows, before settlement can release the reservation. Unknown cost remains unknown after settlement.

Ordinary authenticated client quota checks read the same ledger, fail closed if it is unavailable and block cost-limited requests when batch spend is unknown. Native reservation compares every configured window with ordinary usage plus the durable batch ledger inside a write transaction, preventing concurrent batch jobs from overcommitting. This preserves the existing synchronous admission model; synchronous requests themselves do not yet reserve their unknown final usage. Ordinary traffic without a batch database does not create payload storage.

The journal currently bounds jobs to eight active per client and 16 globally, with 128/512 stored history records. SDK batch routes, remote adapters, user-facing job management, retention of historical accounting and cleanup are still pending. No journal primitive sends a network request.
