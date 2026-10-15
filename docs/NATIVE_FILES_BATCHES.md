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
