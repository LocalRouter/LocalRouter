# Phase 2 · Plan B: engines crate, Direct category, llama.cpp direct provider

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md` (revised 2026-09-24: external installs only). Depends on plan A for the model library, downloads and fit estimates.

## Step 0: tracking
Todo items per step.

## Step 1: prerequisite fixes
- **Registry leak:** `HealthCheckManager` (`crates/lr-providers/src/health.rs`) never unregisters providers replaced or removed through `ProviderRegistry::update_provider`/`remove_provider`, and `check_health` matches on the type name. Key by instance name, add `unregister_provider`, call it from update/remove. A provider that owns child processes must be dropped when removed. Test with a Drop flag.
- **Stable type order:** `list_provider_types` iterates a `HashMap`. Sort by category order, then display name. This also fixes the Add Provider "Custom" tab, which picks the first `generic` factory (now `openai_compatible` or `systemone_compatible` at random).

## Step 2: Direct category and hidden legacy llama.cpp
- `ProviderCategory::Direct` (serde `direct`) in `crates/lr-providers/src/factory.rs`; category order `direct, local, subscription, first_party, third_party, generic`.
- `ProviderFactory::listed(&self) -> bool { true }`; `ProviderTypeInfo.listed`; the Add Provider UI filters unlisted types. `LlamaCppProviderFactory::listed() == false` (existing `llamacpp` instances keep loading; display name becomes "llama.cpp server (legacy)").
- TS: `ProviderCategory` union gains `'direct'`; `ProviderTypeInfo.listed`.

## Step 3: `crates/lr-engines`
- **Recipes** (`recipes.rs`): `EngineRecipe { id: "llamacpp" | "laya" | "kev" | "uv", display_name, binaries (preference order), requires (Laya and Kev need uv), install: Vec<InstallOption { os, label, command, default, needs_sudo, notes }>, docs_url }`. Commands per the overview; Linux options ordered by distro family from `/etc/os-release` (`ID`, `ID_LIKE`), with Linuxbrew as the universal option.
- **Detection** (`detect.rs`): `detect(recipe) -> EngineStatus { found: Option<PathBuf>, binary_name, version, requirements, issues }`.
  - Uses `lr_utils::binary::find_binary` (login-shell PATH, Flatpak host probe). Adds fallback dirs for everyone: `/home/linuxbrew/.linuxbrew/bin`, `~/.nix-profile/bin`, `/nix/var/nix/profiles/default/bin`, `/etc/profiles/per-user/$USER/bin`, `/run/current-system/sw/bin`, `/opt/local/bin`; Windows `%LOCALAPPDATA%\Microsoft\WinGet\Links`, `%LOCALAPPDATA%\Microsoft\WinGet\Packages\ggml.llamacpp_*`, `%LOCALAPPDATA%\Microsoft\WindowsApps`, `%USERPROFILE%\.local\bin`, `%USERPROFILE%\scoop\shims`.
  - Windows: Refresh re-reads `HKLM\...\Session Manager\Environment\Path` and `HKCU\Environment\Path` (expanded) so a binary installed by `winget` is found without restarting the app. `lr-utils` makes the cached shell PATH resettable (`refresh_shell_env()`).
  - Version probe: `llama-server --version` (stderr; regex `version: (\S+) \((?:build (\d+), commit (\w+)|(\w+))\)`), `uv --version`; 5 s timeout, `env_clear` plus shell env.
  - llama.cpp capability probe: parse `--help` once per binary+version for `--fit`, `-fa auto`, `--no-webui`, `--offline`, `--jinja`, `-ngl auto` (old distro builds lack some).
- **Install runner** (`install.rs`): runs one recipe option's command through the user's shell (`$SHELL -lc` via `sandbox::host_invocation` on Unix; `powershell -NoProfile -Command` on Windows) with the shell env. Streams output lines as `engine-install-output` `{run_id, line, stream}` and ends with `engine-install-finished` `{run_id, exit_code}`; cancellable (kills the process group). Options with `needs_sudo` (apt, pacman, port) are copy-only: the app has no terminal for a password prompt. The frontend passes `(recipe_id, option_index)`, never a command string, so only compiled recipe commands can run.
- **Supervisor** (`supervisor.rs`): `start(LaunchSpec) -> RunningEngine { port, api_key, pid }`, `LaunchSpec { program, args, env, readiness: Http{path} | TcpThenHttp{path}, start_timeout, log_name }`.
  - Free port by binding `127.0.0.1:0`; per-launch 32-byte hex API key passed through env (never argv, never logged); env = shell env + spec env; stdout/stderr to `logs/engines/<name>.log` (rotating 10 MB) and a 500-line ring buffer.
  - Readiness: llama.cpp `/health` 200 (503 while loading); Laya TCP then `/health` (port opens after preload); Kev TCP then `/openapi.json`.
  - Crash restart with backoff (1, 2, 4 s; at most 3 in 5 minutes), then Failed with the log tail. Orphans: `run/engines.json` (pid, start time, exe path) cleaned on startup only when pid and exe match; Linux `PR_SET_PDEATHSIG`, Windows Job Object kill-on-close. Shutdown with the app (GUI exit and CLI server shutdown): terminate, kill after 5 s.
  - Idle stop per process (`idle_unload_secs`, default 900, 0 = never), in-flight guards, de-duplicated starts, memory guardrail using plan A's fit estimate for llama.cpp models.
- **Tests:** fake engine binary (`[[bin]] lr-fake-engine`, axum; configurable readiness style, streaming chat, embeddings, crash, hang) via `CARGO_BIN_EXE_lr-fake-engine`: start, each readiness style, crash/restart/fail, idle stop, orphan cleanup, shutdown, de-dup. Detection with a temp dir on PATH; version regex; install runner with harmless test recipes.

## Step 4: llama.cpp direct provider (`llamacpp_direct`)
- Factory: category Direct, `AlwaysFreeLocal`, no catalog id, optional `binary_path` override. One process per loaded model.
- Models: plan A library entries for llama.cpp. `list_models` maps kind to capabilities (Chat, Completion, Vision, FunctionCalling, Embedding).
- Requests: `ensure_running(model)` → internal `LlamaCppProvider` bound to that port and key → delegate. Streams hold an in-flight guard; a dropped client stream closes the upstream HTTP stream so llama-server stops generating.
- Launch args (vector, no shell): `--host 127.0.0.1 --port P -m <path> --jinja --no-webui --offline` plus settings (`-c`, `-ngl`/`--fit`, `-fa`, `-ctk/-ctv` with quantized V requiring flash attention, `-np`, `-b/-ub`, `-t`, `--mmproj`, `--embeddings --pooling <type>`, `--rerank`); `LLAMA_API_KEY` env. Flags the detected build lacks are dropped per the capability probe. The unified `llama` binary is launched as `llama serve …`.
- `supports_feature("logprobs") == true`; `health_check` never spawns (Healthy when the engine is found and a model is installed).
- `supports_pull`/`pull_model("hf.co/{repo}:{quant}")` via plan A.
- Router: `ModelProvider::supports_systemone_model(model)` (default `supports_systemone()`), used by `crates/lr-router/src/systemone.rs`.

## Step 5: wiring, config, commands
- `ProviderType::LlamaCppDirect` (`llamacpp_direct`); main.rs registrations and the exhaustive match; `provider_type_str_to_enum`; `is_local_provider` gains `llamacpp_direct`.
- Config: `AppConfig.local_models` (plan A) gains `engines: EngineSettings { idle_unload_secs, max_loaded_models, start_timeout_secs }`.
- Tauri commands (`src-tauri/src/ui/commands_engines.rs`, mirrored in `src/types/tauri-commands.ts` and the demo mocks): `engine_recipes`, `engine_detect(recipe_id)`, `engine_install(recipe_id, option_index)` → `run_id`, `engine_install_cancel(run_id)`, `engine_processes`, `engine_logs(name)`, `engine_stop(name)`; plan A's model commands.
- Events: `engine-install-output`, `engine-install-finished`, `local-models-model-state`, plus plan A's download events.

## Mandatory final steps
Plan review; test-coverage review; bug hunt (orphans, port races, API keys never logged or shown, no arbitrary command execution from the frontend, Windows PATH refresh, Flatpak host invocation); clippy/fmt/tests; commit.
