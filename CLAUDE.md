# LocalRouter AI - Project Guide for Claude Code

This document serves as a comprehensive guide for understanding and working with the LocalRouter AI project. It explains the key documents, development workflow, and how to navigate the codebase effectively.

## Project Overview

LocalRouter AI is a cross-platform desktop application built with Rust and Tauri that provides a local OpenAI-compatible API gateway with intelligent routing, API key management, and multi-provider support.

**Tech Stack**: Rust (backend), Tauri 2.x (desktop framework), Axum (web server)

## Documentation Organization

All planning and architectural documents are stored in the **local** `plan/` directory with a date prefix indicating when they were created. This helps track the evolution of project decisions and documentation over time.

**IMPORTANT**: Use the **project-local** `./plan/` directory, NOT `~/.claude/plans/`. This ensures:
- Plans are version-controlled with the project
- Plans are accessible to all contributors
- Plans persist with the repository
- No dependency on Claude Code's global storage

**Naming Convention**: `plan/YYYY-MM-DD-<DOCUMENT_NAME>.md`

**Examples**:
- `plan/2026-01-14-ARCHITECTURE.md` - System architecture and design
- `plan/2026-01-14-PROGRESS.md` - Feature tracking and implementation progress
- `plan/2026-01-14-CONTRIBUTING.md` - Development workflow and guidelines

**Exceptions**: Only `README.md` and `CLAUDE.md` (this file) remain in the root directory for easy discovery.

**When creating new planning documents**:
1. Create them in the **project-local** `./plan/` directory (NOT `~/.claude/plans/`)
2. Prefix the filename with the current date in `YYYY-MM-DD` format
3. Use descriptive, uppercase filenames (e.g., `FEATURE_SPEC.md`, `API_DESIGN.md`)
4. Ensure the document is committed to version control

## Key Documents and Their Purpose

### 1. plan/2026-01-14-ARCHITECTURE.md
**Purpose**: Complete system design and technical specifications

**What's Inside**:
- System architecture diagram showing all components and their relationships
- Detailed breakdown of 9 major components with interfaces and data structures
- Technology choices and rationale
- Module structure and file organization
- Security and performance considerations
- Development phases overview

**When to Use**:
- Before implementing any new feature (understand the design first)
- When making architectural decisions
- When adding new components or modules
- When someone asks "how does X work?"
- For onboarding new developers

**Key Sections**:
- **Component Breakdown**: Detailed specs for each system component
- **Provider Trait System**: How model providers are abstracted
- **Smart Router Design**: How routing logic works
- **Module Structure**: File organization and responsibilities

---

### 2. plan/2026-01-14-PROGRESS.md
**Purpose**: Comprehensive feature tracking and implementation progress

**What's Inside**:
- **150+ individual features** organized into 8 phases
- Success criteria for each feature (what defines "done")
- Testing criteria for each feature (how to verify it works)
- Status tracking (⬜ Not Started, 🟨 In Progress, ✅ Completed, ⚠️ Blocked)
- Summary statistics and next steps

**When to Use**:
- At the start of each coding session (pick what to implement next)
- After completing a feature (mark it ✅ and update status)
- To understand project progress and what remains
- When planning work
- To avoid duplicate work

**How to Update**:
1. Find the feature you're working on
2. Change status from ⬜ to 🟨 when starting
3. Check off success criteria as you complete them
4. Mark ✅ when all criteria are met
5. Add implementation notes if relevant

**Example Update**:
```markdown
### 1.2 Configuration System
**Status**: ✅ Completed

**Features**:
- [x] Create `AppConfig` struct with all settings
- [x] Implement YAML configuration loading
- [x] Implement configuration saving
...

**Implementation Notes**: Used `config` crate with YAML backend. Chose bcrypt for key hashing.
```

---

### 3. README.md
**Purpose**: Project introduction and quick start guide

**What's Inside**:
- High-level project description
- Features overview
- Installation instructions
- Usage examples
- Links to other documentation

**When to Use**:
- First time seeing the project
- Need quick start instructions
- Want to understand what the project does at a high level
- Writing documentation or blog posts about the project

---

### 4. plan/2026-01-14-CONTRIBUTING.md
**Purpose**: Development workflow and contribution guidelines

**What's Inside**:
- Development setup instructions
- Code style guidelines
- Commit message conventions (Conventional Commits)
- Testing requirements
- Pull request process
- Feature implementation workflow

**When to Use**:
- Before making your first commit
- When unsure about code style or conventions
- Before submitting a pull request
- When setting up the dev environment

---

### 5. CLAUDE.md (This File)
**Purpose**: Guide for navigating the project and understanding the documentation

**When to Use**:
- At the start of any coding session
- When you need to orient yourself in the project
- When unsure which document to reference
- For understanding the development workflow

---

### 6. Additional Planning Documents

All project-related plans are stored in the local `./plan/` directory (not in `~/.claude/plans/`). This ensures plans are version-controlled and accessible to all contributors.

**Recent Plans (2026-01-17)**:

