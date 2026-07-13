# LocalRouter - Project Guide

Local OpenAI-compatible API gateway with intelligent routing, multi-provider support, and MCP integration.

**Stack**: Rust/Tauri 2.x (backend), Axum (server), React/TypeScript (frontend)

## Critical Rules

### Tauri Conventions
- Backend: **snake_case** params (`client_id: String`)
- Frontend: **camelCase** in `invoke()` (`{ clientId: "..." }`)
- No native dialogs (`window.confirm`) - use Radix UI `AlertDialog`

### Adding/Modifying Tauri Commands

When creating or modifying a Tauri command, update **all locations**:

1. **Rust backend** (`src-tauri/src/ui/commands*.rs`)
   ```rust
   #[derive(Serialize)]
   pub struct MyResult { pub field: String }

   #[tauri::command]
   pub async fn my_command(my_param: String) -> Result<MyResult, String> { ... }
   ```

2. **TypeScript types** (`src/types/tauri-commands.ts`)
   ```typescript
   // Response type (top of file, with other response types)
   /** Rust: src-tauri/src/ui/commands.rs - MyResult struct */
   export interface MyResult { field: string }

   // Request params (bottom of file, in "Command Parameters" section)
   /** Params for my_command */
   export interface MyCommandParams { myParam: string }
   ```

3. **Demo mock** (`website/src/components/demo/TauriMockSetup.ts`)
   ```typescript
   'my_command': (args): MyResult => ({
     field: 'demo value',
   }),
   ```

4. **Usage in frontend**
   ```typescript
   import type { MyResult, MyCommandParams } from '@/types/tauri-commands'
   const result = await invoke<MyResult>('my_command', params satisfies MyCommandParams)
   ```

**Type sync checklist:**
- [ ] Response type added/updated (matches Rust return struct)
- [ ] Request params type added/updated (camelCase, matches Rust params)
- [ ] Optional fields: `Option<T>` in Rust → `T | null` in TypeScript
- [ ] Enums: `#[serde(rename_all = "snake_case")]` in Rust
- [ ] Mock handler returns data matching response type
- [ ] Run `npx tsc --noEmit` to verify types

### Pre-Commit CI Parity (CRITICAL)

CI has been failing repeatedly because local `clippy` lags the CI toolchain.
**Before every commit, run the exact checks CI runs using the rustup stable
toolchain (not the system/Homebrew `rustc`)**:

```bash
rustup update stable                                        # keep parity with CI
rustup run stable cargo clippy --workspace --all-targets -- -D warnings
rustup run stable cargo fmt --all -- --check
rustup run stable cargo test --workspace
```

Why this matters:
- CI uses `dtolnay/rust-toolchain@stable` which installs the latest stable at
  run time (e.g. 1.95), so new clippy lints (`collapsible_match`,
  `useless_conversion`, `sort_by_key`, …) fire there first.
- Homebrew `rustc` may lag stable by several minor versions — do not trust it
  for "clippy clean" verdicts.
- Silently shipping a commit that fails CI wastes ~20 min per retry (runner
  disk-free step + full workspace build). Always verify locally first.

If a commit must be pushed before CI completes, immediately tail the run
(`gh run list --limit 1; gh run view <id>`) and fix failures before
moving on to other work.

### Privacy Policy (CRITICAL)
- **No telemetry** - zero analytics or tracking
- **No external assets** - all bundled at build time
- **Local-only default** - localhost API, restrictive CSP
- Network requests only via: user actions, optional update checks
- **Violations are critical bugs**

---

## Project Structure

### Backend (~67k LOC, multi-crate workspace)

Backend logic lives in `crates/`; `src-tauri/src` is a thin Tauri shell
(`lib.rs` re-exports the crates, only `cli`, `launcher`, `ui`, `updater`
are local modules).

```
crates/
├── lr-server/      # Axum, OpenAI API, OpenAPI (routes/, middleware/, openapi/)
├── lr-providers/   # 19 providers, feature adapters, OAuth (features/, oauth/)
├── lr-mcp/         # MCP proxy (bridge/, gateway/, transport/)
├── lr-monitoring/  # 4-tier metrics, logging
├── lr-config/      # YAML config, validation, migration
├── lr-router/      # Rate limiting, routing engine
├── lr-clients/     # Unified client system
├── lr-catalog/     # Model catalog and pricing
├── lr-routellm/    # RouteLLM integration
└── ...             # skills, memory, guardrails, oauth, types, utils, more

src-tauri/src/
├── ui/             # Tauri commands, tray
├── launcher/       # App startup wiring
├── updater/        # App updates
└── cli.rs          # CLI entry
```

