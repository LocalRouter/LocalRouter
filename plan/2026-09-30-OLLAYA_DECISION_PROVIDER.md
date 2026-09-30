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

## Step 4: hide Kev, Laya, Von, Decider
- `listed() -> false` on the four factories (the legacy llama.cpp precedent). The command palette also honours `listed`.
- Decision recorded in this plan after measuring native Kev/Decider against Ollaya on Apple Silicon (see Findings).

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
(filled in during implementation)
