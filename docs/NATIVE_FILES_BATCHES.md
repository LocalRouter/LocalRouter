# Native Files and Batches

LocalRouter exposes owned local Files and native asynchronous Batch APIs. Uploading a file stays local. Creating a batch validates every row through the current permission, approval and safety pipeline, seals the approved snapshot, reserves quota and output space, then submits native work. Preparation can invoke configured safety classifiers through their existing policy path.

## Supported native transports

| Account | Submission | Results | Cancellation | Explicit remote cleanup |
|---|---|---|---|---|
| Direct OpenAI API key | Multipart input file, then `/v1/batches` | Output/error JSONL files | Native cancel, possibly delayed and partial | Known input/output/error files; native batch record deletion unsupported |
| OpenRouter | Ordered inline `/api/v1/batches` request | Completed inline results; unavailable partial results remain unavailable | No verified cancellation operation | Terminal gateway batch deletion; upstream `deleted`, `unsupported` or `not_applicable` outcome retained |

Subscription/OAuth/ChatGPT backends and generic compatible providers do not advertise native batches. Native submission does not emulate a batch with synchronous paid requests. Automatic routing, free-tier-only strategies, automatic MCP loops, server tools, remote file references and unsupported payload fields fail explicitly.

The first payload subset supports text `/v1/chat/completions`, text `/v1/responses` and string-input `/v1/embeddings`. All rows need unique bounded `custom_id` values, `method=POST`, one endpoint and one explicit `provider-instance/model`. Generation rows require one output cap in `1..65536` and one choice. Chat supports client function declarations/history without executing tools. Responses currently excludes tools and stored continuations. OpenRouter chat uses `max_tokens`; unverified reasoning aliases are rejected.

For OpenRouter, use the base model slug on the wire. Permissions and eligibility use its separately advertised `:batch` catalog entry. `:free`, `:nitro` and other variants are not silently changed into paid base models. Saved gateway restrictions are inherited: provider allowlists are supported, while unsupported privacy, price and routing preferences fail before submission.

## SDK lifecycle

Use the normal local client API key and `/v1` base URL. Persistent files/jobs require an enabled real client; internal test identities and duplicate-hop batch bypass are rejected.

```python
from openai import OpenAI

client = OpenAI(base_url="http://127.0.0.1:3625/v1", api_key="YOUR_LOCAL_CLIENT_KEY")
file = client.files.create(file=open("input.jsonl", "rb"), purpose="batch")
job = client.batches.create(
    input_file_id=file.id,
    endpoint="/v1/chat/completions",
    completion_window="24h",
    extra_headers={"Idempotency-Key": "one-application-operation-id"},
)
# Local state only: this does not contact the provider.
job = client.batches.retrieve(job.id)
# Explicit upstream refresh and bounded result ingestion:
job = client.post(f"/batches/{job.id}/refresh", cast_to=dict)
```

Reuse the same idempotency key and identical create body after a lost response. Idempotency resolves the original job before reopening an expired/deleted input or consulting changed provider credentials. Different bodies conflict; absent keys represent distinct caller operations. SDK retries with a stable key cannot create another native submission. The app disables SDK mutation retries and prepares first, with a separate paid submission confirmation.

| Endpoint | Behavior |
|---|---|
| `POST /v1/files` | Owned local `purpose=batch` JSONL upload |
| `GET /v1/files` | Owned input/result listing; `purpose=batch` or `batch_output` |
| `GET /v1/files/{id}` and `/content` | Owned metadata/original bytes |
| `DELETE /v1/files/{id}` | Explicit local payload deletion; active references conflict |
| `POST /v1/batches` | Durable owned job, asynchronous validation/submission |
| `GET /v1/batches` and `/{id}` | Local records only, with separate asynchronous usage |
| `POST /v1/batches/{id}/resume` | Explicitly revalidate an undispatched draft, preserving approved edits |
| `POST /v1/batches/{id}/submit` | Submit a prepared snapshot or a proven uploaded input |
| `POST /v1/batches/{id}/refresh` | Read the known native ID and ingest available results |
| `POST /v1/batches/{id}/cancel` | Stop local validation first, cancel undispatched inference locally, or invoke verified native cancellation |
| `POST /v1/batches/{id}/reconcile` | Read one bounded private OpenAI metadata page and adopt only a nonce/snapshot/input-proven identity; JSON `{"after":null}` starts at the first page |
| `POST /v1/batches/{id}/cleanup` | Explicit local and/or remote cleanup; JSON `{"local_payloads":true,"remote":false}` |

`extensions.localrouter.batch` accepts `prepare_only`, `result_limit_bytes` and `local_retention_seconds`. Prepared snapshots stop before native upload when `prepare_only=true`. The ordinary SDK default continues to native submission. Unknown extension fields fail explicitly. OpenAI output expiry is separate from local retention; native metadata allows fourteen caller pairs because two private pairs bind reconciliation.

