# Phase 2 overview: Local Embedded providers (LocalRouter runs llama.cpp, Laya and Kev, models managed in-app)

## Context

Phase 1 (`plan/2026-09-24-SYSTEMONE_ENDPOINT.md`) added `/v1/systemone` and providers that talk to servers the user runs themselves. Phase 2 adds **Local Embedded providers**: LocalRouter starts and stops the inference engine itself and manages the models in-app (search and download from Hugging Face, load and unload on demand).

## User decisions (2026-09-24)

- **External installs only.** Engines are installed by the user through their OS package manager (Homebrew, WinGet, apt, pacman, uv…). LocalRouter never downloads or hosts engine binaries. It expects the engine on PATH.
- **Show the install commands.** Each provider shows OS-specific commands, then offers an **Install** button (runs the shown command, with visible output) and a **Refresh** button (re-detects).
- **Three separate providers:** llama.cpp, Laya, Kev.
- **New category, listed first** in Add Provider, above Local: **"Local Embedded Providers"**. Description mentions that LocalRouter runs the engine and models are managed directly in-app, including downloads from Hugging Face.
- **Remove the existing llama.cpp option** (the OpenAI-compatible wrapper around a user-run `llama-server` on localhost) from the Add Provider list. Existing configured instances keep working (config compatibility rule: never remove serde variants), they are just not offered for new setups.
- **Hugging Face sign-in via a Client ID Metadata Document (CIMD)** hosted on localrouter.ai (no manual app registration). Token paste stays as an alternative.
- Repo is public (workflows allowed, free); Intel Macs supported where the engines support them.

## Engines and how they are found and launched

| Provider (type id) | Engine on PATH | Launch (LocalRouter picks port and API key) | Notes |
|---|---|---|---|
| **llama.cpp** (`llamacpp_embedded`) | `llama-server`, or the unified `llama` binary (`llama serve …`, same flags) | `llama-server --host 127.0.0.1 --port P -m <gguf> --jinja --no-webui --offline [...]`, API key via `LLAMA_API_KEY` env (not argv) | one process per loaded model; `/health` 503 while loading, 200 ready; flags feature-detected from `--help` for old distro builds |
| **Laya** (`laya`) | `laya-serve` (from `uv tool install "laya[serve]"`) | env only: `LAYA_HOST=127.0.0.1` (default is 0.0.0.0!), `LAYA_PORT`, `LAYA_API_KEY`, `LAYA_MODELS`, `LAYA_DEVICE`, `LAYA_PRELOAD`, `HF_TOKEN` | one process; port stays closed until preloaded checkpoints are ready; `/health` lists loaded checkpoints; no `/v1/models` |
| **Kev** (`kev`) | `uv` (Kev has no PyPI package and no console script) | `uvx --python 3.13 --from "kev[serve] @ git+https://github.com/jaredpalmer/kev@<pinned sha>" python -m kev.serve --run jaredpalmer/kev-<size> --port P`, `KEV_API_KEY`, `HF_TOKEN` | one process per checkpoint; binds 127.0.0.1 itself; no `/health` (readiness = `GET /openapi.json`); no Intel Mac (torch range) |

Install commands shown per OS (defaults first):

- **llama.cpp**
  - macOS: `brew install llama.cpp`. Intel Macs build from source (slow, CPU only); `sudo port install llama.cpp` is the alternative.
  - Windows: `winget install --id ggml.llamacpp -e` (Vulkan build). Scoop alternative: `scoop bucket add versions` then `scoop install versions/llama.cpp-vulkan`.
  - Linux: `brew install llama.cpp` (Linuxbrew, any distro). Distro packages:
    - Ubuntu 26.04+ / Debian testing: `sudo apt install llama.cpp`
    - Arch: `sudo pacman -S llama-cpp ggml-vulkan`
    - Nix: `nix profile add nixpkgs#llama-cpp-vulkan`
    - Fedora's package is stale; not recommended.
- **uv** (needed by Laya and Kev)
  - macOS/Linux: `curl -LsSf https://astral.sh/uv/install.sh | sh` (Homebrew `brew install uv` fine on Apple Silicon).
  - Windows: `winget install --id=astral-sh.uv -e`.
- **Laya:** `uv tool install --python 3.12 --torch-backend auto "laya[serve]"` (all OSes; Intel Mac unsupported by current PyTorch, shown as such).
- **Kev:** installing uv is enough; the first start prepares Kev's environment (pinned commit). A **Prepare** button runs `uvx … python -c "import kev.serve"` so the multi-GB PyTorch download happens up front with visible output.

## Architecture

- `crates/lr-engines` (new): engine **recipes** (detection names, install commands per OS, version probe), **detection** (reuses `lr_utils::binary::find_binary`/`shell_path`, adds Linuxbrew, Nix, MacPorts and Windows WinGet/Scoop/`.local\bin` dirs, and on Windows re-reads PATH from the registry on every Refresh), **install runner** (runs the shown command in the user's shell, streams output lines as events, cancellable), and the **process supervisor** (ports, API keys, readiness, logs, crash restarts with backoff, idle unload, orphan cleanup, shutdown with the app).
- `crates/lr-local-models` (new): Hugging Face client, resumable verified downloader, GGUF header parser and classifier, hardware detection and fit estimate, model library, HF credentials (CIMD OAuth + token). Used by the llama.cpp provider (Laya and Kev download their own weights through their HF libraries; LocalRouter passes `HF_TOKEN` and shows checkpoint sizes).
- Three providers in `crates/lr-providers/src/direct/`, each delegating HTTP to existing code: llama.cpp → `LlamaCppProvider`; Laya and Kev → `SystemOneProvider` (flavors Laya/Kev).
- New `ProviderCategory::Embedded` (serde `direct`), ordered first. Factories get `fn listed(&self) -> bool` (default true); the legacy `llamacpp` factory returns false, so it is hidden from Add Provider but still loads existing configs. `list_provider_types` returns a stable order (category, then display name), which also fixes the Custom tab's random pick between the two `generic` factories.
- The Phase 1 `laya` and `kev` types (external server URL, unreleased on this branch) become the direct Laya and Kev providers. Remote or self-run System One servers remain reachable through `systemone_compatible`.

## Work streams

| Plan | Scope |
|---|---|
| A: Hub, downloads, library, sign-in | `lr-local-models` (unchanged scope except: sign-in uses CIMD, no Laya/Kev bundles) |
| B: engines crate, llama.cpp Local Embedded provider | `lr-engines`, `llamacpp_embedded`, category, hidden legacy llama.cpp, commands and events |
| C: Laya and Kev Local Embedded providers | recipes, launch/env, checkpoint management, readiness quirks |
| D: UI and website | Local Embedded category, per-provider Engine tab (commands, Install, Refresh, output), llama.cpp model browser/library, Laya/Kev checkpoint lists, HF account card, CIMD document on the website, docs, demo mocks |

## Privacy

Network only on user action: search, download, Install/Prepare clicks, starting a provider (which may download checkpoints; the UI says so first). Model cards shown as sanitized text without remote images. Engines bind 127.0.0.1 with per-launch API keys; Laya is forced off 0.0.0.0.

## Mandatory final steps (each plan)
Plan review; test-coverage review; bug hunt; CI parity (clippy `-D warnings`, fmt, targeted tests), commit only touched files.
