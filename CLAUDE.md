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

### Privacy Policy (CRITICAL)
- **No telemetry** - zero analytics or tracking
- **No external assets** - all bundled at build time
- **Local-only default** - localhost API, restrictive CSP
- Network requests only via: user actions, optional update checks
- **Violations are critical bugs**

---

## Project Structure

### Backend (~67k LOC)
```
src-tauri/src/
├── server/         # Axum, OpenAI API, OpenAPI (routes/, middleware/, openapi/)
├── providers/      # 19 providers, feature adapters, OAuth (features/, oauth/)
├── mcp/            # MCP proxy (bridge/, gateway/, transport/)
├── monitoring/     # 4-tier metrics, logging
├── config/         # YAML config, validation, migration
├── router/         # Rate limiting, routing engine
├── clients/        # Unified client system
├── catalog/        # Model catalog
├── routellm/       # RouteLLM integration
├── updater/        # App updates
├── ui/             # Tauri commands
└── utils/          # Crypto, errors
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
- Config: `src-tauri/src/config/`
- API server: `src-tauri/src/server/`
- Providers: `src-tauri/src/providers/`
- MCP: `src-tauri/src/mcp/`
- OpenAPI: `src-tauri/src/server/openapi/`

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
| `POST /mcp/*` | MCP proxy |
| `GET /openapi.json` | OpenAPI spec |
| `GET /health` | Health check |

---

## Documentation

Plans in `./plan/` directory (114 documents). Key files:
- `plan/2026-01-14-ARCHITECTURE.md` - System design
- `plan/2026-01-14-PROGRESS.md` - Feature tracking
- `plan/2026-01-17-MCP_AUTH_REDESIGN.md` - Client architecture

---

## Development Workflow

1. Check `plan/2026-01-14-PROGRESS.md` for available features
2. Read relevant architecture docs
3. Implement with tests
4. Run `cargo test && cargo clippy && cargo fmt`
5. Commit with Conventional Commits: `<type>(<scope>): <description>`

**Types**: feat, fix, docs, test, refactor, chore

### Plan Documentation (CRITICAL)
Every implementation plan **must** be saved to `./plan/` at the **start** of implementation:
- **Filename**: `plan/YYYY-MM-DD-SHORT_DESCRIPTION.md` (use today's date)
- **Content**: The full plan including goals, approach, files to modify, and any architectural decisions
- **When**: Save the plan file **before** writing any code, immediately after the plan is approved
- This applies to all plans created via Claude Code's plan mode or any multi-step implementation task
- Update `plan/2026-01-14-PROGRESS.md` if the plan adds or completes a tracked feature

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
