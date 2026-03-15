# Files API + Batch API

## Context

The Files API provides local file storage for uploads, and the Batch API enables async processing of large JSONL workloads. Files is a prerequisite for Batches. These are grouped because Batches depends on Files, but both are independent from all other API plans.

## Endpoints

### Files API
- `POST /v1/files` — Upload file (multipart/form-data)
- `GET /v1/files` — List files (optional purpose filter)
- `GET /v1/files/:file_id` — Get file metadata
- `DELETE /v1/files/:file_id` — Delete file
- `GET /v1/files/:file_id/content` — Download file content

### Batch API
- `POST /v1/batches` — Create batch job
- `GET /v1/batches` — List batch jobs
- `GET /v1/batches/:batch_id` — Get batch status
- `POST /v1/batches/:batch_id/cancel` — Cancel batch job

## Provider Coverage

### Files API
Files are **local to LocalRouter** — no provider interaction needed. Files are stored on disk and referenced by batch/fine-tuning jobs.

### Batch API (Native)

| Provider | Batches |
|----------|---------|
| OpenAI | Y |
| Groq | Y |
| TogetherAI | Y |
| All others | N |

## Translation Layer Feasibility

| Feature | Translation Feasible? | Complexity | Notes |
|---------|----------------------|------------|-------|
| Files API | **N/A** | N/A | Purely local storage, no provider involvement. |
| Batch API | **Yes (High feasibility)** | Medium | **This is a strong candidate for translation.** For providers without native batch support, LocalRouter can process the JSONL file locally by sending each line as an individual request through the existing router (chat completions, embeddings, etc.). This means ALL providers effectively support batches — just without the 50% cost discount that native batch APIs offer. |

**Recommendation:**
- **Files:** Local-only, implement once.
- **Batches:** Implement two modes:
  1. **Native proxy** — For OpenAI, Groq, TogetherAI: proxy the batch to the provider's native batch API (preserves cost discount).
  2. **Translation layer** — For all other providers: process JSONL locally, send individual requests through existing router. No cost discount, but full functionality.

## Architecture

### New Crate: `crates/lr-files/`
```
lr-files/
  Cargo.toml
  src/
    lib.rs       — FileManager struct, FileMetadata, FilePurpose enum
    storage.rs   — Filesystem operations: save, list, get, delete, read content
```

**Storage layout:**
```
{config_dir}/files/
  {file_id}.meta.json    — FileMetadata (id, purpose, filename, bytes, created_at, status)
  {file_id}.data         — Raw file content
```

- `file_id` format: `file-{uuid}`
- FilePurpose enum: `Batch`, `FineTune`, `Assistants`, `Vision`
- FileStatus enum: `Uploaded`, `Processed`, `Error`

### New Crate: `crates/lr-batches/`
```
lr-batches/
  Cargo.toml
  src/
    lib.rs        — BatchManager struct, public API
    job.rs        — BatchJob, BatchStatus enum, BatchRequest/BatchResponse
    executor.rs   — Background processing (tokio::spawn)
    native.rs     — Native provider batch proxy (OpenAI, Groq, TogetherAI)
    translated.rs — Translation layer: JSONL → individual router requests
    storage.rs    — Persistent state in {config_dir}/batches/
```

**Batch job lifecycle:** `validating → in_progress → completed | failed | expired | cancelled`

**Translation layer design:**
- Read JSONL file from lr-files
- Parse each line as a batch request: `{"custom_id": "...", "method": "POST", "url": "/v1/chat/completions", "body": {...}}`
- For each line, dispatch through existing `lr-router` (complete, embed, etc.)
- Collect results into output JSONL file
- Concurrency: configurable (default 5 concurrent requests per batch)
- Store output file back in lr-files

**Native proxy design:**
- Upload input file to provider's Files API
- Create batch via provider's Batch API
- Poll for status updates
- Download result file when complete
- Store in lr-files

### Provider Trait
**No new trait methods for Files** — purely local.

For native batch proxy, add to `ModelProvider`:
```rust
async fn create_batch(&self, request: BatchCreateRequest) -> AppResult<BatchObject>
async fn get_batch(&self, batch_id: &str) -> AppResult<BatchObject>
async fn cancel_batch(&self, batch_id: &str) -> AppResult<BatchObject>
async fn list_batches(&self, params: BatchListParams) -> AppResult<BatchListResponse>
// File operations for native batch providers
async fn upload_file(&self, file: Vec<u8>, filename: &str, purpose: &str) -> AppResult<ProviderFileObject>
async fn get_file_content(&self, file_id: &str) -> AppResult<Vec<u8>>
```

### Provider Implementations (Native Batch)
- `crates/lr-providers/src/openai.rs`
- `crates/lr-providers/src/groq.rs`
- `crates/lr-providers/src/togetherai.rs`

## Files to Modify

