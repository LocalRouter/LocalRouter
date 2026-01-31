# Plan: Align LocalRouter Providers with OpenCode

**Date**: 2026-01-28
**Status**: Proposed

## Goal

Restructure LocalRouter's provider system to match OpenCode's architecture:
1. **Same provider list** - 75+ providers (vs current 19)
2. **Same structure** - One provider with multiple auth methods, not separate providers
3. **Same auth techniques** - Plugin-based auth (API key, OAuth, env vars)
4. **Update process** - One-way sync from OpenCode via build-time script

---

## Decisions

| Decision | Choice |
|----------|--------|
| **Provider definitions** | Build-time generation from OpenCode |
| **Auth storage** | System keychain (existing approach) |
| **Sync trigger** | Manual script before releases |
| **Sync direction** | One-way from OpenCode (we don't own it) |

---

## Current vs Target Architecture

### Current State (LocalRouter)

```
OpenAIProviderFactory (API key)     → "openai" provider
OpenAICodexProviderFactory (OAuth)  → "openai-chatgpt-plus" provider (SEPARATE!)
GitHubCopilotProviderFactory        → "github-copilot" provider
```

- 19 providers with auth baked into each factory
- Different auth methods = different provider types
- Auth storage in keychain per-provider

### Target State (Matching OpenCode)

```
"openai" provider:
  - auth methods: [api_key, oauth_codex]
  - env: ["OPENAI_API_KEY"]

"github-copilot" provider:
  - auth methods: [oauth_device_code]
  - supports: github.com + enterprise
```

- 75+ providers with auth as a separate layer
- One provider = multiple auth methods
- Plugin-based auth system

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    BUILD TIME (Manual)                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ../opencode/packages/opencode/src/provider/                   │
│        │                                                         │
│        ▼                                                         │
│   scripts/sync-providers.rs                                     │
│        │                                                         │
│        ├──> src-tauri/src/providers/definitions.rs (generated)  │
│        │    - ProviderDefinition structs                        │
│        │    - AuthMethodDef structs                             │
│        │    - PROVIDER_DEFINITIONS static                       │
│        │                                                         │
│        └──> src-tauri/src/auth/oauth_configs.rs (generated)     │
│             - OAuth client IDs, endpoints                        │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                         RUNTIME                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   User selects provider "openai"                                │
│        │                                                         │
│        ▼                                                         │
│   Auth layer presents options:                                  │
│   ┌─────────────────────────────────────────┐                   │
│   │ How would you like to authenticate?      │                  │
│   │ ○ API Key (enter your OPENAI_API_KEY)   │                  │
│   │ ○ ChatGPT Plus (OAuth sign-in)          │                  │
│   └─────────────────────────────────────────┘                   │
│        │                                                         │
│        ▼                                                         │
│   Auth stored in system keychain                                │
│        │                                                         │
│        ▼                                                         │
│   Provider loader creates OpenAIProvider with auth              │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Implementation Phases

### Phase 1: Create Sync Script

**Goal**: Extract provider definitions from OpenCode

Create `scripts/sync-providers.rs` (Rust build tool):

```rust
// Read from OpenCode source files:
// - provider.ts: BUNDLED_PROVIDERS, CUSTOM_LOADERS
// - models.ts: ModelsDev provider structure
// - plugin/copilot.ts: GitHub OAuth config
// - plugin/codex.ts: OpenAI OAuth config

// Generate:
// - src-tauri/src/providers/definitions.rs
// - src-tauri/src/auth/oauth_configs.rs
```

**Input files (from OpenCode):**
- `../opencode/packages/opencode/src/provider/provider.ts`
- `../opencode/packages/opencode/src/provider/models.ts`
- `../opencode/packages/opencode/src/provider/auth.ts`
- `../opencode/packages/opencode/src/plugin/copilot.ts`
- `../opencode/packages/opencode/src/plugin/codex.ts`

**Output files:**
| Generated File | Content |
|----------------|---------|
| `definitions.rs` | `PROVIDER_DEFINITIONS: &[ProviderDefinition]` |
| `oauth_configs.rs` | OAuth client IDs, authorization URLs, token URLs |

### Phase 2: Auth Layer Restructure

**Goal**: Decouple auth methods from provider factories

**New files:**
```
src-tauri/src/auth/
├── mod.rs           # Module exports
├── types.rs         # AuthMethod enum, AuthMethodDef struct
├── storage.rs       # Keychain wrapper (existing logic moved here)
├── oauth/
│   ├── mod.rs
│   ├── device_code.rs   # GitHub Copilot flow
│   └── pkce.rs          # OpenAI Codex flow
└── env.rs           # Environment variable detection
```

**Key types:**
```rust
// What the user has stored
pub enum AuthCredential {
    ApiKey(String),
    OAuth { access: String, refresh: String, expires: i64 },
}

// What a provider supports (from definitions.rs)
pub struct AuthMethodDef {
    pub method_type: AuthMethodType,
    pub label: String,
    pub env_vars: Vec<&'static str>,
    pub oauth_provider: Option<&'static str>,  // "github", "openai"
}

pub enum AuthMethodType {
    Api,
    OAuth,
    AwsCredentials,
}
```

### Phase 3: Provider Definition Format

**Goal**: Replace per-provider factories with data-driven definitions

**New files:**
```
src-tauri/src/providers/
├── definitions.rs   # Generated: PROVIDER_DEFINITIONS
├── loader.rs        # Load provider from definition + auth
└── custom/          # Provider-specific logic (can't be generated)
    ├── mod.rs
    ├── anthropic.rs # Beta headers, thinking feature
    ├── openai.rs    # Responses API
    ├── copilot.rs   # Custom endpoint, model routing
    └── bedrock.rs   # AWS credential chain
```

**Generated definition structure:**
```rust
pub struct ProviderDefinition {
    pub id: &'static str,              // "openai"
    pub name: &'static str,            // "OpenAI"
    pub api_url: Option<&'static str>, // "https://api.openai.com/v1"
    pub auth_methods: &'static [AuthMethodDef],
    pub env_vars: &'static [&'static str],
    pub custom_loader: Option<&'static str>, // "openai" -> uses custom/openai.rs
}

pub static PROVIDER_DEFINITIONS: &[ProviderDefinition] = &[
    ProviderDefinition {
        id: "openai",
        name: "OpenAI",
        api_url: Some("https://api.openai.com/v1"),
        auth_methods: &[
            AuthMethodDef {
                method_type: AuthMethodType::Api,
                label: "API Key",
                env_vars: &["OPENAI_API_KEY"],
                oauth_provider: None,
            },
            AuthMethodDef {
                method_type: AuthMethodType::OAuth,
                label: "ChatGPT Plus",
                env_vars: &[],
                oauth_provider: Some("openai"),
            },
        ],
        env_vars: &["OPENAI_API_KEY"],
        custom_loader: Some("openai"),
    },
    // ... 74 more providers
];
```

### Phase 4: Merge Existing Provider Factories

**Goal**: Consolidate duplicate providers

| Current (Separate) | New (Unified) |
|--------------------|---------------|
| `OpenAIProviderFactory` | `openai` provider, `api` auth |
| `OpenAICodexProviderFactory` | `openai` provider, `oauth` auth |
| `GitHubCopilotProviderFactory` | `github-copilot` provider |

**Migration in `factory.rs`:**
```rust
// OLD: Separate factory per auth method
pub struct OpenAIProviderFactory;
pub struct OpenAICodexProviderFactory;

// NEW: Single factory that reads definitions
pub struct UnifiedProviderFactory {
    definition: &'static ProviderDefinition,
}

impl UnifiedProviderFactory {
    pub fn for_provider(id: &str) -> Option<Self> {
        PROVIDER_DEFINITIONS.iter()
            .find(|d| d.id == id)
            .map(|definition| Self { definition })
    }
}
```

### Phase 5: Add Missing Providers

**Goal**: Expand from 19 to 75+ providers

**Provider categories from OpenCode:**

| Category | Providers | Status |
|----------|-----------|--------|
| First-party | anthropic, openai, google, mistral, groq, cohere, xai | ✅ Have |
| Third-party | openrouter, togetherai, deepinfra, perplexity | ✅ Have |
| Local | ollama, lmstudio | ✅ Have |
| OAuth | github-copilot, gitlab | 🔄 Needs auth rework |
| Cloud | azure, bedrock, vertex | 🆕 Need custom loaders |
| Regional | deepseek, moonshotai, alibaba, zhipuai | 🆕 Add via definitions |
| Specialized | fireworks, huggingface, baseten, nvidia | 🆕 Add via definitions |

Most new providers are OpenAI-compatible and need only:
1. Definition in `definitions.rs` (generated)
2. No custom loader (use generic OpenAI-compatible)

---

## Files Summary

### New Files
| File | Purpose |
|------|---------|
| `scripts/sync-providers.rs` | Sync script to extract from OpenCode |
| `src-tauri/src/auth/mod.rs` | Auth module root |
| `src-tauri/src/auth/types.rs` | AuthCredential, AuthMethodDef |
| `src-tauri/src/auth/storage.rs` | Keychain wrapper |
| `src-tauri/src/auth/oauth/mod.rs` | OAuth flows |
| `src-tauri/src/auth/oauth/device_code.rs` | GitHub device code flow |
| `src-tauri/src/auth/oauth/pkce.rs` | OpenAI PKCE flow |
| `src-tauri/src/auth/env.rs` | Env var detection |
| `src-tauri/src/providers/definitions.rs` | **Generated**: Provider definitions |
| `src-tauri/src/providers/loader.rs` | Provider instantiation from definition |
| `src-tauri/src/providers/custom/mod.rs` | Custom loader registry |
| `src-tauri/src/auth/oauth_configs.rs` | **Generated**: OAuth configs |

### Modified Files
| File | Changes |
|------|---------|
| `src-tauri/src/providers/factory.rs` | Replace per-provider factories with UnifiedProviderFactory |
| `src-tauri/src/providers/registry.rs` | Support auth method selection per instance |
| `src-tauri/src/providers/mod.rs` | Export new modules |
| `src-tauri/src/main.rs` | Initialize auth module |
| `src-tauri/build.rs` | Optionally run sync script |
| `src/components/ProviderForm.tsx` | UI for auth method selection |

### Deleted/Deprecated Files
| File | Reason |
|------|--------|
| `src-tauri/src/providers/oauth/` | Moved to `auth/oauth/` |

---

## Sync Script Usage

```bash
# Manual sync before release
cd scripts
cargo run --bin sync-providers -- \
    --opencode-path ../opencode \
    --output-dir ../src-tauri/src

# Verify changes
cargo build
cargo test
```

**When to run:**
- Before each LocalRouter release
- When OpenCode adds new providers
- When OpenCode changes auth methods

---

## Config Migration

### Old Format
```yaml
providers:
  - name: my-openai
    type: openai
    api_key: sk-xxx
  - name: chatgpt
    type: openai-chatgpt-plus
```

### New Format
```yaml
providers:
  - name: my-openai
    type: openai
    auth_method: api      # NEW: explicit auth method
  - name: chatgpt
    type: openai          # Same type, different auth
    auth_method: oauth
```

**Migration logic in `config/migration.rs`:**
- `openai-chatgpt-plus` → `openai` with `auth_method: oauth`
- Existing `openai` → `openai` with `auth_method: api`

---

## Verification

1. **Sync verification**: Run sync script, compare generated definitions with OpenCode
2. **Provider count**: `list_provider_types().len() >= 75`
3. **Auth method selection**: UI shows multiple auth options for providers that support them
4. **OAuth flows**: Test GitHub device code and OpenAI PKCE
5. **Migration**: Old configs load correctly after upgrade
6. **Keychain**: Auth credentials stored/retrieved correctly

---

## OpenCode Provider Reference

### Complete Provider List (from OpenCode)

**Core Providers (with bundled SDKs):**
- `anthropic` - API key
- `openai` - API key + OAuth (Codex)
- `google` - API key
- `google-vertex` - GCP project + credentials
- `azure` - API key + region
- `mistral` - API key
- `groq` - API key
- `cerebras` - API key
- `cohere` - API key
- `xai` - API key
- `deepinfra` - API key
- `togetherai` - API key
- `perplexity` - API key
- `openrouter` - API key
- `vercel` - API key
- `amazon-bedrock` - API key OR AWS credentials
- `github-copilot` - OAuth (device code/PKCE)

**Additional Providers (OpenAI-compatible):**
- `moonshotai`, `moonshotai-cn` - Kimi
- `alibaba`, `alibaba-cn` - Alibaba
- `deepseek` - DeepSeek
- `llama` - Llama models
- `ollama-cloud` - Ollama cloud
- `lmstudio` - LM Studio
- `minimax`, `minimax-cn` - MiniMax
- `zhipuai` - Zhipu AI
- `fireworks-ai` - Fireworks AI
- `huggingface` - Hugging Face
- `baseten` - Baseten
- `nebius` - Nebius
- `nvidia` - NVIDIA AI
- `scaleway` - Scaleway
- `ovhcloud` - OVH Cloud
- `vultr` - Vultr
- `upstage` - Upstage
- `siliconflow` - Silicon Flow
- `github-models` - GitHub Models
- `gitlab` - GitLab Duo
- ... and more (75+ total)

### Auth Method Types (from OpenCode)

1. **API Key** (`type: "api"`)
   - User enters key manually
   - Or detected from environment variable

2. **OAuth** (`type: "oauth"`)
   - Device code flow (GitHub Copilot)
   - PKCE flow (OpenAI Codex)
   - Authorization code flow

3. **AWS Credentials** (Bedrock)
   - Bearer token
   - Access key + secret
   - IAM roles
   - Web identity tokens

4. **Well-Known** (custom providers)
   - Discovery via well-known endpoint
