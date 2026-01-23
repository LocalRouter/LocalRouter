# LocalRouter AI - Project Guide

LocalRouter AI is a cross-platform desktop application (Rust + Tauri) providing a local OpenAI-compatible API gateway with intelligent routing, API key management, and multi-provider support.

**Tech Stack**: Rust (backend), Tauri 2.x (desktop), Axum (web server), React (frontend)

## Tauri Conventions (IMPORTANT)

**Parameter naming between frontend and backend:**
- Rust backend uses **snake_case** for parameter names (e.g., `client_id: String`)
- React frontend must use **camelCase** when calling `invoke()` (e.g., `{ clientId: "..." }`)
- Tauri automatically converts camelCase → snake_case via serde

**Example:**
```rust
// Rust backend
#[tauri::command]
pub async fn delete_client(client_id: String) -> Result<(), String>
```
```typescript
// React frontend - use camelCase!
await invoke("delete_client", { clientId: client.client_id })
```

**Also note:** Native browser dialogs (`window.confirm()`, `window.alert()`) do not work in Tauri's WebView. Use Radix UI's `AlertDialog` component instead.

## Privacy & Network Policy (CRITICAL)

**LocalRouter AI is a privacy-focused, local-first application.**

### Rules

1. **User-Initiated & Update Checks Only**:
   - External requests ONLY through user actions (adding providers, configuring MCP, making API requests)
   - Automated update checks (weekly, configurable, can be disabled)
   - No other automatic network requests
2. **No Telemetry**: No analytics, crash reporting, or usage tracking
3. **No External Assets**: No CDN usage - all assets bundled at build time
4. **Local-Only By Default**: API server localhost-only, restrictive CSP in `tauri.conf.json`

**Update Checking:**
- Default: Check for updates weekly (configurable)
- Users can disable in Preferences → Updates
- Only checks version number and release notes
- No user data transmitted
- No usage analytics or tracking

**Violations are critical bugs.**

---

## Documentation Organization

All plans stored in **project-local** `./plan/` directory (NOT `~/.claude/plans/`).

**Naming**: `plan/YYYY-MM-DD-<DOCUMENT_NAME>.md`

### Key Documents

- **plan/2026-01-14-ARCHITECTURE.md** - System design, component specs, interfaces
- **plan/2026-01-14-PROGRESS.md** - 150+ features with status tracking (⬜/🟨/✅/⚠️)
- **plan/2026-01-14-CONTRIBUTING.md** - Dev setup, code style, commit conventions
- **README.md** - Project intro and quick start
- **Recent Plans (2026-01-17)**:
  - MCP_AUTH_REDESIGN.md - Unified client architecture
  - OPENAPI_SPEC.md - API documentation (utoipa)
  - TAURI_UI_REFACTORING.md - UI simplification
  - MCP_TESTING_STRATEGY.md - Test strategy
  - FEATURE_ADAPTERS.md - Feature adapter docs

---

## Development Workflow

### Starting a Feature

1. **Choose**: Check `plan/2026-01-14-PROGRESS.md` for ⬜ Not Started features
2. **Understand**: Read relevant section in `plan/2026-01-14-ARCHITECTURE.md`
3. **Update**: Mark feature as 🟨 In Progress
4. **Implement**: Follow architecture, write tests
5. **Test**: `cargo test && cargo clippy && cargo fmt`
6. **Complete**: Mark ✅, add notes, commit

### Commit Format (Conventional Commits)

```
<type>(<scope>): <description>

[optional body]
```

**Types**: feat, fix, docs, test, refactor, chore

---

## Project Structure

### Backend (Rust) - ~37k LOC

```
src-tauri/src/
├── server/         # Axum web server, OpenAI API, OpenAPI docs (~8k lines)
│   ├── routes/     # chat, completions, embeddings, models, mcp, oauth
│   ├── middleware/ # oauth_auth, client_auth
│   └── openapi/    # utoipa-based schema
├── providers/      # 10+ providers, feature adapters, OAuth (~10k lines)
│   ├── features/   # prompt_caching, json_mode, logprobs, structured_outputs
│   └── oauth/      # anthropic_claude, github_copilot, openai_codex
├── mcp/            # MCP proxy, transports (STDIO, SSE) (~4k lines)
├── monitoring/     # 4-tier metrics, logging (~3k lines)
├── config/         # YAML config, validation, migration (~2.5k lines)
├── router/         # Rate limiting, routing engine (~1.5k lines)
├── api_keys/       # Legacy API key management (~2k lines)
├── clients/        # Unified client system (new)
├── ui/             # Tauri commands (~1.5k lines)
└── utils/          # Crypto, errors (~500 lines)
```

### Frontend (React) - ~3k LOC

```
src/
├── App.tsx
├── components/
│   ├── tabs/       # ApiKeysTab, ProvidersTab, ModelsTab, McpServersTab
│   ├── apikeys/    # ApiKeyDetailPage
│   ├── providers/  # ProviderDetailPage
│   ├── models/     # ModelDetailPage
│   └── charts/     # MetricsChart (Chart.js)
```

### Finding Things

- Configuration → `src-tauri/src/config/`
- Web server & API → `src-tauri/src/server/`
- Providers → `src-tauri/src/providers/`
- MCP → `src-tauri/src/mcp/`
- Metrics → `src-tauri/src/monitoring/`
- OpenAPI → `src-tauri/src/server/openapi/`

---

## Quick Reference

### Common Commands

