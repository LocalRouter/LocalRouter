# Memory: Route Embedding & Compaction Through LocalRouter

## Context

Currently memsearch runs with `--provider local` (sentence-transformers) which silently downloads ~80MB models and has no connection to LocalRouter's provider infrastructure. The user wants:

1. **Embedding calls** routed through LocalRouter's `/v1/embeddings` endpoint
2. **Compaction LLM calls** routed through LocalRouter's `/v1/chat/completions` endpoint
3. **Model selection** using the same picker pattern as guardrails — choose from existing providers, pull via Ollama, or download directly
4. **A transient internal client** with an in-memory API key (like Try It Out) so memsearch can authenticate

## Architecture

```
memsearch CLI
  --provider openai
  --base-url http://localhost:{port}/v1
  --api-key {transient_memory_secret}
  --model {provider/embedding_model}
       │
       ▼
LocalRouter /v1/embeddings  ──►  Configured Provider  ──►  Embedding Model
LocalRouter /v1/chat/completions  ──►  Provider  ──►  LLM (for compact)
```

**Why this works**: memsearch's `openai` provider uses the OpenAI Python SDK, which respects `base_url` and `api_key`. LocalRouter's embeddings endpoint is fully OpenAI-compatible. Any embedding model from any configured provider works — memsearch auto-detects dimensions from the first call.

## Changes

### 1. Transient Memory Client (Rust backend)

**File: `crates/lr-memory/src/lib.rs`**

Add a `memory_secret` field to `MemoryService`:
```rust
pub struct MemoryService {
    pub cli: MemsearchCli,
    // ...
    /// In-memory bearer token for memsearch to call LocalRouter
    memory_secret: String,
    /// LocalRouter server port
    server_port: u16,
}
```

Generate at construction (same pattern as `internal_test_secret` in `state.rs`):
```rust
memory_secret: format!("lr-memory-{}", uuid::Uuid::new_v4().simple()),
```

The auth middleware (`crates/lr-server/src/middleware/auth_layer.rs`) needs to recognize this token. The simplest approach: store it in `AppState` alongside `internal_test_secret` and check it the same way.

**File: `crates/lr-server/src/state.rs`**

Add field:
```rust
pub memory_secret: Arc<String>,
```

**File: `crates/lr-server/src/middleware/auth_layer.rs`** (or `client_auth.rs`)

Add check alongside the internal test token:
```rust
if bearer_token == state.memory_secret.as_str() {
    // Treat as internal memory client — allow embeddings + chat
}
```

### 2. CLI Args Refactor

**File: `crates/lr-memory/src/cli.rs`**

Replace the `provider: String` field with full connection config:
```rust
pub struct MemsearchCli {
    /// LocalRouter base URL (e.g., "http://localhost:33625/v1")
    pub base_url: String,
    /// Bearer token for LocalRouter auth
    pub api_key: String,
    /// Embedding model (e.g., "openai/text-embedding-3-small" or "ollama/nomic-embed-text")
    pub embedding_model: String,
}
```

All commands pass:
- `--provider openai --base-url {base_url} --api-key {api_key} --model {embedding_model}`
- `--milvus-uri {working_dir}/milvus.db`

For compact, additionally pass:
- `--llm-provider openai --llm-base-url {base_url} --llm-api-key {api_key} --llm-model {compaction_model}`

### 3. Config Simplification

**File: `crates/lr-config/src/types.rs`**

Replace `MemoryEmbeddingConfig` enum with a simple model reference:
```rust
pub struct MemoryConfig {
    // ...
    /// Embedding model for memsearch (e.g., "openai/text-embedding-3-small")
    /// Routed through LocalRouter's /v1/embeddings endpoint.
    #[serde(default)]
    pub embedding_model: Option<String>,

    /// Compaction LLM model (e.g., "anthropic/claude-haiku-4-5-20251001")
    /// Routed through LocalRouter's /v1/chat/completions endpoint.
    #[serde(default)]
    pub compaction_model: Option<String>,
    // Remove: embedding: MemoryEmbeddingConfig
    // Remove: compaction: Option<MemoryCompactionConfig>
}
```

Keep `MemoryEmbeddingConfig` enum for backwards compat deserialization but ignore it at runtime — use `embedding_model` instead. Or use `#[serde(alias)]` to migrate.

### 4. Setup Simplification

**File: `src-tauri/src/ui/commands.rs`**

Setup becomes 2 steps (not 3):
1. **Python + memsearch**: `pip3 install --upgrade memsearch`  (no `[local]` extra needed — we use the `openai` provider)
2. **Verify**: Test embedding call through LocalRouter to confirm the selected model works

No model download step — the embedding model lives on the provider (OpenAI cloud, Ollama local, etc.).

The old step 3 (model download) is only needed if the user chooses Ollama and the model isn't pulled yet. In that case, use the existing `pull_provider_model` command (same as guardrails).

### 5. UI: Model Selection

**File: `src/views/memory/index.tsx`**

The Info tab shows:
- Setup (2 steps: Python, memsearch CLI)
- Embedding model selector (same `useIncrementalModels` live picker, grouped by provider)
- Compaction model selector (same picker, `None` = disabled)

The Settings tab has the full config (tool name, top-k, session timeouts).

Both model selectors follow the same pattern already used for compaction — `SelectGroup` + `SelectLabel` with live models from `useIncrementalModels`.

### 6. Test Commands

**File: `src-tauri/src/ui/commands.rs`**

`memory_test_index` and `memory_test_search` use the `MemsearchCli` with LocalRouter connection:
```rust
let cli = MemsearchCli {
    base_url: format!("http://localhost:{}/v1", port),
    api_key: memory_secret.clone(),
    embedding_model: config.embedding_model.clone().unwrap_or_default(),
};
```

No more `memory_test_cli()` helper with hardcoded provider.

## Critical Files

| File | Change |
|------|--------|
| `crates/lr-memory/src/cli.rs` | Replace `provider` with `base_url`/`api_key`/`embedding_model`, pass to all commands |
| `crates/lr-memory/src/lib.rs` | Add `memory_secret`/`server_port`, generate at startup |
| `crates/lr-config/src/types.rs` | Replace `MemoryEmbeddingConfig` with `embedding_model: Option<String>`, `compaction_model: Option<String>` |
| `crates/lr-server/src/state.rs` | Add `memory_secret` field |
| `crates/lr-server/src/middleware/auth_layer.rs` | Recognize memory secret |
| `src-tauri/src/main.rs` | Pass port + secret to MemoryService |
| `src-tauri/src/ui/commands.rs` | Update setup (2 steps), test commands use LocalRouter |
| `src/views/memory/index.tsx` | Embedding model picker (same as compaction picker) |
| `src/types/tauri-commands.ts` | Update MemoryConfig type |

## Verification

1. **Setup**: Run Setup → installs memsearch (no model download needed)
2. **Config**: Select embedding model (e.g., `ollama/qwen3-embedding:0.6b`) from picker
3. **Try It Out**: Index → memsearch calls LocalRouter `/v1/embeddings` → routes to Ollama → returns embeddings → indexes in Milvus
4. **Try It Out**: Search → same flow, returns results
5. **Try It Out**: Compact → memsearch calls LocalRouter `/v1/chat/completions` for summarization
6. **Live**: MCP via LLM session with memory enabled → transcript written → indexed via LocalRouter → `MemoryRecall` tool returns results
