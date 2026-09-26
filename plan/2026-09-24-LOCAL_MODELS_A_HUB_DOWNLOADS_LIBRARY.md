# Phase 2 · Plan A: Hub client, downloads, model library, Hugging Face sign-in, hardware

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md` (revised 2026-09-24: engines are external installs found on PATH; this plan's scope is unchanged apart from CIMD sign-in). Read the overview first for privacy rules.

## Goal

A new crate `crates/lr-local-models` that can: search the Hub, inspect a repo, read GGUF headers remotely, estimate whether a file fits this machine, download files resumably and verifiably, keep a library of installed models with detected capabilities, and hold the user's Hugging Face credentials. No inference happens here.

## Step 0: tracking
- Todo items for every step below; save this plan with `./copy-plan.sh`.

## Step 1: crate skeleton and dependencies
- `crates/lr-local-models` (workspace member). Dependencies: `reqwest` (workspace; rustls/native as the workspace uses), `serde`, `serde_json`, `tokio`, `sha2` (workspace), `futures`, `parking_lot`, `lr-config`, `lr-utils`, `lr-types`, `lr-api-keys` (keychain), `sysinfo` (already used by `lr-coding-agents`; move to `[workspace.dependencies]`), `thiserror`.
- **No `hf-hub` for downloads.** Legacy 0.4/0.5 lacks Xet awareness and exposes little of the headers we need; 1.0 is a fresh rewrite whose non-Xet HTTP path does not resume and whose main branch stops reading env vars. Plain HTTPS through the Xet bridge supports Range resume and gives us `x-linked-etag`/`x-linked-size`/`x-repo-commit`, which is all we need. Existing crates keep their `hf-hub` 0.4 usage untouched.
- `lr_utils::paths::models_dir()` → `config_dir()/models` (new helper; the storage directory itself is configurable, see Step 8).

## Step 2: Hub client (`hub.rs`)
Base URL from config (`huggingface.endpoint`, default `https://huggingface.co`, so mirrors work). All calls take an optional bearer token.
- `search(query: HubSearch) -> HubPage<HubModelSummary>`: `GET /api/models` with `search`, repeatable `filter` (`gguf`, `onnx`, `sentence-similarity`, `text-generation`), `pipeline_tag`, `num_parameters=min:X,max:Y`, `sort` (downloads, likes, trendingScore, lastModified), `limit` (≤ 100 per page in the UI), `expand[]=downloads,likes,gated,pipeline_tag,library_name,tags,gguf,lastModified` (omit `chat_template` from rendering; it is large). Pagination via the `Link: rel="next"` cursor. Note: `library=gguf` is silently ignored; use `filter=gguf`.
- `model_info(repo, revision) -> HubModelInfo`: `/api/models/{repo}/revision/{rev}?blobs=true` → siblings with `size` and `lfs.sha256`, `gated` (`false|"auto"|"manual"`), `cardData` (license, `extra_gated_prompt`), `sha` (commit).
- `tree(repo, revision) -> Vec<HubFile>`: `/api/models/{repo}/tree/{rev}?recursive=true` with cursor paging; entries `{path, size, lfs{oid(sha256), size}}`.
- `readme(repo, revision) -> String`: `/{repo}/resolve/{rev}/README.md` (raw text, rendered sanitized by the UI).
- `whoami(token) -> HubUser`: `GET /api/whoami-v2` (username, avatar URL not loaded, token role/scopes, orgs).
- Error mapping from `X-Error-Code`: `GatedRepo` (401 without token, 403 without access) → `HubError::Gated { repo, message, requires_login }`; `EntryNotFound` → `NotFound`; bare 401 → `NotFoundOrPrivate`; 429 → `RateLimited { retry_after }` (anonymous limit is 500 API calls per 5 minutes).
- Tests: wiremock fixtures captured from real responses (store in `tests/fixtures/hub/`), cursor paging, gated/401/404 mapping, `expand` field parsing.

## Step 3: GGUF header reader and classification (`gguf.rs`, `classify.rs`)
- Minimal pure-Rust GGUF v2/v3 header parser (magic, version, tensor count, KV metadata; skip tensor data). Hard limits on string/array lengths and total header size (reject > 64 MB) so malformed files cannot exhaust memory. This also pre-validates files before an engine ever loads them (the Feb 2026 crash lesson), rejecting bad magic/version or impossible counts.
- `read_remote_header(url)`: fetch with HTTP Range in 2 MB chunks until the metadata is complete (cap 16 MB); `read_local_header(path)`.
- Extract: `general.architecture`, `general.type`, `general.file_type` (quant), `general.name`, `split.count`, `{arch}.context_length`, `.embedding_length`, `.block_count`, `.attention.head_count(_kv)`, `.attention.key_length/value_length`, `.attention.sliding_window(+_pattern)`, `.pooling_type`, `.expert_count`, `tokenizer.chat_template` (+ named variants), `clip.*` for projectors.
- `classify(header, siblings) -> ModelKind` in this order: `architecture == clip` → Projector; `general.type == adapter` → Adapter; `pooling_type == 4` or `cls.*` tensors → Reranker; `pooling_type ∈ {1,2,3}` or `attention.causal == false` → Embedding; causal with a chat template → Chat (plus `tools` if the template references tools or a `tool_use` template exists; `vision` if an `mmproj-*` sibling exists); else Completion. Never by file or repo name.
- Laya and Kev checkpoints are not in the library: those engines download their own weights (plan C).
- Quant label from `general.file_type` and the filename only as a display fallback.
- Tests: tiny synthetic GGUF headers built in-test for each branch; a truncated/garbage file; a split file.

