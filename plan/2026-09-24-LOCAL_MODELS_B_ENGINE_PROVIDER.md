# Phase 2 · Plan B: llama.cpp engine, process supervisor, built-in provider

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md`. Depends on plan A (library, fit, downloads, credentials).

## Goal

Install a pinned `llama-server` build for this machine on request, run one engine process per loaded model on `127.0.0.1`, and expose the library through a single built-in provider, `localrouter_local`, so local models work everywhere a provider does.

## Step 0: tracking
- Todo items per step; save this plan with `./copy-plan.sh`.

## Step 1 (M0 prerequisite): fix the provider registry leak
- `HealthCheckManager` (`crates/lr-providers/src/health.rs:35-54`) has no unregister, and `ProviderRegistry::update_provider`/`remove_provider` (`registry.rs:505-584`) drop instances from the map but leave the old `Arc<dyn ModelProvider>` in the health manager. A provider that owns engine processes would never release them. Also `check_health(name)` matches on `provider.name()` (the type name), which can return a stale instance.
- Change: key health entries by instance name; add `unregister_provider(instance_name)`; call it from update and remove; test that the old `Arc` is dropped (`Arc::strong_count` / a Drop flag in a mock provider).

## Step 2: engine pack manifest and installer (`crates/lr-engines`)
- New crate `lr-engines` (pure Rust; no native build deps).
- `engines.json` compiled in via `include_str!`: one entry per `(engine, version, os, arch, backend)`:
  `{ engine: "llamacpp", version: "b11167", os, arch, backend: metal|cpu|vulkan|cuda12|cuda13, url, sha256, size, archive: zip|tar.gz, entrypoint: "llama-server[.exe]", min_driver: Option<String>, notes }`.
  - llama.cpp URLs point at upstream GitHub release assets (e.g. `llama-b11167-bin-macos-arm64.zip`, `-win-vulkan-x64.zip`, `-ubuntu-vulkan-x64.zip`, `-win-cuda-12.4-x64.zip` + its `cudart` zip as a second archive). Plan C adds our own Laya/Kev engine entries.
  - `scripts/update-engine-manifest.sh <build>`: downloads each asset, computes SHA-256 and size, rewrites `engines.json` (run by a developer; reviewed in a PR). Bumping llama.cpp is a normal PR with release notes.
- `recommended_backend(hardware) -> Backend`: macOS arm64 → metal; macOS x64 → cpu; Windows/Linux → vulkan when a Vulkan-capable GPU is detected (`vulkaninfo`-free check: try the installed CPU pack's `--list-devices`, else offer both), cuda as an explicit option for NVIDIA users (large download, labelled with size), else cpu. Every platform can install cpu.
- `EngineInstaller`: download (reuses plan A's downloader primitives: resume, SHA-256, progress, cancel), extract to `{config_dir}/engines/{engine}/{version}-{backend}/` (zip/tar.gz with path-traversal checks), `chmod +x` the entrypoint, on macOS remove `com.apple.quarantine` if present, verify by running `llama-server --version` (5 s timeout), then record in `engines/installed.json`.
- `uninstall(pack)` refuses while a process from it runs. `check_updates()` compares installed versions with the compiled manifest (no network; newer packs arrive with app updates). The UI can show "An update is available" after an app update.
- Platform risks to verify in a spike (Step 9): macOS runs upstream ad-hoc-signed binaries when not quarantined; Windows files written by the app carry no Mark-of-the-Web; Flatpak and Snap sandboxes allow executing files from the app's data dir; AppImage/Docker headless (`src-tauri/src/cli.rs`) works with the CPU pack.

## Step 3: process supervisor
`EngineSupervisor` owns every running engine process:
- **Launch:** `tokio::process::Command` with `kill_on_drop(true)`; free port found by binding `127.0.0.1:0` then passing it (retry on race); random 32-byte hex API key per process; env cleared except PATH/HOME/TMP and backend-specific vars; stdout/stderr to `logs/engines/{instance}.log` (rotating, 10 MB) plus a 500-line ring buffer for the UI.
- **Readiness:** poll `GET /health` (llama-server answers 503 while loading, 200 when ready) with a timeout scaled by model size (default `start_timeout_secs` 300).
- **Crash handling:** watch the exit status; restart with backoff (1 s, 2 s, 4 s; at most 3 restarts in 5 minutes), then mark `Failed { exit_code, last_log_lines }` and emit an event. A load failure caused by a bad file (GGUF assert) shows the log tail and offers "Verify file" / "Delete and re-download".
- **Orphans:** `run/engines.json` records pid, start time and executable path. On startup, kill processes whose pid and executable path both match a stale record (guards against PID reuse). Linux sets `PR_SET_PDEATHSIG` via `pre_exec`; Windows assigns children to a Job Object with kill-on-close; macOS relies on the startup cleanup.
- **Shutdown:** on app exit (Tauri `RunEvent::ExitRequested`/`Exit`, and the CLI server shutdown path), terminate gracefully, then kill after 5 s. Disabling the feature stops everything.
- **Idle unload:** background tick every 30 s stops processes idle longer than their `idle_unload_secs` (default 900; 0 = never; per-model "keep loaded").
- **Memory guardrail:** before starting a model, estimate its footprint (plan A `FitEstimate`) plus running engines' footprints; if over budget, evict least-recently-used idle models (never ones with in-flight requests); if still over and the guardrail is Balanced/Strict, fail with `AppError::Provider("insufficient memory to load '{model}' (needs X, available Y)")` (router classifies as `Unreachable` so auto-routing falls through; add the phrase to `RouterError::classify`). Relaxed warns only; Off skips the check. `max_loaded_models` (default 1) caps concurrency independently.
- **Concurrency:** starting the same model twice is de-duplicated (per-model async mutex / shared future); in-flight requests hold a guard so eviction and idle unload skip that process.

## Step 4: launch arguments (`args.rs`)
`llama_server_args(entry, settings, pack) -> Vec<String>`, unit-tested per pinned version:
- Always: `--host 127.0.0.1 --port P --api-key K --no-webui --offline --jinja -m <path>`.
- Settings: `-c <ctx>` (0 = trained; Auto = fit-based, see below), `-ngl` / `--fit` (Auto uses llama.cpp's `--fit`, on by default in current builds), `-fa auto|on|off`, `-ctk/-ctv f16|q8_0|q4_0` (quantized V requires flash attention; enforce), `-np <slots>` (Auto = engine default), `-b/-ub`, `-t <threads>`, `--no-mmap`, `--mlock`.
- Kind: Vision → `--mmproj <projector>`; Embedding → `--embeddings --pooling <from header>`; Reranker → `--rerank`.
- Auto context: largest of {4096, 8192, 16384, 32768} that fits per plan A's `max_context_that_fits`, capped at the trained context (Ollama uses VRAM tiers; we use the estimator).
- No flag with user-controlled free text is passed through a shell; args are a vector.

## Step 5: the built-in provider `localrouter_local`
- `lr_config::ProviderType::LocalRouterLocal` (`#[serde(rename = "localrouter_local")]`), factory `LocalModelsProviderFactory` (category Local, `AlwaysFreeLocal`, `catalog_provider_id() -> None`, no setup parameters). Wire into `main.rs` (factory registration, the exhaustive `ProviderType` match), `provider_type_str_to_enum`, `is_local_provider`.
- Exactly one instance, created when `local_models.enabled` flips to true (and on startup if enabled), removed when disabled. Instance name "Local models".
- `LocalModelsProvider` holds `Arc<Library>`, `Arc<EngineSupervisor>`, settings; implements `ModelProvider`:
  - `list_models()` → library entries as `ModelInfo` (capabilities from plan A classification: Chat, Completion, Vision, FunctionCalling, Embedding, Decision; `context_window` = effective context; `supports_streaming` for chat).
  - `complete` / `stream_complete` / `embed`: `ensure_running(model_id)` → an internal `LlamaCppProvider` bound to that process's port and key → delegate with the bare model name. Streams keep an in-flight guard alive until dropped, so client disconnects both stop generation (llama-server stops on closed connection) and release the guard.
  - `systemone`: for Laya/Kev entries → internal `SystemOneProvider` (plan C); chat models go through the router's Phase 1b translation.
  - `supports_feature("logprobs") == true` (llama-server `top_logprobs`); `supports_embeddings() == true`; `supports_chat() == true`; `supports_systemone()` see Step 6.
  - `health_check()` never starts a process: Healthy if an engine pack is installed and the library has at least one entry; Degraded if an engine failed in the last 5 minutes; Unhealthy if no pack is installed.
  - `get_pricing` → free. `supports_pull() == true`; `pull_model("hf.co/{repo}:{quant}")` downloads through plan A (lets the guardrails safety-model picker pull local models in M5, like Ollama).