### Frontend (~28k LOC)
```
src/
├── views/          # dashboard, clients, resources, mcp-servers, settings, try-it-out
├── components/     # ui/, wizard/, connection-graph/, shared/
├── hooks/          # Custom React hooks
└── utils/          # Helpers
```

### Key Locations
- Config: `crates/lr-config/`
- API server: `crates/lr-server/`
- Providers: `crates/lr-providers/`
- MCP: `crates/lr-mcp/`
- OpenAPI: `crates/lr-server/src/openapi/`
- Tauri commands: `src-tauri/src/ui/`

---

## Commands

```bash
cargo tauri dev              # Dev mode (port 3625, ~/.localrouter-dev/)
cargo test && cargo clippy   # Test and lint
cargo build --release        # Production build

# Dev helper - avoid keychain prompts
export LOCALROUTER_KEYCHAIN=file  # WARNING: plain text secrets!
```

## API Endpoints (port 3625)

| Endpoint | Description |
|----------|-------------|
| `GET /v1/models` | List models |
| `POST /v1/chat/completions` | Chat (streaming) |
| `POST /v1/completions` | Completions |
| `POST /v1/embeddings` | Embeddings |
| `POST /v1/audio/transcriptions` | Speech-to-text (STT) |
| `POST /v1/audio/translations` | Speech-to-English translation |
| `POST /v1/audio/speech` | Text-to-speech (TTS) |
| `POST /mcp/*` | MCP proxy |
| `GET /openapi.json` | OpenAPI spec |
| `GET /health` | Health check |

---

## Documentation

Plans in `./plan/` directory (260+ dated documents — the newest files are the
current source of truth). Key files:
- `plan/2026-01-14-ARCHITECTURE.md` - System design
- `plan/2026-01-17-MCP_AUTH_REDESIGN.md` - Client architecture

Note: `plan/2026-01-14-PROGRESS.md` is historical (abandoned 2026-01-21) —
do not use it for feature tracking.

---

## Development Workflow

1. Check the newest dated files in `plan/` for current work and context
2. Read relevant architecture docs
3. Implement with tests
4. Run `cargo test && cargo clippy && cargo fmt`
5. Commit with Conventional Commits: `<type>(<scope>): <description>`
6. **Always commit your own changes at the end of a task** — only stage files you modified, never unrelated changes (do not push unless the user explicitly asks)

**Types**: feat, fix, docs, test, refactor, chore

### Plan Documentation (CRITICAL)

Every implementation plan **must** be saved to `./plan/` and every plan **must** include the mandatory final steps below.

#### First Steps (BEFORE any code)

The **very first part** of every plan must use the **todo list to keep track of progress**. Create tasks for each step of the plan so progress is visible and trackable throughout implementation.

Then, **save the plan** before writing any code using `copy-plan.sh`:
  ```bash
  ./copy-plan.sh <claude-plan-name> <SHORT_DESCRIPTION>
  ```
  Example: `./copy-plan.sh iridescent-stargazing-whale MONITOR_EVENT_REDESIGN`

#### Final Steps: Review, Test, Bug Hunt (AFTER implementation)

Every plan **must** include these mandatory final steps after all implementation is complete:

1. **Plan Review**: Review the plan against the implementation — identify any missed changes, behaviors, or edge cases that were specified in the plan but not yet implemented, and implement them
2. **Test Coverage Review**: Review code coverage for all new/modified code — add tests for any uncovered paths, edge cases, or error handling
3. **Bug Hunt**: Re-read the implementation code with fresh eyes specifically looking for bugs — off-by-one errors, race conditions, missing error handling, incorrect state transitions, etc.
4. **Commit**: Commit all changes — only stage files you modified, never unrelated changes (do not push unless the user explicitly asks)

### OpenAPI Requirements
When modifying endpoints:
1. Add `#[utoipa::path]` annotation
2. Register types in `server/openapi/mod.rs`
3. Add `ToSchema` derive to request/response types
4. Verify at http://localhost:3625/openapi.json

---

## Providers (19)

Anthropic, Cerebras, Cohere, DeepInfra, Gemini, Groq, LMStudio, Mistral, Ollama, OpenAI, OpenRouter, Perplexity, TogetherAI, xAI, plus OpenAI-compatible generic

**Feature adapters**: prompt_caching, json_mode, logprobs, structured_outputs

---

## Updating the Model Catalog

Where each provider's model list comes from, and how to refresh it when a
provider ships new models. Most providers self-update; only a few carry a
hardcoded list that can go stale.

