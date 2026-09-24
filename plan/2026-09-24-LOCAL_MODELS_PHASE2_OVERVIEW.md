# Phase 2 overview: Local Models (download and run Hugging Face models inside LocalRouter)

## Context

Phase 1 (`plan/2026-09-24-SYSTEMONE_ENDPOINT.md`) added `/v1/systemone` and providers that talk to *externally run* servers (Ollama, LM Studio, llama.cpp, laya-serve, kev.serve). Phase 2 lets a user find a model on Hugging Face, download it, and serve it from LocalRouter with no other software installed: chat and completions (streaming, tools, JSON output, logprobs), embeddings, vision where the model supports it, and System One decisions (Laya, Kev).

Two lessons from this repo's history shape the design:
- **Embedded llama.cpp was tried and removed** (`plan/2026-02-23-REMOVE_EMBEDDED_LLAMA_CPP.md`, commit `ab3708ea`): large C++ build, CI weight, and malformed GGUF files calling `abort()` and killing the app (`plan/2026-02-17-FIX_SAFETY_MODEL_GGUF_LOADING_CRASH_REMOVE_BROKEN.md`). The release profile also sets `panic = "abort"`.
- **`llama-cpp-2` 0.1.147 (June 2026) removed its Jinja chat templating and tool-call parsing**, so in-process use would mean owning that layer.

## Architecture decision: engines are separate processes, installed on demand

LocalRouter never loads model weights into its own process. It manages **engines**: small server executables that it downloads (only when the user asks), verifies, starts on `127.0.0.1` with a random API key, health-checks, restarts, and stops. LocalRouter then talks to them with provider code it already has.

| Model kind | Engine | Talks through (existing code) |
|---|---|---|
| GGUF chat / completion / vision / embedding / rerank | upstream **llama.cpp `llama-server`**, pinned release build per platform and GPU backend (Metal, Vulkan, CUDA, CPU) | `LlamaCppProvider` (OpenAI-compatible; logprobs added in Phase 1) |
| Laya (ONNX, `receptron/laya-onnx`) | **`localrouter-laya-engine`**: our Rust binary (`ort` + `tokenizers`), built and published by our CI | `SystemOneProvider` (Laya flavor) |
| Kev (Qwen + LoRA + pointer head) | **`localrouter-kev-engine`**: our Rust binary wrapping `kev-rs`'s `kev-core` (MLX on Apple Silicon, Candle CPU elsewhere) | `SystemOneProvider` (Kev flavor) |

Why this over in-process:
- A crash, GPU driver fault or `GGML_ASSERT` kills an engine, not the app. The supervisor reports it and restarts with backoff.
- Unloading a model frees VRAM completely (process exit).
- The user's GPU gets the right backend (Vulkan or CUDA on Windows/Linux, Metal on Apple Silicon) without shipping hundreds of MB of CUDA in the app.
- The app binary, main CI and every package format (Flatpak, Scoop, Docker take only the main binary) stay unchanged.
- llama.cpp can be updated independently of app releases.
- Client disconnects already cancel generation: dropping the HTTP stream to `llama-server` stops it (the existing `chat.rs` stream-drop path).

Costs, accepted: process supervision (ports, orphans, logs), engine updates to curate, and first-use downloads of the engine as well as the model.

Rejected alternatives: in-process `llama-cpp-2` (crash coupling, own Jinja/tool layer, CI weight); `llama-cpp-4` (wraps upstream chat layer but single maintainer); mistral.rs (no Vulkan, Windows GPU only via CUDA, crates.io lagging); Candle (no general GGUF coverage or grammar); bundling engines inside the installer (package-format and size problems).

## One built-in provider

A new provider type **`localrouter_local`** ("Local models (built-in)", category Local, `AlwaysFreeLocal`, no catalog id) with exactly one instance, created when the feature is enabled. Its models are the installed library entries; it delegates each request to the engine that serves that model, starting it first if needed. It is distinct from the existing `huggingface` provider type (the HF Inference router) and from the external `llamacpp` / `laya` / `kev` types, which stay as they are.

## Hugging Face access