- Model names seen by clients: `localrouter_local/<library id>`.

## Step 6: router adjustment for mixed providers
The built-in provider serves both chat models and native System One models. Today `execute_systemone_request` decides native vs translation per provider (`supports_systemone()`). Add `fn supports_systemone_model(&self, model: &str) -> bool { self.supports_systemone() }` to `ModelProvider`, use it in `crates/lr-router/src/systemone.rs`, and override it in `LocalModelsProvider` (true only for Laya/Kev entries). Same for `EndpointType::is_supported_by_provider` callers where a model is known. Tests: a mixed mock provider routes one model natively and another through translation.

## Step 7: settings, commands and events
Config (extends plan A's `LocalModelsConfig`):
```rust
pub struct EngineSettings { backend: BackendPref /* Auto|Metal|Vulkan|Cuda|Cpu */, idle_unload_secs: u64 /* 900 */,
    max_loaded_models: u32 /* 1 */, start_timeout_secs: u64 /* 300 */, threads: Option<u32>, preload_on_startup: Vec<String> }
pub struct ModelRunDefaults { context: ContextSetting /* Auto | Trained | Fixed(u32) */, gpu_offload: GpuOffload /* Auto | Max | Off | Layers(u32) */,
    flash_attention: Tristate /* Auto */, kv_cache_type: KvType /* F16 */, parallel_slots: Option<u32>, batch: Option<u32>, ubatch: Option<u32>, mmap: bool /* true */ }
pub struct ModelOverrides { /* every ModelRunDefaults field as Option, plus keep_loaded: bool, alias: Option<String> */ }   // keyed by library id in LocalModelsConfig.models
```
Tauri commands (all in `src-tauri/src/ui/commands_local_models.rs`; every one mirrored in `src/types/tauri-commands.ts` and `website/src/components/demo/TauriMockSetup.ts`, per CLAUDE.md):

| Command | Params | Returns |
|---|---|---|
| `local_models_get_config` / `local_models_update_config` | `config` | `LocalModelsConfig` |
| `local_models_hardware` | none | `HardwareInfo` (+ engine device list if installed) |
| `hf_search` | `HubSearch` | `HubPage<HubModelSummary>` |
| `hf_model_details` | `repo, revision?` | `HubRepoDetails { info, files: Vec<HubFileView { path, size, quant, kind_hint, fit: Option<FitEstimate> }>, readme_text, gated, gate_prompt }` |
| `hf_inspect_file` | `repo, revision, path` | `GgufSummary` + `FitEstimate` (remote header read) |
| `local_models_download_start` | `repo, revision, files` | `job_id` |
| `local_models_download_pause` / `_resume` / `_cancel` | `job_id` | none |
| `local_models_downloads` | none | `Vec<DownloadJobView>` |
| `local_models_library` | none | `Vec<LibraryEntryView>` (+ runtime state: unloaded/loading/loaded/failed, memory, last used) |
| `local_models_import_file` | `path` | `LibraryEntryView` |
| `local_models_delete` / `_verify` / `_rename` | `id`, … | … |
| `local_models_load` / `_unload` / `_unload_all` | `id` | none |
| `local_models_set_overrides` | `id, overrides` | none |
| `local_models_engines` | none | `Vec<EnginePackView>` (available for this platform, installed, recommended, size) |
| `local_models_engine_install` / `_uninstall` | `pack_id` | `job_id` / none |
| `local_models_engine_logs` | `instance` | `Vec<String>` |
| `hf_account` / `hf_set_token` / `hf_sign_in_start` / `hf_sign_in_poll` / `hf_sign_out` | … | `HfAccountView { method, username, scopes, expires_at }` |
| `local_models_open_folder` | `id?` | none |

Events: `local-models-download-progress`, `local-models-download-state`, `local-models-engine-install-progress`, `local-models-model-state` (loading/loaded/unloaded/failed with message), `local-models-library-changed`, `providers-changed` (existing) when the built-in provider is created or removed.

## Step 8: tests
- Supervisor with a **fake engine**: a tiny `[[bin]] lr-fake-engine` in `lr-engines` (axum; implements `/health` with configurable loading delay, `/v1/chat/completions` streaming, `/v1/embeddings`, can be told to crash or hang). Integration tests via `CARGO_BIN_EXE_lr-fake-engine`: start, readiness wait, request, idle unload, crash → restart → fail after limit, orphan cleanup, eviction under a small memory budget, concurrent `ensure_running` de-dup, graceful shutdown.
- Args builder snapshot tests for each kind and setting.
- Provider: list/complete/embed delegation against the fake engine; health check never spawns; pull via mocked downloads.
- Registry leak test (Step 1); router mixed-provider test (Step 6).
- Installer: wiremock-served zip/tar.gz with path traversal attempt, SHA mismatch, quarantine attribute removal on macOS (cfg-gated).
- Real-engine e2e, `#[ignore]` and gated by `LOCALROUTER_E2E_ENGINE=1`: installs the pinned CPU pack and a sub-200 MB instruct GGUF pinned by SHA-256; runs chat (stream), tool call, JSON mode, logprobs, embeddings, and a System One letter-mode request through the gateway.

## Step 9: spike before building Steps 2-3 (half a day)
Verify on the pinned build and write findings into this plan: (1) upstream macOS arm64 zip runs when written by the app (no quarantine); (2) Windows Vulkan zip runs from `%APPDATA%` with its DLLs; (3) Linux Vulkan build on Ubuntu 22.04 (CI runner) and inside the Flatpak sandbox; (4) `--list-devices` output format for GPU/VRAM detection; (5) `--offline` + `--jinja` + `--api-key` flags on that build; (6) health endpoint semantics while loading.

## Mandatory final steps
1. Plan review; 2. test-coverage review; 3. bug hunt (orphans, port races, eviction during in-flight requests, restart loops, secrets in logs (API key must not be logged), path traversal on extract, shutdown ordering); 4. clippy/fmt/targeted tests and commit.