Standard SDK status is supplemented by `localrouter.phase`, progress, worker state, reserved usage, pricing provenance, owned output aliases, accounting, cleanup outcomes and safe diagnostics. Missing native timestamps stay null rather than using local poll times. GET/list/startup never resume validation, submit inference, poll providers or list arbitrary remote account objects. The app offers an explicit five-minute watch of a known submitted job; watching never retries creation.

## Recovery and policy

The SQLite journal commits intent before every native upload/submit/cancel. Upload and submit can each be attempted only once per job. A timeout, server error, malformed acknowledgement or lost response leaves a durable uncertain phase and quota hold. There is no automatic create retry on restart. A verified native HTTP rejection can terminate an unexecuted job; ambiguous conflicts/throttling/server failures remain uncertain.

OpenAI upload reconciliation requires the private nonce filename and an exact byte match to the sealed snapshot. Submission reconciliation requires the saved nonce, snapshot hash, input ID and endpoint. Only a matching record is adopted; account pages are never returned as an ownership list. OpenRouter exposes no verified recovery listing, so uncertain creation remains locked. Starting a new intentional operation requires a new job and may duplicate work if the original uncertain request succeeded.

Every row passes the sequential shared policy pipeline before the first native upload. Approved supported edits are saved before progress and applied to the exact wire snapshot; unsupported tool/history/metadata edits fail rather than disappearing. A later denial preserves earlier edits for an explicit resume. Policy and the original credential/origin fingerprint are rechecked before mutations. Changed policy after sealing requires cancellation and a new preparation. Time grants cannot silently expire before dispatch. Current authorization checks remain on native control operations. Stopping/shutdown cancels validation and removes abandoned pending approval sessions; it does not grant approval or resume work.

## Accounting and retention

Reservations share the existing client quota windows. Ordinary requests see durable native reservations/usage; native reservations include ordinary usage and other native jobs. Existing synchronous in-flight admission semantics remain unchanged. Asynchronous batch usage is reported separately from synchronous latency so jobs do not create fictional inference calls, zero prices or latency samples.

Provider-reported charges, native estimates, BYOK gateway fees and upstream inference estimates have separate provenance. A BYOK gateway fee alone is not the full inference cost. Missing token usage uses conservative reserved bounds for quota accounting while actual token fields remain null. Unknown charges stay unknown and fail closed for cost limits; missing output/error files never prove free execution. Receipts and aggregate reports are idempotent. A verified invoice can replace a native estimate without changing the original usage-observation timestamp; conflicting known invoices fail.

Verified public OpenAI price ceilings currently cover `gpt-6-astra`, `gpt-6.1-sol` and `gpt-6-luna`, using the maximum native Batch long-context/cache-write input and output tiers checked on 2026-10-15. They are conservative estimates, not invoices or a universal fifty-percent discount. Custom origins and other models/accounts have unknown ceilings; cost-limited jobs pause before native upload when no ceiling is verified. OpenRouter `:batch` eligibility does not itself prove a conservative price ceiling across BYOK accounts.

Storage is lazy, private and local: Unix directories `0700`, database `0600`, SQLite full synchronization, DELETE journals, secure deletion, no symlink traversal and no user-provided filesystem paths. Provider keys are never persisted in jobs, returned in job objects or included in raw upstream errors. Job identity uses a durable salted credential/origin fingerprint.

Inputs are at most 16 MiB, 1 MiB/row and 50,000 rows; embeddings also cap total input strings at 50,000. Output/error bytes share a reserved default/maximum 32 MiB per job and are never truncated. Limits include draft edits, result reservations and owned output aliases: 128 files/128 MiB per client and 512 files/512 MiB globally. At most two uploads and two validation/submission workers run concurrently; controls are limited to two globally and one per job. History is bounded to 128 jobs/client and 512 globally, with 8/16 active; it is not silently evicted while it may contain quota evidence.

Local retention defaults to thirty days, selectable from one hour to thirty days. Expired unreferenced payloads are removed on explicit store access. Active references protect recovery evidence. Terminal local payload cleanup may remove generated snapshots/results even when billing is incomplete, preserving usage/unknown quota evidence. Original uploaded files are managed separately through Files. After explicit payload cleanup, refresh does not download the payload again. Remote cleanup requires terminal reconciled accounting and only uses IDs bound to the original account; unsupported upstream deletion remains visible. Clearing local data never implies provider deletion.

## Primary protocol references

- [OpenAI Batch guide](https://developers.openai.com/api/docs/guides/batch)
- [OpenAI Batch creation](https://developers.openai.com/api/reference/resources/batches/methods/create)
- [OpenAI Files API](https://developers.openai.com/api/reference/resources/files/methods/create)
- [OpenAI native pricing](https://developers.openai.com/api/docs/pricing)
- [OpenRouter Batch quickstart](https://openrouter.ai/docs/batch-quickstart)
- [OpenRouter model variants](https://openrouter.ai/docs/guides/routing/model-variants/overview)