## Step 4: hardware detection and fit estimate (`hardware.rs`, `fit.rs`)
- `HardwareInfo { os, arch, total_ram, available_ram, cpu_cores, unified_memory: bool, gpus: Vec<GpuInfo { name, backend, vram_total, vram_free }> }`.
  - RAM/CPU via `sysinfo`.
  - GPUs: the authoritative source is the installed engine's `llama-server --list-devices` (plan B exposes it); before an engine is installed, fall back to: macOS `sysctl hw.memsize` + Apple Silicon unified memory (Metal working set ≈ 75% of RAM, the llama.cpp default); Windows/Linux "unknown GPU" (fit shown against RAM only, labelled as such).
- `estimate(header, settings, hardware) -> FitEstimate { weights_bytes, kv_bytes, overhead_bytes, total, placement: FullGpu | PartialGpu | CpuOnly, verdict: Fits | Tight | TooLarge }`:
  - weights ≈ file size (sum of split parts) + mmproj size.
  - KV = Σ over layers `n_ctx_layer × n_head_kv × (k_len × bpe_K + v_len × bpe_V) × parallel_slots`, where SWA layers use `min(n_ctx, n_swa × n_seq + n_ubatch)`; bytes per element f16 = 2, q8_0 = 1.0625, q4_0 = 0.5625. Recurrent/hybrid architectures use the fixed state size.
  - overhead = compute buffer (KV(256 tokens) + 5% of weights, as huggingface.js does).
  - Budget: GPU VRAM minus 1 GiB headroom; unified memory = (RAM − 2.5 GiB) × 0.9; thresholds Fits ≤ 85%, Tight ≤ 100%, else TooLarge.
  - Also `max_context_that_fits()` so the UI can suggest a context length.
- Tests: the worked examples (Qwen3-8B Q4_K_M at 40k ctx f16 ≈ 5.03 GB weights + 5.6 GiB KV; Gemma-3-4B 32k with SWA ≈ 0.85 GB KV) as golden values within 5%.