- **plan/2026-01-17-MCP_AUTH_REDESIGN.md** - Comprehensive plan for unified client architecture and MCP authentication redesign. Includes OAuth flow implementation, token management, and migration strategy.
- **plan/2026-01-17-OPENAPI_SPEC.md** - OpenAPI 3.1 specification implementation using utoipa (code-first approach) for auto-generated API documentation.
- **plan/2026-01-17-TAURI_UI_REFACTORING.md** - UI refactoring plan for simplified, consistent Tauri interface structure.
- **plan/2026-01-17-MCP_TESTING_STRATEGY.md** - Comprehensive testing strategy for MCP implementation covering all transport types and OAuth flows.
- **plan/2026-01-17-FEATURE_ADAPTERS.md** - Documentation of feature adapters implementation and testing.
- **plan/2026-01-17-MCP_CONNECTION_EXAMPLES.md** - Real-world MCP connection examples and patterns.
- **plan/2026-01-17-MCP_BUGS_FOUND.md** - Bug tracking and fixes for MCP implementation.
- **plan/2026-01-17-BUG_FIXES_COMPLETE.md** - Completed bug fixes documentation.
- **plan/2026-01-17-TEST_BUG_ANALYSIS.md** - Analysis of test failures and debugging notes.

**Earlier Plans**:
- **plan/2026-01-15-OPENCODE_PROVIDERS_RESEARCH.md** - Research on open-source AI providers.
- **plan/2026-01-15-PROVIDER_LOGOS_GUIDE.md** - Provider branding and logo guidelines.
- **plan/2026-01-15-TRAY_ICON_TESTING.md** - System tray icon implementation testing.
- **plan/2026-01-15-BUG_REPORT.md** - Consolidated bug reports.
- **plan/2026-01-14-WEB_SERVER_IMPLEMENTATION.md** - Web server design and implementation details.
- **plan/2026-01-14-CONFIG_SUBSCRIPTION_GUIDE.md** - Configuration subscription system guide.
- **plan/2026-01-14-PROVIDER_CONFIG_EXAMPLES.md** - Provider configuration examples.
- **plan/2026-01-14-ENDPOINTS.md** - API endpoints documentation.

**When to Reference These Plans**:
- Before implementing OAuth or MCP features → Read MCP_AUTH_REDESIGN.md
- When adding API endpoints → Read OPENAPI_SPEC.md and ENDPOINTS.md
- When refactoring UI → Read TAURI_UI_REFACTORING.md
- When writing tests → Read MCP_TESTING_STRATEGY.md
- When debugging → Check MCP_BUGS_FOUND.md and BUG_FIXES_COMPLETE.md

---

## Development Workflow

### Starting a New Feature

1. **Choose a Feature**:
   - Open `plan/2026-01-14-PROGRESS.md`
   - Find a feature marked ⬜ Not Started
   - Prefer features in the current phase (Phase 1 → Phase 2 → etc.)
   - Check if the feature has dependencies on other features

2. **Understand the Design**:
   - Open `plan/2026-01-14-ARCHITECTURE.md`
   - Read the relevant component section
   - Understand the interfaces, data structures, and relationships
   - Note any security or performance considerations

3. **Update Progress**:
   - In `plan/2026-01-14-PROGRESS.md`, change feature status to 🟨 In Progress
   - This signals to others that you're working on it

4. **Implement**:
   - Follow the architecture design
   - Follow code style guidelines from `plan/2026-01-14-CONTRIBUTING.md`
   - Write tests that verify all success criteria
   - Keep functions small and focused

5. **Test**:
   - Verify all success criteria in `plan/2026-01-14-PROGRESS.md` are met
   - Run unit tests: `cargo test`
   - Run integration tests if applicable
   - Check for linting issues: `cargo clippy`
   - Format code: `cargo fmt`

6. **Complete**:
   - Mark all checkboxes in `plan/2026-01-14-PROGRESS.md` success criteria
   - Change status to ✅ Completed
   - Add implementation notes if relevant
   - Commit with clear message following Conventional Commits

### Example Session

```bash
# 1. Check what to work on
cat plan/2026-01-14-PROGRESS.md | grep "Not Started" | head -5

# 2. Read architecture for that component
# Open plan/2026-01-14-ARCHITECTURE.md, find the relevant section

# 3. Update plan/2026-01-14-PROGRESS.md status to "In Progress"

# 4. Implement the feature
# Write code in src-tauri/src/...

# 5. Write tests
# Add tests to verify success criteria

# 6. Test
cargo test
cargo clippy
cargo fmt

# 7. Update plan/2026-01-14-PROGRESS.md to "Completed"

# 8. Commit
git add .
git commit -m "feat(config): implement YAML configuration loading

- Add AppConfig struct with all settings
- Implement load_config() and save_config()
- Add OS-specific path resolution
- Add tests for config loading/saving"
```

---

## Project Structure Navigation

