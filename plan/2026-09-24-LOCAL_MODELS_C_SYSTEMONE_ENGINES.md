# Phase 2 · Plan C: Laya and Kev Local Embedded providers

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md` (revised 2026-09-24: external installs only; the earlier custom Rust engines are dropped). Depends on plan B's `lr-engines` (recipes, detection, install runner, supervisor).

## Step 0: tracking
Todo items per step.

## Step 1: convert the Phase 1 `laya` and `kev` types
The Phase 1 `laya`/`kev` types (connect to a user-run server URL) are unreleased on this branch; they become Local Embedded providers under the same type ids, category Local Embedded. `systemone_compatible` remains for remote or self-run System One servers. Update factory tests, website mocks and docs that describe connecting to `laya-serve`/`kev.serve` by URL.

## Step 2: Laya Local Embedded provider (`laya`)
- **Engine:** `laya-serve` on PATH, installed with `uv tool install --python 3.12 --torch-backend auto "laya[serve]"` after installing uv. Official `laya` package only. The unofficial PyPI `laya-serve` package also installs a `laya-serve` command; it is detected (it answers `/healthz`, not `/health`, and ignores `LAYA_*`) and reported with instructions to uninstall it.
- **Setup parameters:** `binary_path` (optional), `device` (Auto | cpu | cuda | mps), `threads` (optional).
- **Checkpoints (managed in-app):** english (843 MB), multilingual (≈678 MB), typed-decisions (843 MB) from `convaiinnovations/laya` (Apache-2.0, not gated). Each can be enabled or disabled; enabled ones go into `LAYA_MODELS` with `LAYA_PRELOAD=1`. Laya downloads them into the Hugging Face cache on first start; the UI shows sizes and says so first. "Download now" starts the engine. State from `/health` (`loaded` list).
- **Launch:** env only: `LAYA_HOST=127.0.0.1` (its default is 0.0.0.0, always overridden), `LAYA_PORT`, `LAYA_API_KEY`, `LAYA_MODELS`, `LAYA_PRELOAD=1`, `LAYA_DEVICE`, `LAYA_THREADS`, `HF_TOKEN` when signed in (plan A). The port opens only after preloading, so readiness is TCP then `/health` 200, with a long start timeout; the UI shows log lines meanwhile.
- **Serving:** one process; `list_models` returns enabled checkpoints (`Capability::Decision`); `systemone()` delegates to an internal `SystemOneProvider` (flavor Laya) at the process port and key, starting it if needed. `supports_chat() == false`, `supports_systemone() == true`.
- **Platforms:** macOS arm64, Windows, Linux. Intel Macs shown as unsupported (no current PyTorch builds).

## Step 3: Kev Local Embedded provider (`kev`)
- **Engine:** `uv` on PATH. Kev has no PyPI package (the PyPI name `kev` is an unrelated project) and no console script, so LocalRouter runs `uvx --python 3.13 --from "kev[serve] @ git+https://github.com/jaredpalmer/kev@<pinned sha>" python -m kev.serve --run <checkpoint> --port P`. The commit is pinned in the recipe and bumped by PR. **Prepare** runs `uvx … python -c "import kev.serve"` (with `--torch-backend auto`) so the PyTorch/MLX download happens once with visible output.
- **Checkpoints (managed in-app):** `jaredpalmer/kev-0.8b` (≈1.8 GB total; any Apple Silicon Mac), `kev-4b` (≈9.5 GB; 32 GB Mac or ~9-14 GB VRAM), `kev-9b` (≈19.5 GB; ~17 GB VRAM). Larger and legacy checkpoints are not offered. Kev downloads adapter and Qwen base itself (HF cache, `HF_TOKEN` passed). Plan A's hardware detection marks checkpoints that will not fit.
- **Launch:** one process per enabled checkpoint; env `KEV_API_KEY` (always set; Kev's CORS is `*`), `HF_TOKEN`, optional `KEV_DTYPE`, `KEV_BACKEND`. Kev binds 127.0.0.1 itself (no `--host`). Readiness: TCP, then `GET /openapi.json` (no `/health`).
- **Serving:** `list_models` returns `kev-0.8b`, `kev-4b`, `kev-9b` as enabled (`Capability::Decision`); `systemone()` routes to that checkpoint's process via `SystemOneProvider` (flavor Kev).
- **Platforms:** Apple Silicon (MLX), Linux and Windows (PyTorch; CPU unless a CUDA build is chosen by `--torch-backend auto`). Intel Macs unsupported (Kev's PyTorch range).

## Step 4: tests
- Launch specs: exact env/args per setting (Laya always sets `LAYA_HOST=127.0.0.1`; Kev always sets `KEV_API_KEY`; `HF_TOKEN` only when signed in; API keys never in args).
- Readiness styles against the fake engine (late-opening port for Laya, `/openapi.json` for Kev).
- Provider delegation: requests reach the right process; changing enabled Laya checkpoints restarts with the new `LAYA_MODELS`; Kev runs one process per checkpoint and stops idle ones.
- Env-gated real-engine tests (`LOCALROUTER_E2E_LAYA=1`, `LOCALROUTER_E2E_KEV=1`).

## Mandatory final steps
Plan review; test-coverage review; bug hunt (0.0.0.0 never possible, key always set, long first-start timeouts, uv cache pruning re-prepare path, unofficial laya-serve detection); clippy/fmt/tests; commit.
