# stable-diffusion.cpp Local Embedded image provider

## Context

LocalRouter's Local Embedded providers run engines the user installs (llama.cpp, Laya, Kev, Von, Decider). Image generation had no local engine: a Qwen-Image GGUF downloaded through the llama.cpp Models tab cannot run in llama.cpp (no tokenizer; now classified `unsupported`). Research on 2026-09-25 compared Mochi Diffusion, Draw Things, Rapid-MLX, vMLX, mlx-openai-server, mflux, LocalAI, Lemonade, KoboldCpp, ComfyUI, SD.Next, InvokeAI, Forge and Ollama. stable-diffusion.cpp (`sd-server`, leejet, MIT) is the engine with the widest, freshest model coverage (Qwen-Image 2.1 day-0, FLUX.2, Z-Image, SDXL, SD3.5, Wan, LTX), native GPU backends (Metal, CUDA, Vulkan, ROCm, CPU) on macOS, Windows and Linux, and an OpenAI-compatible `POST /v1/images/generations` (+ `/v1/images/edits`, `/v1/models`).

User decision (2026-09-25): stable-diffusion.cpp for all platforms. In the UI the user either selects the `sd-server` binary, or clicks Install and LocalRouter downloads the latest GitHub release for this platform. (sd.cpp is not in Homebrew or winget; this is the one engine LocalRouter downloads itself, only on that click.)

## Facts the design relies on

- `sd-server --listen-ip 127.0.0.1 --listen-port N --diffusion-model F --vae F --llm F [--cfg-scale X --steps N --sampling-method euler --diffusion-fa --offload-to-cpu]`. One model per process. No API key support: bind to loopback, LocalRouter's own auth protects it. Generation defaults come from launch flags; per request `sd_cpp_extra_args` JSON can be embedded in the prompt.
- `POST /v1/images/generations` accepts `prompt`, `n`, `size` (`WxH`), `output_format` (`png|jpeg|webp`), `output_compression`; returns `{created, output_format, data:[{b64_json}]}` (no URLs). `GET /v1/models` returns `sd-cpp-local`.
- Releases: rolling tags (`master-NNN-sha`), assets per platform: `sd-master-*-bin-Darwin-macOS-*-arm64.zip`, `…-bin-Linux-Ubuntu-*-x86_64.zip` (CPU), `…-x86_64-vulkan.zip`, `…-x86_64-rocm-*.zip`, `…-bin-win-cpu-x64.zip`, `…-bin-win-vulkan-x64.zip`, `…-bin-win-cuda12-x64.zip` (+ `cudart-sd-bin-win-cu12-x64.zip`), `…-bin-win-rocm-*-x64.zip`. Latest via `GET https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest`.
- Modern image models are bundles: diffusion weights + VAE + text encoder. Starter bundles (all ungated on Hugging Face):
  - **Z-Image Turbo** (~6.7 GB): `leejet/Z-Image-Turbo-GGUF` `z_image_turbo-Q4_K.gguf`; VAE `Comfy-Org/z_image_turbo` `split_files/vae/ae.safetensors`; LLM `unsloth/Qwen3-4B-Instruct-2507-GGUF` `Qwen3-4B-Instruct-2507-Q4_K_M.gguf`; `--cfg-scale 1.0 --steps 8`.
  - **FLUX.2 Klein 4B** (~5.3 GB): `leejet/FLUX.2-klein-4B-GGUF` `flux-2-klein-4b-Q4_0.gguf`; VAE `Comfy-Org/flux2-dev` `split_files/vae/flux2-vae.safetensors`; LLM `unsloth/Qwen3-4B-GGUF` `Qwen3-4B-Q4_K_M.gguf`; `--cfg-scale 1.0 --steps 4 --sampling-method euler`.
  - **Qwen-Image 2.1** (~11 GB): `leejet/Qwen-Image-2.1-GGUF` `qwen_image_2.1-Q4_K.gguf`; VAE `Comfy-Org/Qwen-Image-2.1` `vae/qwen_image_2.1_vae_bf16.safetensors`; LLM `Qwen/Qwen3-VL-8B-Instruct-GGUF` `Qwen3VL-8B-Instruct-Q4_K_M.gguf` plus its vision projector `mmproj-Qwen3VL-8B-Instruct-F16.gguf` (`--llm_vision`, required for image edits with a GGUF text encoder); `--cfg-scale 6.0 --sampling-method euler`.
- A library GGUF whose header says `general.architecture = qwen_image21` (e.g. the Qwen-Image-2.1-Uncensored file) can replace the Qwen-Image 2.1 bundle's diffusion file; it needs that bundle's VAE and text encoder.

## Design