### Backend (Rust)
```
src-tauri/src/
├── main.rs                 # Entry point, Tauri initialization
├── lib.rs                  # Library root, module declarations
├── server/                 # Web server (Axum, OpenAI API) ✅
│   ├── mod.rs             # Server setup and state
│   ├── manager.rs         # Server lifecycle management
│   ├── state.rs           # Shared application state
│   ├── types.rs           # Request/response types
│   ├── routes/            # API endpoints
│   │   ├── chat.rs        # Chat completions (streaming & non-streaming)
│   │   ├── completions.rs # Text completions
│   │   ├── embeddings.rs  # Embeddings generation
│   │   ├── models.rs      # Model listing
│   │   ├── mcp.rs         # MCP server proxy
│   │   ├── oauth.rs       # OAuth token endpoint
│   │   └── generation.rs  # Text generation
│   ├── middleware/        # HTTP middleware
│   │   ├── mod.rs         # Middleware registration
│   │   ├── oauth_auth.rs  # OAuth authentication
│   │   └── client_auth.rs # Client authentication
│   └── openapi/           # OpenAPI 3.1 spec generation
│       └── mod.rs         # utoipa-based schema
├── config/                 # Configuration management ✅
│   ├── mod.rs             # AppConfig struct, load/save
│   ├── storage.rs         # File-based config storage
│   ├── validation.rs      # Config validation
│   ├── migration.rs       # Schema version migration
│   └── paths.rs           # OS-specific paths
├── providers/              # Model provider implementations ✅
│   ├── mod.rs             # ModelProvider trait
│   ├── factory.rs         # Provider factory
│   ├── registry.rs        # Provider registration
│   ├── health.rs          # Health checking & circuit breaker
│   ├── key_storage.rs     # Provider API key storage
│   ├── ollama.rs          # Ollama (local models)
│   ├── openai.rs          # OpenAI (GPT-4, GPT-3.5, o1)
│   ├── anthropic.rs       # Anthropic (Claude)
│   ├── gemini.rs          # Google Gemini
│   ├── openrouter.rs      # OpenRouter (multi-provider)
│   ├── mistral.rs         # Mistral AI
│   ├── cohere.rs          # Cohere
│   ├── perplexity.rs      # Perplexity
│   ├── lmstudio.rs        # LM Studio
│   ├── openai_compatible.rs # Generic OpenAI-compatible
│   ├── features/          # Provider feature adapters
│   │   ├── mod.rs         # Feature adapter trait
│   │   ├── prompt_caching.rs    # Prompt caching support
│   │   ├── json_mode.rs         # JSON output mode
│   │   ├── logprobs.rs          # Log probabilities
│   │   ├── structured_outputs.rs # Structured output schemas
│   │   ├── openai_reasoning.rs  # OpenAI reasoning models
│   │   └── gemini_thinking.rs   # Gemini thinking mode
│   └── oauth/             # OAuth provider integration
│       ├── mod.rs         # OAuth trait
│       ├── storage.rs     # Token storage
│       ├── anthropic_claude.rs  # Anthropic OAuth
│       ├── github_copilot.rs    # GitHub Copilot
│       └── openai_codex.rs      # OpenAI Codex
├── router/                 # Smart routing system
│   ├── mod.rs             # Router configuration
│   ├── engine.rs          # Routing algorithm (partial)
│   ├── strategy.rs        # Routing strategies
│   └── rate_limit.rs      # Rate limiting ✅
├── api_keys/               # API key management ✅
│   ├── mod.rs             # CRUD operations
│   ├── storage.rs         # Encrypted storage
│   ├── keychain.rs        # System keychain backend
│   └── keychain_trait.rs  # Keychain abstraction
├── clients/                # Unified client system
│   ├── mod.rs             # Client entity (replaces api_keys)
│   └── token_store.rs     # OAuth token management
├── oauth_clients/          # OAuth client configuration
│   └── mod.rs             # OAuth client management
├── mcp/                    # MCP (Model Context Protocol) ✅
│   ├── mod.rs             # MCP manager & types
│   ├── protocol.rs        # MCP protocol implementation
│   ├── oauth.rs           # MCP OAuth flows
│   ├── transport/         # MCP transport layers
│   │   ├── stdio.rs       # STDIO transport (local processes)
│   │   ├── sse.rs         # SSE transport (HTTP)
│   │   └── websocket.rs   # WebSocket transport (being phased out)
├── monitoring/             # Monitoring & logging ✅
│   ├── mod.rs             # Module definition
│   ├── metrics.rs         # Four-tier metrics system
│   ├── logger.rs          # Access log writer
│   └── graphs.rs          # Chart.js data generation
├── ui/                     # Tauri integration ✅
│   ├── mod.rs             # Module exports
│   ├── commands.rs        # Tauri command handlers
│   ├── commands_metrics.rs # Metrics-specific commands
│   └── tray.rs            # System tray with status
└── utils/                  # Utilities ✅
    ├── mod.rs             # Module exports
    ├── crypto.rs          # Cryptographic functions
    └── errors.rs          # Error types
```

**Total**: ~36,683 lines of Rust code

