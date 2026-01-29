# LocalRouter - Project Guide

Local OpenAI-compatible API gateway with intelligent routing, multi-provider support, and MCP integration.

**Stack**: Rust/Tauri 2.x (backend), Axum (server), React/TypeScript (frontend)

## Critical Rules

### Tauri Conventions
- Backend: **snake_case** params (`client_id: String`)
- Frontend: **camelCase** in `invoke()` (`{ clientId: "..." }`)
- No native dialogs (`window.confirm`) - use Radix UI `AlertDialog`

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

Plans in `./plan/` directory (57 documents). Key files:
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

**Version**: 0.1.0 | **License**: AGPL-3.0-or-later | **Updated**: 2026-01-25