### 1. Engine: detection and managed install (`lr-engines`)
- `RecipeId::SdCpp` (`sdcpp`), display "stable-diffusion.cpp", binary `sd-server`.
- Detection order: the provider's `binary_path` setting → LocalRouter's managed install (`{config_dir}/engines/managed/sdcpp/current`, see below) → PATH.
- Install options are **downloads**, not shell commands: one per build for this platform (macOS arm64: Metal; Linux x64: Vulkan (recommended), CPU, ROCm; Windows x64: Vulkan (recommended), CUDA 12, CPU, ROCm). `InstallOption` gains `kind: command | download`; download options carry the asset selector and show "Downloads the latest stable-diffusion.cpp release (Vulkan build) from github.com/leejet/stable-diffusion.cpp" instead of a command.
- `InstallRunner` runs a download option in-process: fetch the latest release JSON, pick the asset, stream it to a temp file with progress lines (`engine-install-output`), extract (zip) into `managed/sdcpp/<tag>-<build>/`, mark executables (`chmod +x`), for CUDA also extract the cudart zip into the same folder, then atomically point `current` (a small `install.json` with the binary path) at it and remove older installs. Cancellable. Network only on this click.
- Engine tab: "Choose sd-server…" (file dialog) saves the provider's `binary_path`; download options show a Download button (no Copy); status shows the managed install's release tag.

### 2. Image model bundles (`lr-local-models::image_models`)
- Static catalog of bundles: id, name, description, size, files `[{role: diffusion|vae|llm|t5xxl|clip_l|clip_g, repo, path, size}]`, `server_args`, `architectures` (for derived bundles).
- `ImageModelStore` persisted to `{models}/image_models.json`: installed bundles (`role → local path`), pending download jobs (`job id → bundle id`), derived bundles (library entry id + base bundle).
- Download = one `DownloadManager` job per repo of the bundle (files already present are skipped). The completion hook routes a job to the store when its id is pending for a bundle; otherwise it goes to the llama.cpp library as today. Status: downloaded (all role files present on disk), downloading (+ progress from jobs), failed (job error).
- Derived bundles: each library entry of kind `unsupported` whose GGUF architecture matches a bundle's `architectures` becomes a catalog entry "<entry name> (Qwen-Image 2.1 pipeline)"; downloading it fetches only the base bundle's non-diffusion files.

### 3. Provider (`lr-providers::embedded::sdcpp`, type `sdcpp_embedded`)
- Category Local Embedded, list priority after llama.cpp. Settings: `offload_to_cpu` (default on), `flash_attention` (default on), `idle_unload_minutes` (15), `binary_path`.
- The app injects an `ImageModelBackend` (catalog, start/cancel download, installed files) like the HF token source.
- `list_models` = installed bundles with `Capability::ImageGeneration` (new capability; `EndpointType::ImageGeneration` now matches it).
- `generate_image`: ensure the bundle's engine (one image model loaded at a time; the previous one is stopped), `POST /v1/images/generations` with `prompt`, `n`, `size`, `output_format: png`, long timeout; `b64_json` passes through, `response_format: url` returns a `data:` URL.
- `EmbeddedControl`: catalog/download/cancel/load/unload/model states; health via `engine_health`.

### 4. Wiring and UI
- `ProviderType::SdCppEmbedded` (`sdcpp_embedded`), main.rs registration and mapping, `provider_type_str_to_enum`, local-provider pricing list, ServiceIcon, `EMBEDDED_PROVIDER_RECIPES`.
- Models tab reuses `EngineModelsTab`; `EmbeddedCatalogModel` gains `progress` (0–1) for a progress bar.
- `Capability::as_str()` used by `/v1/models` and the UI model list (`image_generation`).
- Website demo mocks and docs (Local Embedded Providers: stable-diffusion.cpp).

## Mandatory final steps
1. Plan review against the implementation.
2. Test coverage review: asset selection per platform/build, extract + current pointer, detection order, bundle catalog integrity, store routing and persistence, derived bundles, launch args, response mapping.
3. Bug hunt: partial downloads/extractions, cancellation, Windows exe names, dylib/DLL placement, one-model-at-a-time eviction, path handling with spaces.
4. fmt, clippy (stable), targeted tests, tsc (app + website), commit.

## Addendum (2026-09-26): image edits
- `POST /v1/images/edits` (and `/images/edits`): multipart `model`, `prompt`, `image[]`/`image` (1–16, ≤20 MB each), optional `mask`, `n`, `size` (`auto` or `WxH` 64–4096), `response_format`, `user`. Same client/access/strategy checks as generation (added to generation too). 50 MB route body limit with axum's multipart `DefaultBodyLimit` raised (the audio upload routes had the same 2 MB multipart cap; raised to 25 MB).
- `ModelProvider::edit_image` / `supports_image_edits`; feature-support row "Image Edits". stable-diffusion.cpp forwards the images, mask and prompt as multipart to `sd-server` `/v1/images/edits` (`image[]` fields; no size = first image's size).
- Try It Out Images: Generate / Edit modes, image upload (multiple, drag and drop), optional mask, number of images, "Same as input" size, and an Edit button on every result.