## Step 5: downloader (`download.rs`)
Revive the design of the removed guardrails downloader (`git show ab3708ea^:crates/lr-guardrails/src/downloader.rs`), upgraded:
- **Job model:** `DownloadJob { id, repo, revision_commit, files: Vec<FileJob { path, size, sha256 }>, state: Queued | Running | Paused | Verifying | Done | Failed{error} | Cancelled, bytes_done, bytes_total, speed_bps }`. A job is one library entry (e.g. a GGUF + its mmproj; a Kev bundle's many files).
- **Queue:** `max_concurrent_downloads` jobs (default 2), one file at a time per job; per-destination lock so the same file is never written twice.
- **Resolution:** `GET {endpoint}/{repo}/resolve/{commit}/{path}` without following redirects automatically. Record `x-linked-etag` (SHA-256), `x-linked-size`, `x-repo-commit` from the first response; follow `302` to the CDN host **without** the Authorization header, and relative `307` (small files) on `huggingface.co` **with** it. Signed CDN URLs expire in ~1 hour: never persist them; re-resolve on resume or on 403 from the CDN.
- **Resume:** write to `{dest}.partial` plus `{dest}.partial.json` `{url_resolved_from, sha256, size, commit}`; on resume send a single `Range: bytes=N-` with `If-Range`; on `200` instead of `206`, or `416`, restart the file; if the stored SHA differs from the current one, restart.
- **Verification:** streaming SHA-256 while writing (hash state is not persisted; on resume, re-hash the existing partial bytes first); compare with the tree `lfs.oid` (never the `xetHash`); mismatch → delete partial, one automatic retry, then `Failed(ChecksumMismatch)`. Non-LFS small files are compared by size and git blob id is ignored.
- **Disk space:** check free space on the target volume before starting (`sysinfo` disks; replaces the shell-out in `lr-routellm`) with 1 GB margin; fail fast with a clear message.
- **Cancel / pause:** a `CancellationToken` per job; pause keeps partials, cancel deletes them.
- **Atomic install:** move verified files into the library path, then write the library entry last.
- **Progress:** a `DownloadEvents` trait (like `lr-embeddings`' `DownloadProgress`), throttled to 250 ms, carrying `{job_id, bytes_done, bytes_total, speed_bps, current_file, state}`; plan B wires it to Tauri events.
- **Resume across app restarts:** jobs persist in `models/downloads.json`; on startup, unfinished jobs appear as Paused (never auto-resume; network only on user action).
- Tests (wiremock): 302 to a second mock host (assert no Authorization header reaches it), relative 307, Range resume, 200-instead-of-206, 416, SHA mismatch then retry, expired CDN URL (403) → re-resolve, cancel mid-file, disk-space failure, persisted job reload.

## Step 6: library (`library.rs`)
- Layout: `{storage_dir}/hf/{org}/{repo}/{commit}/{path}`; imported files are referenced in place (not copied) under `imported/`. Index `{storage_dir}/library.json`:
  `LibraryEntry { id (stable slug, e.g. "qwen3-8b-q4_k_m"), source: Hf{repo, commit, files} | Imported{paths} | Bundle{manifest_id}, kind: ModelKind, capabilities: Vec<Capability>, engine: EngineKind (LlamaCpp | Laya | Kev), display_name, quant, size_bytes, context_length_trained, architecture, projector_file: Option, header_summary, installed_at, verified_at, license }`.
- The id is what users see as the model name: `localrouter_local/<id>` (overridable alias).
- Operations: `list`, `get`, `add_from_download`, `import_file(path)` (validates the header first), `remove(id, delete_files)`, `verify(id)` (re-hash), `rename(id, alias)`, `disk_usage()`.
- Migrations of the index file are versioned (`"version": 1`).
- Tests: add/list/remove, import of a valid and an invalid file, verify detecting a corrupted byte, id collision handling.

## Step 7: Hugging Face credentials (`auth.rs`)
Two methods; both store tokens in the keychain via `CachedKeychain` (service `LocalRouter-HuggingFace`), never in `settings.yaml` (the removed guardrails code kept `hf_token` in plain config: do not repeat that).
- **Token paste:** validate with `whoami-v2`; store; show username and token role. Recommend a fine-grained read token with "Read access to contents of all public gated repos you can access".
- **Sign in with Hugging Face (OAuth):** authorization code + PKCE (S256), public client (no secret), loopback redirect `http://127.0.0.1:{port}/callback` (Hugging Face accepts any port for a port-less loopback URI, RFC 8252), scopes `openid profile read-repos gated-repos`. The client id is a **Client ID Metadata Document** URL, `https://localrouter.ai/oauth/huggingface-client.json` (plan D Step 6), so no app registration on huggingface.co is needed. Implement as an `OAuthFlowConfig` for `lr_oauth::browser::OAuthFlowManager` (same machinery as the OpenAI Codex/Anthropic flows). Tokens expire after 30 days with rotating refresh tokens: reuse the `token_source` refresh logic. The device-code flow (`POST /oauth/device`) is the fallback for the headless CLI/Docker mode.
- `HfCredentials::token() -> Option<String>` is used by the Hub client and downloader, and passed as `HF_TOKEN` to the Laya and Kev engines (plan C); `sign_out()` deletes keychain entries.
- Gated-repo UX data: when a repo is gated and the user lacks access, return the gate prompt from `cardData.extra_gated_prompt` and the repo URL so the UI can say "Request access on huggingface.co" (there is no API to request access).
- Tests: token validation with a mocked `whoami-v2`; OAuth config builds the right authorize URL (PKCE challenge, scopes, redirect); refresh path with a mocked token endpoint.

## Step 8: configuration (`lr-config`)
Add `AppConfig.local_models: LocalModelsConfig` (`#[serde(default)]`, config version bump to 28 as a documented no-op migration):
```rust
pub struct LocalModelsConfig {
    pub enabled: bool,                       // default false; enabling creates the built-in provider (plan B)
    pub storage_dir: Option<PathBuf>,        // None = config_dir()/models
    pub huggingface: HuggingFaceSettings {   // endpoint (default "https://huggingface.co"), auth_method: None | Token | OAuth (token itself in keychain)
    pub downloads: DownloadSettings {        // max_concurrent (2), verify_checksums (true, not user-disableable in UI)
    pub memory_guardrail: MemoryGuardrail,   // Off | Relaxed | Balanced (default) | Strict  (thresholds for Fits/Tight blocking)
    // engine + run settings live in plan B's EngineSettings / ModelRunDefaults
}
```
- Changing `storage_dir` offers "move existing models" (copy + verify + delete old) or "start empty"; refuse while downloads run.

## Interfaces consumed by other plans
- Plan B: `Library::list/get`, `LibraryEntry`, `FitEstimate`, `HardwareInfo`, `DownloadManager` + `DownloadEvents`, `HfCredentials`.
- Plan C: `HfCredentials::token()` (passed as `HF_TOKEN`) and `HardwareInfo` (Kev checkpoint fit).
- Plan D: types serialized through Tauri commands defined in plan B.

## Mandatory final steps
1. Plan review; 2. test-coverage review; 3. bug hunt (redirect auth leakage, resume correctness, SHA over resumed files, path traversal in repo file paths (reject `..` and absolute paths), partial-file cleanup, keychain errors); 4. clippy/fmt/targeted tests and commit.