### Files API
- **New crate:** `crates/lr-files/` (lib.rs, storage.rs)
- `Cargo.toml` (workspace) — add lr-files to members
- `crates/lr-server/Cargo.toml` — add lr-files dependency
- `crates/lr-server/src/state.rs` — add `file_manager: Arc<lr_files::FileManager>` to AppState
- **New file:** `crates/lr-server/src/routes/files.rs`
- `crates/lr-server/src/routes/mod.rs` — add module
- `crates/lr-server/src/lib.rs` — register routes (512MB body limit for file upload route)
- `crates/lr-server/src/openapi/mod.rs` — register
- `src-tauri/src/lib.rs` or wherever AppState is constructed — instantiate FileManager

### Batch API
- **New crate:** `crates/lr-batches/` (lib.rs, job.rs, executor.rs, native.rs, translated.rs, storage.rs)
- `Cargo.toml` (workspace) — add lr-batches to members
- `crates/lr-server/Cargo.toml` — add lr-batches dependency
- `crates/lr-server/src/state.rs` — add `batch_manager: Arc<lr_batches::BatchManager>` to AppState
- `crates/lr-providers/src/lib.rs` — batch trait methods (for native providers)
- `crates/lr-providers/src/openai.rs` — native batch + file upload
- `crates/lr-providers/src/groq.rs` — native batch + file upload
- `crates/lr-providers/src/togetherai.rs` — native batch + file upload
- **New file:** `crates/lr-server/src/routes/batches.rs`
- `crates/lr-server/src/routes/mod.rs` — add module
- `crates/lr-server/src/lib.rs` — register routes
- `crates/lr-server/src/openapi/mod.rs` — register

## Cross-Cutting Features Applicability

### Files API

| Feature | Applies? | Notes |
|---------|----------|-------|
| **Auth (API Key)** | **Yes** | Must authenticate to upload/list/delete files |
| **Permission checks** | **Minimal** | Check client is enabled. No model/provider checks needed (local storage). |
| **Rate limiting** | **Yes** | Rate limit uploads to prevent abuse |
| **Secret scanning** | **No** | Files are opaque binary — scanning content of arbitrary files is out of scope |
| **Guardrails** | **No** | Not applicable |
| **Prompt compression** | **No** | Not applicable |
| **RouteLLM** | **No** | Not applicable |
| **Model firewall** | **No** | No model involved |
| **Token tracking** | **No** | No tokens consumed |
| **Cost calculation** | **No** | Local storage is free |
| **Generation tracking** | **No** | Not a generation |
| **Metrics/logging** | **Yes** | Log file operations (upload, delete) |
| **Client activity** | **Yes** | Record activity |

### Batch API

| Feature | Applies? | Notes |
|---------|----------|-------|
| **Auth (API Key)** | **Yes** | Standard auth |
| **Permission checks** | **Yes** | For translated batches: each individual request goes through full permission checks via the router. For native batches: check client has access to the provider. |
| **Rate limiting** | **Yes (translated only)** | Each individual request in translated batch hits rate limits. Native batches: single rate limit check at batch creation. |
| **Secret scanning** | **Yes (translated only)** | Each individual chat/completion request in translated batch can be secret-scanned. Native batches: not scanned (provider handles). |
| **Guardrails** | **Yes (translated only)** | Each individual request in translated batch goes through guardrails. Native batches: provider handles. |
| **Prompt compression** | **Yes (translated only)** | Per-request in translated batch. |
| **RouteLLM** | **Yes (translated only)** | Per-request in translated batch if model is auto. |
| **Model firewall** | **Partial** | Firewall popup on batch creation (approve all requests). Per-request firewall not practical for batches. |
| **Token tracking** | **Yes** | Aggregate tokens across all requests in batch. |
| **Cost calculation** | **Yes** | Sum costs across all requests. Native batches: use batch pricing (typically 50% discount). |
| **Generation tracking** | **Yes** | One generation ID per batch, plus individual generation IDs per request in translated mode. |
| **Metrics/logging** | **Yes** | Log batch creation, completion, individual requests (translated). |
| **Client activity** | **Yes** | Record activity |

## Implementation Order
1. Files API first (no external dependencies)
2. Batch API second (depends on Files API)
3. Native batch proxy for OpenAI/Groq/TogetherAI
4. Translation layer for all other providers

## Verification
1. `cargo test` — file storage CRUD, batch job lifecycle
2. Upload file via `curl -F "file=@test.jsonl" -F "purpose=batch" localhost:3625/v1/files`
3. List files: `curl localhost:3625/v1/files`
4. Create batch: `curl -X POST localhost:3625/v1/batches -d '{"input_file_id":"file-xxx","endpoint":"/v1/chat/completions","completion_window":"24h"}'`
5. Poll batch: `curl localhost:3625/v1/batches/{batch_id}`
6. Test translated batch: create batch targeting a provider without native batch support
7. Verify `/openapi.json` includes files + batches paths