### Frontend (React + TypeScript)
```
src/
├── App.tsx                # Main app component with routing
├── components/
│   ├── Sidebar.tsx        # Navigation sidebar
│   ├── tabs/              # Main tab components
│   │   ├── ApiKeysTab.tsx      # API key management
│   │   ├── ProvidersTab.tsx    # Provider configuration
│   │   ├── ModelsTab.tsx       # Model catalog
│   │   ├── McpServersTab.tsx   # MCP server management
│   │   ├── OAuthClientsTab.tsx # OAuth client config
│   │   └── DocumentationTab.tsx # OpenAPI docs viewer
│   ├── apikeys/           # API key components
│   │   └── ApiKeyDetailPage.tsx
│   ├── providers/         # Provider components
│   │   └── ProviderDetailPage.tsx
│   ├── models/            # Model components
│   │   └── ModelDetailPage.tsx
│   ├── charts/            # Charting components
│   │   └── MetricsChart.tsx # Chart.js integration
│   ├── mcp/               # MCP components
│   │   └── (MCP-related UI)
│   └── oauth/             # OAuth components
│       └── (OAuth-related UI)
└── index.html             # HTML entry point
```

### Finding Things

**Where is X implemented?**
- Configuration → `src-tauri/src/config/`
- Web server & API → `src-tauri/src/server/`
- Providers → `src-tauri/src/providers/`
- Feature adapters → `src-tauri/src/providers/features/`
- OAuth integration → `src-tauri/src/providers/oauth/`
- Routing → `src-tauri/src/router/`
- API keys → `src-tauri/src/api_keys/` (legacy) or `src-tauri/src/clients/` (new)
- MCP servers → `src-tauri/src/mcp/`
- Metrics & monitoring → `src-tauri/src/monitoring/`
- OpenAPI docs → `src-tauri/src/server/openapi/`
- Tauri commands → `src-tauri/src/ui/commands.rs`
- Frontend UI → `src/` (React components)

**How does X work?**
- Check `plan/2026-01-14-ARCHITECTURE.md` for the original design
- Check `plan/2026-01-17-*` files for recent architectural changes
- Check the relevant module's `mod.rs` for implementation details

**What needs to be done for X?**
- Check `plan/2026-01-14-PROGRESS.md` for the feature breakdown
- Check recent plan files for specific implementation strategies

---

## Common Questions

### Q: Where do I start?
**A**: Most core features are implemented. Current priorities:
1. Fix failing MCP tests (see "Known Issues" above)
2. Complete unified Client architecture (`plan/2026-01-17-MCP_AUTH_REDESIGN.md`)
3. Finish routing engine (strategies, fallback)
4. UI refactoring for unified Clients tab

### Q: I want to add a new provider. What do I do?
**A**:
1. Read the Provider Trait System section in `plan/2026-01-14-ARCHITECTURE.md`
2. Look at Phase 2 in `plan/2026-01-14-PROGRESS.md` for provider implementation tasks
3. Check an existing provider (e.g., `src-tauri/src/providers/ollama.rs`) as a reference
4. Implement the `ModelProvider` trait for your new provider

### Q: How do I know if a feature is complete?
**A**: Check the success criteria in `plan/2026-01-14-PROGRESS.md`. All checkboxes should be marked, and all tests should pass.

### Q: Can I change the architecture?
**A**: Yes, but update `plan/2026-01-14-ARCHITECTURE.md` to reflect the change. Discuss significant changes first.

### Q: How do I run the app?
**A**:
```bash
# Development mode (recommended - uses ~/.localrouter-dev/)
cargo tauri dev

# Build release
cargo tauri build

# Run tests
cargo test --lib  # Unit tests only
cargo test        # All tests (some integration tests may fail without API keys)

# Check compilation
cargo check
```

### Q: How do I use the API?
**A**: Once running, the server listens on `http://localhost:3625`. Example:

```bash
# Get API key from the UI first (API Keys tab)
curl http://localhost:3625/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-api-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

Available endpoints:
- `GET /v1/models` - List available models
- `POST /v1/chat/completions` - Chat completions (streaming supported)
- `POST /v1/completions` - Text completions
- `POST /v1/embeddings` - Generate embeddings
- `POST /mcp/*` - MCP server proxy
- `POST /oauth/token` - OAuth token endpoint
- `GET /openapi.json` - OpenAPI specification
- `GET /health` - Health check

### Q: What's the coding style?
**A**: Follow the Rust standard style:
- Run `cargo fmt` before committing
- Run `cargo clippy -- -D warnings` to catch issues
- See `plan/2026-01-14-CONTRIBUTING.md` for detailed guidelines

### Q: How do I update progress?
**A**: Edit `plan/2026-01-14-PROGRESS.md` directly:
- Change status from ⬜ to 🟨 when starting
- Mark checkboxes with `[x]` when complete
- Change status to ✅ when done
- Add notes under "Implementation Notes" if relevant

### Q: What's the test strategy?
**A**: Multi-layered testing approach:
- **Unit tests**: Test individual functions/components (most of 367 tests)
- **Integration tests**: Test provider APIs, MCP flows, OAuth (8 ignored, need external services)
- **Feature adapter tests**: Comprehensive tests in `tests/provider_integration_tests.rs`
- **E2E tests**: Not yet implemented
- See `plan/2026-01-17-MCP_TESTING_STRATEGY.md` for MCP-specific testing

**Current status**: 367 tests, 353 passing (96% pass rate)

---

## Phase Overview

### Phase 1: Core Infrastructure ✅ (Complete)
- ✅ Configuration system with YAML storage
- ✅ Error handling and logging
- ✅ Encrypted storage with keychain integration
- ✅ OS-specific path resolution

### Phase 2: Model Providers ✅ (Complete)
- ✅ ModelProvider trait abstraction
- ✅ 10+ provider implementations:
  - Ollama, OpenAI, Anthropic, Google Gemini, OpenRouter
  - Mistral, Cohere, Perplexity, LM Studio, OpenAI-compatible
- ✅ Health checking with circuit breaker
- ✅ Feature adapters (prompt caching, JSON mode, logprobs, structured outputs, reasoning)
- ✅ OAuth provider integration (Anthropic, GitHub Copilot, OpenAI Codex)

### Phase 3: Smart Router ⚠️ (Partial - 20%)
- ✅ Rate limiting (requests, tokens, cost)
- ⚠️ Routing engine (partial implementation)
- ❌ Routing strategies (not complete)
- ❌ Fallback system (not implemented)

### Phase 4: Web Server & API ✅ (Complete)
- ✅ HTTP server setup (Axum on port 3625)
- ✅ OpenAI compatibility layer
- ✅ Authentication middleware (OAuth + Bearer tokens)
- ✅ All core endpoints:
  - `/v1/chat/completions` (streaming & non-streaming)
  - `/v1/completions`
  - `/v1/embeddings`
  - `/v1/models`
  - `/mcp/*` (MCP server proxy)
  - `/oauth/token` (OAuth token endpoint)
- ✅ OpenAPI 3.1 documentation (`/openapi.json`)

### Phase 5: API Key Management ✅ (Complete - transitioning to unified Clients)
- ✅ API key generation (bcrypt hashing)
- ✅ Encrypted storage (AES-256-GCM)
- ✅ System keyring integration with fallback
- ✅ Authentication middleware
- ⚠️ Transitioning to unified Client architecture

### Phase 6: Monitoring ✅ (Complete)
- ✅ Four-tier metrics system (request, API key, provider, global)
- ✅ Access log writer with JSON Lines format
- ✅ Historical log parser
- ✅ Chart.js graph data generation
- ✅ Real-time metrics collection

### Phase 7: Desktop UI ✅ (Complete)
- ✅ Tauri desktop application
- ✅ React + TypeScript frontend
- ✅ All main tabs:
  - Dashboard (metrics overview)
  - API Keys (management UI)
  - Providers (configuration)
  - Models (catalog browser)
  - MCP Servers (MCP management)
  - OAuth Clients (OAuth config)
  - Documentation (OpenAPI viewer)
- ✅ System tray with status indicator
- ✅ Metrics charts and dashboards

### Phase 8: Polish & Testing ⚠️ (In Progress - 50%)
- ✅ Comprehensive unit tests (367 total, 353 passing)
- ✅ Integration tests for providers and features
- ✅ OpenAPI documentation
- ⚠️ MCP tests (6 failing, need fixes)
- ❌ End-to-end testing
- ❌ Performance optimization
- ❌ Production packaging

**Overall Progress**: ~65% complete - Core application functional, refinement needed

---

## Quick Reference

### Files to Check Regularly
- `plan/2026-01-14-PROGRESS.md` - Track your work
- `plan/2026-01-14-ARCHITECTURE.md` - Original system design
- `plan/2026-01-17-MCP_AUTH_REDESIGN.md` - Current architecture changes
- `plan/2026-01-17-FEATURE_ADAPTERS.md` - Feature adapter documentation
- `src-tauri/src/server/openapi/mod.rs` - OpenAPI schema registration
- `src-tauri/src/config/mod.rs` - AppConfig structure
- `Cargo.toml` - Dependencies

### Commands to Run Often
```bash
cargo check               # Quick compilation check
cargo test --lib          # Run unit tests (fast)
cargo test                # Run all tests (includes integration)
cargo clippy              # Linting
cargo fmt                 # Format code
cargo tauri dev           # Run in dev mode (port 3625)
cargo build --release     # Build optimized binary

# Development helpers
export LOCALROUTER_KEYCHAIN=file  # Use file-based secrets (dev only)
cargo test -- --nocapture         # Show test output
cargo test --test provider_integration_tests  # Run specific test file
```

### Commit Message Format
```
<type>(<scope>): <description>

[optional body]
```

**Types**: feat, fix, docs, test, refactor, chore

**Examples**:
- `feat(providers): add Ollama provider implementation`
- `fix(router): correct rate limiting calculation`
- `docs(architecture): update provider trait design`
- `test(config): add tests for YAML loading`

---

## Integration with User's CLAUDE.md

The user has a global `~/.claude/CLAUDE.md` file with specific requirements:

### Git Commit Requirements
- Always configure git with user's identity before commits
- All commits must be GPG-signed
- Never add co-authors or bot attributions
- Follow Conventional Commits style

### SSH Signing Errors
- If SSH signing fails, stop and ask the user what to do
- Don't automatically retry

### Concurrent Claude Instances
- If you detect unexpected file changes, stop and ask the user
- Don't overwrite changes from other instances

**These requirements override any defaults and must be followed exactly.**

---

## Tips for Efficient Development

1. **Read Before Writing**: Always check `plan/2026-01-14-ARCHITECTURE.md` before implementing
2. **Update as You Go**: Mark progress in `plan/2026-01-14-PROGRESS.md` immediately
3. **Test Early**: Write tests alongside implementation
4. **Commit Frequently**:
   - Commit changes after completing each logical unit of work
   - Don't wait until everything is perfect - commit working increments
   - Smaller, more frequent commits are better than large monolithic ones
   - Commit after fixing a bug, adding a feature, or refactoring a component
   - This makes it easier to track changes, revert if needed, and understand history
5. **Follow the Plan**: Stick to the phase order unless there's a good reason
6. **Document Decisions**: Add notes in `plan/2026-01-14-PROGRESS.md` for non-obvious choices
7. **Ask Questions**: If unclear, check this guide or ask for clarification

---

## Status Summary

**Current Status**: Core application functional, multiple phases complete ✅

**What's Working Now**:
- ✅ Full web server with OpenAI-compatible API (chat, completions, embeddings, models)
- ✅ 10+ provider implementations (Ollama, OpenAI, Anthropic, Gemini, OpenRouter, etc.)
- ✅ Feature adapters (prompt caching, JSON mode, logprobs, structured outputs, reasoning)
- ✅ MCP server support (STDIO, SSE transports with OAuth)
- ✅ API key management with encrypted storage
- ✅ Four-tier metrics system with real-time dashboards
- ✅ Rate limiting (requests, tokens, cost)
- ✅ OAuth authentication & token management
- ✅ OpenAPI 3.1 documentation (auto-generated)
- ✅ Full Tauri desktop UI with system tray
- ✅ Configuration management with migration
- ✅ Health checking with circuit breaker

**Test Status**: 367 tests total (353 passing, 6 failing, 8 ignored)

**Next Priorities**:
- Fix remaining 6 test failures (MCP-related)
- Complete unified Client architecture (replace api_keys with clients)
- Finish routing engine implementation (strategies, fallback)
- Complete MCP authentication redesign
- UI refactoring for unified Clients tab

**Progress**: ~60-70% of core features implemented, application is functional for basic use

---

## Known Issues & Current Work

### Test Failures (6 failing tests)
Current test suite: **367 tests total** (353 passing, 6 failing, 8 ignored)

**Failing tests** (all MCP-related):
- MCP authentication tests
- MCP transport tests
- OAuth flow tests

These failures are being addressed as part of the MCP authentication redesign. See `plan/2026-01-17-TEST_BUG_ANALYSIS.md` for detailed analysis.

### Architecture Transitions in Progress

**Unified Client Architecture** (Partial Implementation):
- Plan: Merge `ApiKeyConfig` and `OAuthClientConfig` into unified `Client` entity
- Status: Client type defined, config migration written, but not fully integrated
- Impact: API Keys tab still uses legacy structure, OAuth Clients separate
- Next: Complete middleware integration and UI refactoring

See `plan/2026-01-17-MCP_AUTH_REDESIGN.md` for the complete redesign plan.

**Routing Engine** (Partial Implementation):
- Status: Rate limiting complete, but routing strategies and fallback system incomplete
- Impact: Requests route to first available provider, no cost optimization
- Next: Implement routing strategies (cost, performance, load balancing)

### UI Refactoring Needed

Current UI has separate tabs for:
- API Keys
- OAuth Clients
- MCP Servers

Target UI should have:
- **Clients** (unified API Keys + OAuth Clients)
- **MCP Servers** (with client assignment)

See `plan/2026-01-17-TAURI_UI_REFACTORING.md` for detailed refactoring plan.

---

## Codebase Statistics

**Size**: ~36,683 lines of Rust code (production + tests)

**Module Breakdown**:
- **Server** (`src-tauri/src/server/`): ~8,000 lines
  - Routes: ~6,000 lines (chat, completions, models, MCP, OAuth)
  - Middleware: ~1,000 lines (authentication, CORS)
  - OpenAPI: ~1,000 lines (schema definitions)
- **Providers** (`src-tauri/src/providers/`): ~10,000 lines
  - Core providers: ~5,000 lines (10+ implementations)
  - Feature adapters: ~3,000 lines (6 adapters)
  - OAuth integration: ~2,000 lines
- **MCP** (`src-tauri/src/mcp/`): ~4,000 lines
  - Protocol implementation: ~1,500 lines
  - Transports: ~1,500 lines (STDIO, SSE, WebSocket)
  - OAuth flows: ~1,000 lines
- **Monitoring** (`src-tauri/src/monitoring/`): ~3,000 lines
  - Metrics system: ~1,500 lines
  - Logging: ~800 lines
  - Graph generation: ~700 lines
- **Configuration** (`src-tauri/src/config/`): ~2,500 lines
- **API Keys/Clients** (`src-tauri/src/api_keys/`, `src-tauri/src/clients/`): ~2,000 lines
- **Routing** (`src-tauri/src/router/`): ~1,500 lines
- **UI Commands** (`src-tauri/src/ui/`): ~1,500 lines
- **Utils** (`src-tauri/src/utils/`): ~500 lines
- **Tests** (`src-tauri/tests/`): ~3,700 lines

**Frontend**: ~3,000 lines of TypeScript/TSX (React components)

**Total Project**: ~40,000 lines of code

**Dependencies**: 50+ crates including:
- Tauri 2.x (desktop framework)
- Axum 0.7 (web server)
- Tokio (async runtime)
- Serde (serialization)
- utoipa (OpenAPI generation)
- Many more...

---

## Development History & Milestones

### Recent Commits (January 2026)

**Week 3 (Jan 17)**:
- `9788e29` - Comprehensive integration tests and documentation for feature adapters
- `59a8e5e` - Week 2 feature adapters (prompt caching, logprobs, JSON mode)
- `d008bde` - Fix metrics MetricType enum serialization
- `15ae69f` - Enhanced token tracking and structured outputs adapter

**Week 2 (Jan 14-16)**:
- `8bee288` - Complete all detail pages with metrics tabs
- `706cb8e` - Add metrics tab to API Key detail page
- `09cd10e` - Transform HomeTab into comprehensive metrics dashboard
- `22acbf0` - Add charting infrastructure and Chart.js components
- `1b28451` - Wire metrics recording into request handlers
- `4861e44` - Implement DetailPageLayout and refactor pages
- `909bd74` - Implement four-tier metrics tracking system

**Week 1 (Jan 13-14)**:
- `860fda4` - Change license from MIT to AGPL-3.0-or-later
- `0890792` - Add file-based storage for development mode
- `4082ac8` - Replace Ollama SDK with direct HTTP API
- `2f3aefb` - Use separate `~/.localrouter-dev` directory for development

### Major Implementation Phases

**Phase 1-2** (Early January): Core infrastructure and provider system
- Configuration management, error handling, crypto utilities
- 10+ provider implementations with feature adapters
- Health checking and circuit breaker pattern

**Phase 4-6** (Mid January): Web server and monitoring
- Full Axum-based web server with OpenAI-compatible API
- Four-tier metrics system with real-time collection
- Access logging and historical analysis

**Phase 7** (Mid-Late January): Desktop UI
- Complete Tauri application with React frontend
- All main tabs (Dashboard, API Keys, Providers, Models, MCP, OAuth, Documentation)
- System tray integration with status indicator
- Chart.js visualizations for metrics

**Current Phase** (Late January): MCP & OAuth integration
- MCP server proxy with STDIO and SSE transports
- OAuth 2.0 client credentials flow
- OpenAPI 3.1 documentation generation
- Unified client architecture (in progress)

---

## Development Mode & Environment

### File-Based Keychain (Development Only)

For development, avoid constant macOS keychain permission prompts by using file-based storage:

```bash
# Add to ~/.zshrc or ~/.bashrc
export LOCALROUTER_KEYCHAIN=file
```

**⚠️ WARNING**: This stores secrets in **plain text** at `~/.localrouter/secrets.json` or `~/.localrouter-dev/secrets.json`. **Only use for development with test API keys**.

See `plan/2026-01-15-DEVELOPMENT.md` for detailed information.

### Development vs Production Directories

When running in development mode (`cargo tauri dev`):
- Config: `~/.localrouter-dev/`
- Secrets: `~/.localrouter-dev/secrets.json` (if using file-based keychain)

When running in production mode (`cargo tauri build`):
- macOS: `~/Library/Application Support/LocalRouter/`
- Linux: `~/.localrouter/`
- Windows: `%APPDATA%\LocalRouter\`

### Server Port

The API server runs on **port 3625** (not 3000). Access at:
- API: `http://localhost:3625/v1/*`
- OpenAPI spec: `http://localhost:3625/openapi.json`
- Health: `http://localhost:3625/health`

---

## Recent Features & Implementations

### MCP (Model Context Protocol) Support
LocalRouter AI acts as an MCP proxy, allowing clients to connect to MCP servers through LocalRouter with authentication.

**Transports Supported**:
- STDIO (local processes, e.g., `npx @modelcontextprotocol/server-filesystem`)
- SSE (HTTP Server-Sent Events, e.g., Anthropic Supergateway)
- WebSocket (being phased out)

**Authentication**:
- OAuth 2.0 (auto-discovery and manual configuration)
- Bearer tokens
- Custom headers

See `plan/2026-01-17-MCP_AUTH_REDESIGN.md` and `plan/2026-01-17-MCP_CONNECTION_EXAMPLES.md` for details.

### Feature Adapters
Provider-specific feature adapters allow use of advanced model capabilities through a unified interface:

- **Prompt Caching**: Anthropic prompt caching for faster responses
- **JSON Mode**: Structured JSON output (OpenAI, Anthropic)
- **Logprobs**: Token probability logging (OpenAI)
- **Structured Outputs**: JSON schema validation (OpenAI)
- **Reasoning Models**: OpenAI o1/o3 reasoning modes
- **Thinking Mode**: Gemini 2.0 Flash Thinking

See `plan/2026-01-17-FEATURE_ADAPTERS.md` for comprehensive documentation and tests.

### Metrics System
Four-tier metrics collection system:

1. **Request-level**: Individual request tracking
2. **API Key-level**: Per-client aggregation
3. **Provider-level**: Per-provider statistics
4. **Global-level**: System-wide metrics

Metrics tracked: requests, tokens (input/output), cost, latency (P50/P95/P99), success rate, errors.

All metrics are available via Tauri commands and displayed in the Dashboard tab with Chart.js visualizations.

### OAuth Integration
OAuth 2.0 client credentials flow for:
- Client authentication to LocalRouter
- LocalRouter authentication to OAuth-protected MCP servers
- Token storage and refresh

Supports both server-side token endpoint (`/oauth/token`) and client-side token management.

### OpenAPI Documentation
Auto-generated OpenAPI 3.1 specification using `utoipa`:
- Code-first approach (annotations in Rust)
- Live documentation at `/openapi.json`
- Interactive "Try It Out" in Documentation tab
- Automatic schema generation for all types

---

## Additional Resources

- **Rust Book**: https://doc.rust-lang.org/book/
- **Tauri Documentation**: https://tauri.app/v2/
- **Axum Documentation**: https://docs.rs/axum/
- **Tokio Documentation**: https://tokio.rs/

---

**Last Updated**: 2026-01-17 (comprehensive status update)
**Project Version**: 0.1.0
**License**: AGPL-3.0-or-later (changed from MIT on 2026-01-14)
**Status**: Core Application Functional - 65% Complete

---

## OpenAPI Documentation Requirements

LocalRouter AI uses OpenAPI 3.1 specification for API documentation. The spec is automatically generated from code annotations using utoipa.

### When Adding New Endpoints

When adding a new API endpoint, you MUST:

1. **Annotate the route handler** with `#[utoipa::path]`:
   ```rust
   #[utoipa::path(
       post,
       path = "/v1/your-endpoint",
       tag = "category",
       request_body = YourRequestType,
       responses(
           (status = 200, description = "Success", body = YourResponseType),
           (status = 400, description = "Bad request", body = ErrorResponse),
           (status = 401, description = "Unauthorized", body = ErrorResponse)
       ),
       security(("bearer_auth" = []))
   )]
   pub async fn your_handler(/* ... */) -> ApiResult<Response> {
       // Implementation
   }
   ```

2. **Add types to OpenAPI builder** in `src-tauri/src/server/openapi/mod.rs`:
   ```rust
   paths(
       // ... existing paths
       crate::server::routes::your_module::your_handler,
   ),
   components(
       schemas(
           // ... existing schemas
           crate::server::types::YourRequestType,
           crate::server::types::YourResponseType,
       ),
   )
   ```

3. **Ensure request/response types have schemas**:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
   #[schema(
       title = "Your Type",
       description = "What this type represents",
       example = json!({"field": "value"})
   )]
   pub struct YourType {
       #[schema(example = "example value")]
       pub field: String,
   }
   ```

