# Phase 2 · Plan C: System One engines (Laya, Kev)

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md`. Depends on plan B (engine packs, supervisor, built-in provider) and plan A (bundle downloads).

## Goal

Two engine executables that serve `POST /v1/systemone` (TypeSafe wire format, as implemented in Phase 1) for locally downloaded Laya and Kev models, built by our own CI, published as engine packs, supervised like `llama-server`, and reached through the existing `SystemOneProvider`.

## Step 0: tracking
- Todo items per step; save this plan with `./copy-plan.sh`.

## Step 1: layout (kept out of the main workspace)
```
engines/
  common/          systemone-engine-common (lib): axum server, API-key auth, /health, /v1/models, /v1/systemone,
                   request validation (reuses the Phase 1 wire types by path dependency on lr-providers' systemone types
                   extracted into a tiny `lr-systemone-types` crate), graceful shutdown, stdin-EOF parent watch
  laya-engine/     own [workspace], Cargo.lock, depends on common + ort + tokenizers
  kev-engine/      own [workspace], Cargo.lock, rust-toolchain.toml (1.95), depends on common + kev-core (git rev)
```
- Separate workspaces keep `kev-core`'s exact `=` pins (tokenizers 0.23.2, candle 0.11, mlx-rs 0.32) and `ort`'s native downloads out of the app's dependency graph and out of main CI.
- Extract `SystemOneRequest/Response/Question/Answer` + `validate_systemone_request` from `lr-providers/src/systemone/types.rs` into `crates/lr-systemone-types` (no heavy deps); `lr-providers` re-exports it, so Phase 1 code is unchanged.
- CLI for both: `--host 127.0.0.1 --port P --api-key K --model-dir DIR [--device cpu|metal] [--threads N]`, plus `--version` and `--self-test` (loads the model, answers one fixture question; used by the installer check and by support).
- Parent watch: the engine exits when its stdin reaches EOF (the supervisor keeps a pipe open), which covers macOS where there is no parent-death signal.

## Step 2: Laya engine (`engines/laya-engine`)
- **Runtime:** `ort` 2.0.0-rc.13 (pin exact), CPU execution provider everywhere at first (CoreML/DirectML later, measured). `tokenizers` for `tokenizer/tokenizer.json`. Load with `commit_from_file` so `laya.onnx.data` (external weights) resolves.
- **Linking per platform:**
  - macOS arm64, Windows x64: static via `ort` `download-binaries` at build time (engines CI only).
  - Linux x64/arm64: `load-dynamic` with Microsoft's official `libonnxruntime.so` shipped inside the pack (pyke's prebuilts need glibc 2.39; the official builds support older distros).
  - macOS x64 (Intel): `load-dynamic` against official ONNX Runtime 1.23.x (the last Intel release), `api-23` feature level; otherwise Laya is unavailable on Intel Macs (overview decision 4).
- **Port of the reference encoder/decoder** (receptron `@receptron/laya` 0.1.2 and upstream `NandhaKishorM/laya` `common.py`):
  - Inputs `input_ids`, `attention_mask` (i64 [b, seq]), `marker_pos` (i64 [b, opts]), `marker_mask` (bool [b, opts]), `qtype` (i64 [b]: 0 choice, 1 score, 2 noul); outputs `logits` (f32 [b, opts], masked −1e4), `act_probs` (unused initially).
  - Sequence: `[CLS] tok("<type> question: <instructions>") [SEP] ([MASK] tok(" " + option)[:48])… [SEP] tok(state)[:room] [SEP]`, `add_special_tokens = false`, literal `[MASK]` in user text replaced by a space, `max_len` 512 and `head_max_len` 192 from `laya_config.json`, same truncation budget rules.
  - JSON state serialized exactly like Python `json.dumps(ensure_ascii=False)` (separators `", "` and `": "`, key order preserved; needs a custom `serde_json` formatter and `preserve_order`).
  - One row per question, right-padded with `[PAD]`; noul renders two options (false, true), so p(yes) = p[1].
  - Decode: `softmax(logits[:k] / T)` with T from `laya_config.json` by type and option-count bucket, **clamped to [0.5, 5.0]** (upstream fix for over-sharp `choice:11+`); choice → argmax + probabilities; score → expected level + legend; noul → p[1]; confidence as Phase 1 defines it.
  - Limits surfaced as 422 with a clear message: >255 options, options that cannot fit the 192-token head budget.
- **Models (curated bundles, pinned SHA-256 from the Hub tree):** `receptron/laya-onnx` fp32 (≈1.7 GB, ~2 GB RAM); `inferenceprince/laya-onnx-int8` (≈606 MB) as the smaller option. Both Apache-2.0. The multilingual Laya checkpoint has no ONNX export yet: listed as unavailable.
- **Tests:** parity goldens generated once from `laya-serve` / the receptron package on ~30 fixtures (all three types, long state, many options, JSON state with unicode) and committed as JSON; an env-gated test (`LOCALROUTER_E2E_LAYA=1`) runs the fp32 model and asserts |Δp| ≤ 1e-4 and identical argmax. Encoder unit tests (token layout, truncation, JSON serialization) run without weights using a small fixture tokenizer.

## Step 3: Kev engine (`engines/kev-engine`)
- **Runtime:** `kev-core` from `codesoda/kev-rs` pinned to a git rev (v0.1.1 or later), with its required `[patch.crates-io] mlx-sys` entry, `MLX_RS_METAL_JIT=1` and `MACOSX_DEPLOYMENT_TARGET=14.0` on Apple Silicon.
  - API: `Runtime::load(LoadOptions { model_dir, device: Metal | Cpu, temperature })`, `rt.evaluate(&SystemOneRequest)` → answers + probabilities + token counts; prefix cache enabled.
- **Platform matrix (as kev-rs supports today):**

  | Platform | Device | Checkpoints |
  |---|---|---|
  | macOS arm64 (macOS 14+) | MLX / Metal | kev-0.8b, kev-4b (~10 GB RAM) |
  | macOS x64, Windows x64, Linux x64/arm64 | Candle CPU | kev-0.6b only (Qwen3 dense) |

  Hybrid Qwen3.5 checkpoints on CPU are refused with a clear message. When kev-rs lands its llama.cpp path (its issue #1), revisit to add GPU/CPU coverage for 0.8b/4b everywhere.
- **Artifacts per checkpoint** (bundle manifest built from kev-rs `manifests/sources.json` SHA-256s): `base/` (Qwen base snapshot from the Qwen HF repo: Qwen3-0.6B-Base ≈1.2 GB, Qwen3.5-0.8B-Base ≈1.75 GB, Qwen3.5-4B-Base ≈9.3 GB), `adapter/` (PEFT LoRA from `jaredpalmer/kev-*`, 40-130 MB), `head.safetensors` + `head.meta.json`.
- **Head conversion without Python:** kev-rs expects a converted head produced by its Python `kev-convert-head`. Implement the conversion in the engine (`kev-engine convert-head <head.pt> <out_dir>`) using Candle's restricted PyTorch pickle reader (`candle_core::pickle`, which reads tensors without executing pickled code), writing the same safetensors layout and metadata; verify bit-exactness against kev-rs goldens for each supported checkpoint. The installer runs it once after download. Fallback if a checkpoint's `head.pt` needs unsupported pickle features: publish converted heads as release assets of this repo (pinned by SHA-256), not as a new service.
- **Tests:** kev-rs's frozen parity fixtures (22) run env-gated (`LOCALROUTER_E2E_KEV=1`) against our engine's HTTP surface: max |Δp| within kev-rs's published tolerances, zero argmax flips; head-conversion equality test on a small synthetic `.pt` built in-test; HTTP surface tests with a stub runtime.

## Step 4: packs, bundles and provider wiring
- `crates/lr-engines/engines.json` gains `laya` and `kev` entries per platform (URL = this repo's GitHub release asset under tag `engines-vX.Y.Z`, SHA-256, size, `min_os`).
- Curated bundles (`crates/lr-local-models/curated.json`, static, shipped in-app): Laya fp32, Laya int8, kev-0.6b, kev-0.8b, kev-4b, each with the files, SHA-256s, total size, required engine, supported platforms, license, and short description. The Discover UI shows them under "System One (decision) models"; unavailable ones show why.
- Library entries of kind `Decision` with `engine: Laya | Kev`. `LocalModelsProvider::supports_systemone_model` is true for them; requests go to an internal `SystemOneProvider` (flavor Laya/Kev) at the supervised port with the process's API key. `list_models` marks them `Capability::Decision`, so chat routing skips them and System One auto-routing includes them.
- Supervisor launch args per engine; readiness via `/health`; `--self-test` during install.

## Step 5: engines CI (`.github/workflows/engines.yml`)
- Triggers: tag `engines-v*` and `workflow_dispatch`. Never runs on PRs to the main app.
- Matrix: `laya` × {macos-latest arm64, an Intel macOS target (cross-compiled from arm64 with the x86_64 ORT 1.23 dylib), windows-latest x64, ubuntu-22.04 x64, ubuntu-22.04-arm}; `kev` × {macos-latest arm64 (MLX; Xcode 15+ with the Metal toolchain), macOS x64 cross, windows-latest x64, ubuntu-22.04 x64/arm64 (Candle CPU)}.
- Steps: build release, run `--version` and unit tests, codesign + notarize macOS binaries and bundled dylibs with the existing Developer ID secrets, archive, compute SHA-256, upload to the GitHub release, then open a PR updating `engines.json` via `scripts/update-engine-manifest.sh`.
- Cost: macOS runner minutes cost money on private repos (overview decision 3); keep the matrix minimal and dispatch manually.

## Mandatory final steps
1. Plan review; 2. test-coverage review (parity goldens present for both engines); 3. bug hunt (tokenization truncation edge cases, JSON serialization parity, temperature clamp, masked options, head conversion safety, API key checks on every route, shutdown on stdin EOF); 4. clippy/fmt for each engine workspace plus the main workspace, targeted tests, commit.
