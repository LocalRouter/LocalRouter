# Ollaya: one Local Embedded provider for decision models; Ollama decision models natively

## Goal
- One provider for open decision models instead of one per model family. [Ollaya](https://github.com/ollaya-dev/ollaya) ("Ollama for decision models", Apache-2.0) serves Laya, Kev, Decider, Von, Winnow, JEVK5, NLI, GLiClass, Qwen3Guard, CLM and `decision` behind a TypeSafe-compatible `/v1/systemone`, from one Rust binary with ONNX Runtime, MLX and llama.cpp. No Python or uv.
- Ollama 0.35 serves decision models (`nimble`, `tev1`) on `/v1/systemone` and marks them with `"decision"` in `capabilities`. The existing Ollama provider answers `/v1/systemone` natively for them.
- Kev, Laya, Von and Decider stop being offered in the add-provider list (`listed() == false`). Configured instances keep working.

## Step 0: tracking
Todo items per step.

## Step 1: `lr-engines` — Ollaya recipe and download
- `RecipeId::Ollaya` (`ollaya`): binaries `["ollaya"]`, no requirements. `probe_version` parses `ollaya --version` (`client version is X` on stderr when no server runs, `ollaya version is X` otherwise).
- Download options (LocalRouter's managed folder), pinned to a tested release (`OLLAYA_VERSION`, bumped by PR — Ollaya releases several times a day):
  - macOS arm64: `ollaya-darwin-arm64.tar.zst` plus `ollaya-darwin-arm64-mlx.tar.zst` (Apple GPU kernels), both extracted at the release root so `bin/` and `lib/ollaya/` stay together.
  - Linux x86_64: CPU (`ollaya-linux-amd64.tar.zst`); NVIDIA (adds `ollaya-linux-amd64-cuda.tar.zst`, ~1.5 GB, driver R580+).
  - Linux arm64: `ollaya-linux-arm64.tar.zst`.
  - Windows x64: CPU (`ollaya-windows-amd64.zip`); NVIDIA (adds `ollaya-windows-amd64-cuda.zip`).
  - Intel Macs: unsupported (no build). Linux notes: glibc 2.38+ (Ubuntu 24.04+).
- `DownloadSpec` gains `tag: Option<&str>` (pinned release instead of latest) and `extras_at_root: bool`. Extraction handles `.tar.zst` (pure-Rust `ruzstd` + `tar`), with the same escape checks as zip.
- `PortArg::Addr { var, host }`: the engine takes `host:port` in one variable (`OLLAYA_HOST=127.0.0.1:<port>`).

## Step 2: `lr-providers` — `embedded/ollaya.rs`
- One process per instance (key `ollaya:{instance}`), `ollaya serve`, env only: `OLLAYA_HOST=127.0.0.1:<port>`, `OLLAYA_API_KEY` (per-launch key), `OLLAYA_ORIGINS` unset (Ollaya's own allowlist is localhost/app origins; the key blocks the rest), `OLLAYA_DEVICE`, `OLLAYA_KEEP_ALIVE`, `OLLAYA_MAX_LOADED_MODELS`, optional `OLLAYA_MODELS`. Ready: `GET /` (unauthenticated 200). Idle unload of the process as for the other engines.
- Models directory: Ollaya's default `~/.ollaya/models` (shared with the Ollaya CLI and app), or the `models_dir` setting. Downloaded models are read from its manifests on disk, so health and the model list never start the engine.
- Catalog: the Ollaya library (name, family, size, guidance) plus any other model found on disk (e.g. made with `ollaya create`). Download = start the engine, `POST /api/pull` (NDJSON progress → `progress`); cancel aborts the pull; remove = `DELETE /api/delete`.
- Load/unload per model: `POST /api/decide {model, keep_alive: -1 | 0}`.
- `systemone()`: forwards to `/v1/systemone` with the model; no model → the first downloaded model in catalog order.
- `supports_chat=false`, `Capability::Decision`, free, `AlwaysFreeLocal`.
- Settings: `device` (auto|cpu|cuda|cuda:N|metal), `keep_alive_minutes`, `max_loaded_models`, `models_dir`, `idle_unload_minutes`, `binary_path`.

## Step 3: Ollama native System One
- Map Ollama's `decision` capability to `Capability::Decision`; remember decision models from `list_models` (and `/api/show` on a miss).
- `supports_systemone_model(m)` = decision model; `supports_systemone()` = any known; `systemone()` posts `/v1/systemone` (generic System One client). Chat models keep going through the router's translation.
- Letter mode for Ollama chat models: send `logprobs`/`top_logprobs` on `/api/chat` and return them (Ollama supports both); `supports_feature("logprobs")`.

## Step 4: the dedicated engines stay
- Measured on Apple Silicon (see Findings): Kev, Decider and Von need their own engines for GPU support, so Laya, Kev, Von and Decider all stay offered next to Ollaya.
- The command palette honours `listed` (it offered the legacy llama.cpp type).

## Step 5: app wiring
- `ProviderType::Ollaya`, string maps, factory registration, `is_local_provider`, engine tab recipe map, service icon.
- `validate_model_id` accepts Ollaya/Ollama names (`laya:en`, `kev:0.8b`).
- Website demo mocks and docs (providers page, System One section), CLAUDE.md provider list.

## Step 6: tests
- Recipe/asset/pinning/extraction (tar.zst, escapes), `PortArg::Addr`, version parsing.
- Ollaya provider against the fake engine (Ollaya-style routes: `/`, `/api/pull`, `/api/delete`, `/api/decide`, `/v1/systemone`): launch spec, catalog from manifests, download progress, systemone with default model, load/unload.
- Ollama: capability mapping, native vs translated routing per model, logprobs round trip (wiremock).
- Registry: hidden types are listed with `listed: false`.

## Mandatory final steps
Plan review; test-coverage review; bug hunt (key always set, loopback only, pinned version, manifests path, `:` in ids and file names, pull cancel); clippy/fmt/tests; commit.

## Findings
- Ollama 0.35 lists decision models with `"decision"` in `capabilities` (`/api/tags` and `/api/show`), serves TypeSafe's wire format on `/v1/systemone` (404 `{"error"}` for an unknown model, 400 for bad questions), and returns per-token `logprobs`/`top_logprobs` on native `/api/chat`. tev1's Modelfile system prompt is exactly the router's `LETTER_SYSTEM_PROMPT`.
- Ollaya v0.7.5 (Apache-2.0): single binary + `lib/ollaya` (llama.cpp dylibs, MLX metallib, optional CUDA packs laid out at the release root); `OLLAYA_HOST`, `OLLAYA_API_KEY`, `OLLAYA_MODELS`, `OLLAYA_DEVICE`, `OLLAYA_MAX_LOADED_MODELS`; `GET /` answers 200 without the key; `/api/pull` NDJSON; `/api/delete`; `/api/decide {model, keep_alive}` loads/unloads; no HF token, no pulling arbitrary HF repos; Nimble and Tev1 are not in its library.
- Native vs Ollaya on an M2 Max (20 identical requests, warm p50; machine loaded by other work):

  | Model | Native | Ollaya | Ollaya device | Answers |
  |---|---|---|---|---|
  | Kev 0.8B | 0.075 s (MLX bf16) | 6–12 s | CPU fp32 | top choice 20/20, noul Δ 0.003 |
  | Decider 0.8B | 0.39 s (torch mps) | 27 s | CPU fp32 | top choice 20/20, noul Δ 0.001 |
  | Von | 0.84 s (mps, Von 1.3) | 15 s | CPU fp32 | older checkpoint (1.1): noul Δ 0.25 |
  | Laya English | 0.35 s (mps) | 0.056 s | Metal (MLX) | identical probabilities |

  `OLLAYA_DEVICE=metal` fails to load Kev and Decider ("the MLX engine does not run layout kev-pointer-v1 / decider-slots-v1"). Score `confidence` is computed differently by Ollaya; Laya native and Decider native return extra fields Ollaya drops.
- Decision: keep all four dedicated providers offered for their GPU support. Ollaya's catalog says on its Kev/Decider/Von entries that the dedicated providers are faster on Macs.

## Follow-up (2026-10-03): models Ollaya publishes after the pin
Ollaya v0.8.0 and v0.9.0 added `nimble:9b`, `jeb:4b/9b/27b`, `cygnet:12b`, `jeeves:9b` and `clef:flash`, each needing that release or newer. Ollaya has no endpoint that lists its library, but its repository carries `registry/v2/library/<model>/manifests/<tag>` at every release tag, and each manifest's config blob has a description and context length (no minimum version).
- The pin moves to v0.9.0 and the built-in `LIBRARY` lists everything in it (works offline).
- `embedded/ollaya_registry.rs` reads the library at the installed engine's release tag and at Ollaya's latest release (three GitHub API calls; manifests from raw.githubusercontent.com, configs from ollaya.dev). Models the built-in list lacks are added to the Models tab; models only in a newer release are listed with `unavailable` ("Needs Ollaya vX or newer") and cannot be downloaded. Refreshed at most daily, or when the installed engine's release changes (checked every five minutes); offline, the built-in list stands alone.
- `EmbeddedCatalogModel.unavailable` is new; the Models tab disables Download and shows the reason.
- Tests: tree parsing, the merge against a mocked GitHub (`needs_newer`, a non-release engine, an unreachable registry), the provider's catalog, and an opt-in check against the real repository that the built-in list covers the pinned release.
