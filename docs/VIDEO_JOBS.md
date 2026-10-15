# Native video jobs

`POST /v1/videos` (also `/videos`) creates a native asynchronous video job.
Enable **Allow video generation** in the client's Settings. Supply an explicit
configured `provider/model`, text `prompt`, `seconds` and an `Idempotency-Key`
header. JSON and text-only OpenAI SDK multipart forms are supported.

```sh
curl http://localhost:3625/v1/videos \
  -H 'Authorization: Bearer YOUR_LOCAL_CLIENT_KEY' \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: one-explicit-video-intent' \
  -d '{"model":"openai/sora-2","prompt":"A quiet river","seconds":"4","size":"1280x720"}'
```

Direct OpenAI public API keys support Sora 2 / Sora 2 Pro at 4, 8 or 12 seconds.
ChatGPT OAuth credentials do not grant public video API access. LLM Gateway
forwards its native video JSON endpoint with no fallback and an explicit model;
model-specific durations may be rejected upstream. LocalRouter's initial limits
are 4–30 seconds, 720p landscape/portrait and a 16 KiB prompt. Reference assets,
remote URLs, callbacks, edits, remix and extensions are unsupported.

The response ID is a local owner-scoped ID. `GET /v1/videos` lists only this
client's recorded jobs, without enumerating the upstream account. Explicit
`GET /v1/videos/{id}` refreshes native status (at most once per second).
`GET /v1/videos/{id}/content` downloads completed MP4 bytes, with single byte
ranges and a 128 MiB transfer limit. No provider URLs or credentials are returned.
Status and content access require the current video grant, permitted model and
original provider origin/credential binding. Another client cannot adopt a job
by guessing its local or native ID.

An intent is durably recorded before the single upstream create. Concurrent or
later retries with the same key and normalized request return the recorded
intent; changing the payload with the same key returns 409. If creation is
interrupted or ambiguous, the job remains `unknown`. It is not automatically
resubmitted, including after restart. Keep and reuse the original key; inspect
the local job list rather than creating another generation automatically.
Native HTTP errors after dispatch carry `x-should-retry: false` and the local
job ID. No provider idempotency guarantee is assumed.

Cancellation is unsupported for these mappings: `POST /v1/videos/{id}/cancel`
returns 501. Stopping local waiting does not stop remote rendering or billing.
Direct OpenAI `DELETE /v1/videos/{id}` deletes only a completed/failed asset;
it does not cancel an active generation. Ambiguous deletion is recorded and
never replayed automatically. LLM Gateway remote deletion is unsupported.
`POST /v1/videos/{id}/archive` hides terminal jobs locally without deleting the
remote asset; an unknown intent can be archived after ten minutes. Its
idempotency tombstone remains for the 30-day tracking window. Expired rows are
hidden from reads and purged on the next create; there is no background cleanup
or polling task.

`extensions["localrouter.video"]` reports capability flags and native reported
`cost_usd` for the whole job. Missing fees stay unknown; failed or cancelled
jobs are not assumed free. List totals cover the listed rows only. Repeated
status polls replace the job's fee snapshot rather than adding another charge.
Video fees are kept in this job ledger, separate from text-token estimates.
Strict local/no-retention policies, free-only routing, and cost/token ceilings
are rejected because these mappings cannot establish those guarantees. Request
quotas apply before creation. Active jobs, retained rows and concurrent transfers
are bounded. There is no background polling or automatic retry/fallback.

Try It Out provides generation, local list, manual status refresh and download.
The bundled demo returns labeled job metadata and never renders, fetches or
bills a video. Validation uses local mock servers rather than paid generations.