### 1. models.dev snapshot (shared pricing/metadata) — `lr-catalog`

`crates/lr-catalog/catalog/modelsdev_raw.json` is an **auto-generated,
build-time** snapshot of the [models.dev](https://models.dev) catalog. It
backs pricing and metadata for every catalog-aware provider. Do **not**
hand-edit it. Refresh it (network happens at build time only — never at
runtime):

```bash
LOCALROUTER_REBUILD_CATALOG=1 cargo build -p lr-catalog   # re-fetches + rewrites the JSON
```

Commit the regenerated `modelsdev_raw.json` (and `catalog/.last_fetch`). The
build otherwise re-uses the cached snapshot for 7 days.
(`LOCALROUTER_SKIP_CATALOG_FETCH=1` forces cache-only / offline builds.)

### 2. Dynamic providers — nothing to do

OpenAI (public API), Anthropic, Gemini, Groq, Cerebras, Cohere, DeepInfra,
Mistral, OpenRouter, TogetherAI, xAI, and the local providers
(Ollama, LMStudio, gpt4all, …) fetch their model list live from the
upstream `GET /models` (or `/api/tags`) endpoint. They surface whatever the
provider returns, so **new models appear automatically** — no code change.
Any hardcoded `get_known_models()` in these files is only an offline/pricing
backstop, not the served list.

- **Anthropic** is dynamic but curates display names via a `match` in
  `get_model_info` (`crates/lr-providers/src/anthropic.rs`). Unknown ids are
  **not** dropped — `model_info_from_api` falls back to catalog/`display_name`
  defaults — so a new Claude model lists without a code change. Add a `match`
  arm only to give a known model nicer curated metadata.

### 3. ChatGPT Plus/Pro (Codex backend) — hardcoded fallback to sync

This provider's primary path is dynamic (`GET
chatgpt.com/backend-api/codex/models`, OAuth-gated). The **offline fallback**
list `chatgpt_plus_fallback_models()` in `crates/lr-providers/src/openai.rs`
must mirror codex-rs's authoritative
`codex-rs/models-manager/models.json` — specifically the entries marked
`visibility: "list"`. To refresh it, pull the upstream file and copy the
list-visible slugs / display names / context windows:

```bash
gh api repos/openai/codex/contents/codex-rs/models-manager/models.json \
  --jq '.content' | base64 -d | \
  python3 -c 'import json,sys; [print(m["slug"], m["context_window"], m["display_name"]) for m in json.load(sys.stdin)["models"] if m.get("visibility")=="list"]'
```

Use the **codex** context windows (they are the backend-specific caps — do
**not** pull these from models.dev, whose same-named public-API models
advertise a larger window). The test
`chatgpt_plus_fallback_list_covers_codex_visible_models` guards this list.

### 4. Perplexity — models.dev only

No public `/models` endpoint. The served list comes from the models.dev
snapshot (registered `CatalogOnly`), so refresh it via step 1. The in-code
`get_known_models()` is only a backstop.

---

## Environment

**Dev**: `~/.localrouter-dev/`
**Prod**:
- macOS: `~/Library/Application Support/LocalRouter/`
- Linux: `~/.localrouter/`
- Windows: `%APPDATA%\LocalRouter\`

---

## Windows XP Demo (Website)

The website includes a Windows XP demo that showcases LocalRouter. The source is in a separate repo.

### Source Location
- **Repo**: `/Users/matus/dev/winXP` (external, not part of this repo)
- **Key files**:
  - `src/WinXP/apps/LocalRouter/index.js` - LocalRouter app component
  - `src/WinXP/apps/index.js` - App registry and icons
  - `src/WinXP/Footer/TrayMenu.js` - System tray menu (synced with `src-tauri/src/ui/tray_menu.rs`)
  - `src/WinXP/Footer/index.js` - Taskbar with tray icon
  - `src/assets/windowsIcons/localrouter.svg` - LocalRouter icon

### Build & Deploy
After editing the winXP repo, always run the build script:

```bash
./scripts/build-winxp.sh
```

This script:
1. Builds winXP with `homepage: /winxp` for correct asset paths
2. Copies build to `website/public/winxp/`
3. Updates index.html with LocalRouter metadata

### Important Notes
- The tray menu should stay in sync with `src-tauri/src/ui/tray_menu.rs`
- Uses mock data for demo purposes (clients, strategies, MCP servers, skills)
- The LocalRouter app embeds `/demo` in an iframe

---

**Version**: 0.1.0 | **License**: AGPL-3.0-or-later | **Updated**: 2026-01-25
