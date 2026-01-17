# LocalRouter AI - Project Guide for Claude Code

This document serves as a comprehensive guide for understanding and working with the LocalRouter AI project. It explains the key documents, development workflow, and how to navigate the codebase effectively.

## Project Overview

LocalRouter AI is a cross-platform desktop application built with Rust and Tauri that provides a local OpenAI-compatible API gateway with intelligent routing, API key management, and multi-provider support.

**Tech Stack**: Rust (backend), Tauri 2.x (desktop framework), Axum (web server)

## Documentation Organization

All planning and architectural documents are stored in the `plan/` directory with a date prefix indicating when they were created. This helps track the evolution of project decisions and documentation over time.

**Naming Convention**: `plan/YYYY-MM-DD-<DOCUMENT_NAME>.md`

**Examples**:
- `plan/2026-01-14-ARCHITECTURE.md` - System architecture and design
- `plan/2026-01-14-PROGRESS.md` - Feature tracking and implementation progress
- `plan/2026-01-14-CONTRIBUTING.md` - Development workflow and guidelines

**Exceptions**: Only `README.md` and `CLAUDE.md` (this file) remain in the root directory for easy discovery.

**When creating new planning documents**:
1. Create them in the `plan/` directory
2. Prefix the filename with the current date in `YYYY-MM-DD` format
3. Use descriptive, uppercase filenames (e.g., `FEATURE_SPEC.md`, `API_DESIGN.md`)

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
├── server/                 # Web server (Axum, OpenAI API)
│   ├── mod.rs             # Module definition
│   └── ...                # Route handlers, middleware
├── config/                 # Configuration management
│   ├── mod.rs             # Settings struct, load/save
│   └── ...                # Migration, validation
├── providers/              # Model provider implementations
│   ├── mod.rs             # ModelProvider trait
│   ├── ollama.rs          # Ollama provider
│   ├── openai.rs          # OpenAI provider
│   └── ...                # Other providers
├── router/                 # Smart routing system
│   ├── mod.rs             # Router config
│   ├── engine.rs          # Routing algorithm
│   ├── strategy.rs        # Routing strategies
│   └── rate_limit.rs      # Rate limiting
├── api_keys/               # API key management
│   ├── mod.rs             # Key CRUD operations
│   └── auth.rs            # Authentication middleware
├── monitoring/             # Monitoring & logging
│   ├── mod.rs             # Module definition
│   ├── metrics.rs         # In-memory metrics
│   ├── logger.rs          # Access log writer
│   └── graphs.rs          # Graph data generation
├── ui/                     # Tauri integration
│   ├── mod.rs             # Module exports
│   ├── commands.rs        # Tauri command handlers
│   └── tray.rs            # System tray
└── utils/                  # Utilities
    ├── mod.rs             # Module exports
    ├── crypto.rs          # Cryptographic functions
    └── errors.rs          # Error types
```

### Frontend
```
src/
└── index.html             # Main HTML (placeholder for now)
```

### Finding Things

**Where is X implemented?**
- Configuration → `src-tauri/src/config/`
- Web server → `src-tauri/src/server/`
- Providers → `src-tauri/src/providers/`
- Routing → `src-tauri/src/router/`
- API keys → `src-tauri/src/api_keys/`
- Metrics → `src-tauri/src/monitoring/`
- Tauri commands → `src-tauri/src/ui/commands.rs`

**How does X work?**
- Check `plan/2026-01-14-ARCHITECTURE.md` for the design
- Check the relevant module's `mod.rs` for implementation

**What needs to be done for X?**
- Check `plan/2026-01-14-PROGRESS.md` for the feature breakdown

---

## Common Questions

### Q: Where do I start?
**A**: Open `plan/2026-01-14-PROGRESS.md` and start with Phase 1 features. The configuration system (1.2) is a good starting point.

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
# Development mode
cargo tauri dev

# Build release
cargo tauri build

# Run tests
cargo test

# Check compilation
cargo check
```

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
**A**:
- Unit tests: Test individual functions/components
- Integration tests: Test component interactions
- E2E tests: Test full application flow
- See each feature's "Testing" section in `plan/2026-01-14-PROGRESS.md`

---

## Phase Overview

### Phase 1: Core Infrastructure (Current)
Build the foundation: configuration, logging, error handling, crypto utilities.

### Phase 2: Model Providers
Implement provider abstraction and 5+ providers (Ollama, OpenAI, Anthropic, etc.).

### Phase 3: Smart Router
Build the intelligent routing system with strategies and fallbacks.

### Phase 4: Web Server & API
Implement the OpenAI-compatible HTTP API.

### Phase 5: API Key Management
Add key generation, storage, and authentication.

### Phase 6: Monitoring
Implement metrics collection, logging, and graph data.

### Phase 7: Desktop UI
Build the Tauri frontend with all tabs and system tray.

### Phase 8: Polish & Testing
Comprehensive testing, documentation, and packaging.

---

## Quick Reference

### Files to Check Regularly
- `plan/2026-01-14-PROGRESS.md` - Track your work
- `plan/2026-01-14-ARCHITECTURE.md` - Understand the design
- `src-tauri/src/utils/errors.rs` - Error types
- `Cargo.toml` - Dependencies

### Commands to Run Often
```bash
cargo check        # Quick compilation check
cargo test         # Run tests
cargo clippy       # Linting
cargo fmt          # Format code
cargo tauri dev    # Run in dev mode
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

**Current Status**: Initial setup complete ✅

**Next Steps**: Begin Phase 1 implementation
- Configuration system (1.2)
- Error handling (1.4)
- Encrypted storage (1.3)

**Progress**: 0/150+ features completed (just getting started!)

---

## Additional Resources

- **Rust Book**: https://doc.rust-lang.org/book/
- **Tauri Documentation**: https://tauri.app/v2/
- **Axum Documentation**: https://docs.rs/axum/
- **Tokio Documentation**: https://tokio.rs/

---

**Last Updated**: 2026-01-17
**Project Version**: 0.1.0
**Status**: Setup Complete, Ready for Development

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