```bash
cargo tauri dev              # Dev mode (port 3625, uses ~/.localrouter-dev/)
cargo test --lib             # Unit tests
cargo test                   # All tests
cargo clippy && cargo fmt    # Lint and format
cargo build --release        # Production build

# Dev helpers
export LOCALROUTER_KEYCHAIN=file  # File-based secrets (⚠️ plain text!)
```

### API Endpoints (port 3625)

- `GET /v1/models` - List models
- `POST /v1/chat/completions` - Chat (streaming supported)
- `POST /v1/completions` - Text completions
- `POST /v1/embeddings` - Embeddings
- `POST /mcp/*` - MCP proxy
- `POST /oauth/token` - OAuth tokens
- `GET /openapi.json` - OpenAPI spec
- `GET /health` - Health check

### Status Files

- `plan/2026-01-14-PROGRESS.md` - Feature tracking
- `plan/2026-01-14-ARCHITECTURE.md` - System design
- `src-tauri/src/config/mod.rs` - AppConfig
- `src-tauri/src/server/openapi/mod.rs` - OpenAPI schemas

---

## Current Status

**Progress**: ~65% complete - Core functional, refinement needed

**Working**:
- ✅ OpenAI-compatible API (chat, completions, embeddings, models)
- ✅ 10+ providers (Ollama, OpenAI, Anthropic, Gemini, etc.)
- ✅ Feature adapters (prompt caching, JSON mode, logprobs, structured outputs)
- ✅ MCP support (STDIO, SSE transports with OAuth)
- ✅ 4-tier metrics system with Chart.js dashboards
- ✅ Rate limiting, OAuth, OpenAPI docs
- ✅ Full Tauri UI with system tray

**Tests**: 367 total (353 passing, 6 failing, 8 ignored) - 96% pass rate

**Next Priorities**:
1. Fix 6 MCP test failures
2. Complete unified Client architecture
3. Finish routing engine (strategies, fallback)
4. UI refactoring (unified Clients tab)

**Known Issues**:
- MCP tests failing (see `plan/2026-01-17-TEST_BUG_ANALYSIS.md`)
- Client architecture partially implemented
- Routing engine incomplete (no cost optimization)

---

## Phase Overview

1. **Core Infrastructure** ✅ - Config, errors, crypto, storage
2. **Model Providers** ✅ - 10+ providers, features, OAuth
3. **Smart Router** ⚠️ 20% - Rate limiting done, strategies incomplete
4. **Web Server & API** ✅ - Axum server, all endpoints, OpenAPI
5. **API Key Management** ✅ - Encrypted storage, transitioning to Clients
6. **Monitoring** ✅ - 4-tier metrics, logging, graphs
7. **Desktop UI** ✅ - Tauri app, all tabs, system tray
8. **Polish & Testing** ⚠️ 50% - Tests mostly done, 6 failing, optimization needed

---

## Development Environment

### Dev vs Production Directories

**Development** (`cargo tauri dev`):
- Config: `~/.localrouter-dev/`
- Secrets: `~/.localrouter-dev/secrets.json` (if file-based)

**Production** (`cargo tauri build`):
- macOS: `~/Library/Application Support/LocalRouter/`
- Linux: `~/.localrouter/`
- Windows: `%APPDATA%\LocalRouter\`

### File-Based Keychain (Dev Only)

```bash
export LOCALROUTER_KEYCHAIN=file  # Avoid macOS keychain prompts
```

**⚠️ WARNING**: Stores secrets in **plain text**. Only for development with test keys.

---

## OpenAPI Requirements

When adding/modifying endpoints:

1. **Annotate handler** with `#[utoipa::path]`
2. **Register types** in `src-tauri/src/server/openapi/mod.rs`
3. **Add `ToSchema` derive** to all request/response types
4. **Verify**: `cargo check` → http://localhost:3625/openapi.json

**Best practices**:
- Keep schemas in sync with types
- Add descriptions and examples
- Document all error responses
- Test in Documentation tab before committing

---

## Git Commit Requirements (from ~/.claude/CLAUDE.md)

- Always configure git with user identity: `matus@matus.io`
- All commits GPG-signed with `user.signingkey matus@matus.io`
- Never add co-authors or bot attributions
- Use Conventional Commits format
- If SSH signing fails, stop and ask user (don't retry)
- If unexpected file changes detected, stop and ask user (concurrent instances)

---

## Tips for Efficient Development

1. **Read First**: Check `plan/2026-01-14-ARCHITECTURE.md` before implementing
2. **Update Progress**: Mark status in `plan/2026-01-14-PROGRESS.md` immediately
3. **Test Early**: Write tests alongside implementation
4. **Commit Often**: Small, logical commits after each unit of work
5. **Follow Phases**: Stick to phase order unless necessary
6. **Document Decisions**: Add notes for non-obvious choices

---

## Quick Start Checklist

For each coding session:

1. [ ] Read CLAUDE.md (this file)
2. [ ] Check `plan/2026-01-14-PROGRESS.md` for next feature
3. [ ] Review `plan/2026-01-14-ARCHITECTURE.md` for design
4. [ ] Update progress to 🟨 In Progress
5. [ ] Implement following architecture
6. [ ] Write tests for success criteria
7. [ ] Run `cargo test && cargo clippy && cargo fmt`
8. [ ] Update progress to ✅ Completed
9. [ ] Commit with Conventional Commits format
10. [ ] Repeat!

---

**Version**: 0.1.0 | **License**: AGPL-3.0-or-later | **Updated**: 2026-01-18

**Welcome to LocalRouter AI development!** 🚀