4. **Refresh the documentation** after adding endpoints:
   - Compile: `cargo check` (spec regenerates automatically)
   - Verify: Access http://localhost:3625/openapi.json
   - Test: Open Documentation tab in UI and verify new endpoint appears

### When Modifying Existing Endpoints

When modifying an existing endpoint:

1. Update the `#[utoipa::path]` annotation if:
   - Path or method changes
   - Request/response types change
   - New query parameters or headers added
   - Error responses change

2. Update type schemas if:
   - New fields added/removed
   - Field types change
   - Validation constraints change (min, max, required, etc.)

3. Update examples to reflect realistic current usage

### Best Practices

- **Keep schemas in sync**: Always update OpenAPI annotations when changing types
- **Add descriptions**: Use `description` attribute for fields and endpoints
- **Provide examples**: Include realistic examples for complex types
- **Document errors**: List all possible error responses
- **Test "Try It Out"**: Verify endpoints work in Documentation tab before committing

### Validation

Before committing changes:

```bash
# Ensure spec compiles
cargo check

# Validate spec is valid OpenAPI 3.1
npx @apidevtools/swagger-cli validate http://localhost:3625/openapi.json

# Run tests
cargo test
```

### Common Mistakes

❌ **Don't**: Add endpoint without `#[utoipa::path]` annotation
✅ **Do**: Always annotate new endpoints

❌ **Don't**: Skip adding types to OpenAPI builder
✅ **Do**: Register all request/response types in `openapi/mod.rs`

❌ **Don't**: Leave `#[derive(ToSchema)]` off new types
✅ **Do**: Add schema derives to all API types

❌ **Don't**: Forget to update examples when behavior changes
✅ **Do**: Keep examples current and realistic

---

## Quick Start Checklist

For each coding session:

1. [ ] Read this file (CLAUDE.md) to orient yourself
2. [ ] Check `plan/2026-01-14-PROGRESS.md` to see current status and pick a feature
3. [ ] Review relevant section in `plan/2026-01-14-ARCHITECTURE.md`
4. [ ] Update `plan/2026-01-14-PROGRESS.md` to mark feature as "In Progress"
5. [ ] Implement the feature following the architecture
6. [ ] Write tests to verify success criteria
7. [ ] Run `cargo test && cargo clippy && cargo fmt`
8. [ ] Update `plan/2026-01-14-PROGRESS.md` to mark feature as "Completed"
9. [ ] Commit with proper message format
10. [ ] Repeat!

---

**Welcome to LocalRouter AI development!** 🚀