Plain HTTPS to the Hub API (search, model info, file tree, resolve) with our own resumable downloader: SHA-256 checked against the Hub's LFS `oid`, Range resume, re-resolving signed CDN URLs after expiry, token sent only to `huggingface.co`. Sign-in is optional and only needed for gated or private repos: **"Sign in with Hugging Face"** (OAuth authorization code + PKCE, public client, loopback redirect, refresh tokens) or **paste an access token**. Tokens live in the keychain.

## Privacy (CLAUDE.md: network only on user action)

- No Hub request until the user searches, opens a repo, or starts a download. The curated "Recommended" list ships inside the app as static data.
- Engine installs and updates only on explicit click ("Install engine", "Check for engine updates").
- Model cards render as sanitized text: images and remote embeds are stripped, links open in the system browser.
- Engines run with `--offline`, bind `127.0.0.1`, and require a per-launch random API key.

## Work streams (separate plans)

| Plan | Scope | Depends on |
|---|---|---|
| **A: Hub, downloads, library, sign-in, hardware** | new crate `lr-local-models`: Hub client, downloader, on-disk library, GGUF header parser, model classification, hardware detection, fit estimator, HF OAuth/token | none |
| **B: llama.cpp engine and built-in provider** | new crate `lr-engines`: engine pack manifest and installer, process supervisor; `localrouter_local` provider; load/unload; config; Tauri commands | A (library, fit) |
| **C: System One engines (Laya, Kev)** | `engines/laya-engine`, `engines/kev-engine` (own workspaces), engines CI workflow, pack manifest entries, Kev artifact assembly | B (supervisor, packs); A (downloads) |
| **D: UI and website** | Local Models view (Discover, Library, Downloads, Engines, Settings), HF account card, provider detail, Try It Out and Monitor touches, tray, docs, demo mocks | A, B command/type contracts (can start against mocks) |

Shared contracts that let the streams run in parallel are fixed in each plan's "Interfaces" section: Tauri command names and payloads (D consumes), `LibraryEntry` / `EngineStatus` types (B and C consume), event names.

## Milestones

1. **M0, prerequisites (in B):** fix the registry leak where `update_provider`/`remove_provider` never unregister the old provider from `HealthCheckManager`; add hardware detection (A).
2. **M1, download and library (A + D):** search, download, verify, list, delete. Useful on its own for "Import GGUF into LM Studio/Ollama folders" later, but mainly the base for M2.
3. **M2, chat and embeddings end to end (B + D):** Metal (macOS arm64), CPU (all), Vulkan (Windows/Linux); CUDA as an optional pack.
4. **M3, Laya engine (C).**
5. **M4, Kev engine (C):** Apple Silicon (kev-0.8b, kev-4b via MLX) and CPU (kev-0.6b) as supported by kev-rs today.
6. **M5, polish:** tray, Monitor events, website docs and demo, guardrails safety-model picker integration.

## Cross-cutting features (all reuse existing paths, no special cases)

Local models appear in `/v1/models`, strategies, the model firewall, auto-routing, free-tier (always free), Try It Out, monitoring, metrics and access logs like any provider's. System One translation (Phase 1b) works against local chat models in letter mode, because llama-server returns `top_logprobs`. Guardrails, secret scanning and compression apply unchanged. The guardrails safety-model picker gains the built-in provider as a "pullable" source (M5).

## Decisions needed from the user

1. **Engines downloaded on demand vs bundled in the installer.** Recommended: on demand (above). Bundling would inflate every installer and break Flatpak/Scoop/Docker packaging.
2. **Register a "LocalRouter" OAuth app on huggingface.co** (free; produces a public client id committed to source). Without it, sign-in is token-paste only.
3. **Engines CI (plan C) runs macOS, Windows and Linux builds.** GitHub Actions macOS minutes cost money on private repos; confirm the repo's billing situation before enabling the workflow.
4. **Intel Macs:** llama.cpp CPU works; Laya needs ONNX Runtime ≤ 1.23 (last Intel build) or is unavailable; Kev CPU (kev-0.6b) works. Recommended: support with those limits, clearly labelled.
5. **Hosting engine binaries:** GitHub Releases of this repo under an `engines-v*` tag (free, no new infra). Recommended.

## Mandatory final steps (every work-stream plan repeats these)

1. Plan review against the implementation.
2. Test-coverage review.
3. Bug hunt.
4. CI parity (`rustup run stable cargo clippy --workspace --all-targets -- -D warnings`, fmt, targeted tests) and commit only touched files.
