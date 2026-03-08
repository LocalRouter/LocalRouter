# Prompt Compression Feature Plan

## Context

Long conversations accumulate tokens, increasing cost and latency. This adds a per-client prompt compression step to the **OpenAI-compatible proxy** (`/v1/chat/completions`) that reduces input tokens before they reach the target LLM. The MCP unified gateway already has compression via ContextMode — this feature covers the LLM proxy path.

**Chosen approach**: LLMLingua-2 via `@atjsh/llmlingua-2` npm package (global install), following the ContextMode spawn pattern. LLMLingua-2 is extractive (keeps exact original tokens, cannot hallucinate) and fast (single encoder pass, not autoregressive generation).

**Parallel execution**: When compression + guardrails + strong/weak routing are all enabled, all three spawn as parallel tokio tasks. Once all complete, the compressed request goes to the model selected by RouteLLM.

---

## Research Summary

### Why LLMLingua-2 (not LLM summarization)

| | LLMLingua-2 | LLM Summarization |
|--|-------------|-------------------|
| **Method** | Token classification (keep/discard per token) | Abstractive text generation |
| **Hallucination** | Impossible (extractive, keeps original tokens) | Possible (generates new text) |
| **Compression** | 5-14x (benchmarked) | 3-10x (variable) |
| **Speed** | Single encoder pass, 3-6x faster than LLMLingua-1 | Autoregressive, slowest |
| **Quality** | F1: 86.92 (faithful) | Unpredictable |
| **Runs on Ollama?** | No (encoder model, token classifier) | Yes |

LLMLingua-2 **cannot** run as a provider model — it's a BERT/XLM-RoBERTa encoder doing token classification, not a generative model. Must be integrated via npm/Node.js.

### @atjsh/llmlingua-2 npm package

- Pure JavaScript, uses `@huggingface/transformers` (Transformers.js v3) → ONNX Runtime
- Models downloaded from HuggingFace on first use (not bundled)
- Available model sizes:
  - **TinyBERT**: 57 MB (fastest, least accurate)
  - **MobileBERT**: 99 MB (good balance)
  - **BERT-base**: 710 MB (Microsoft's default)
  - **XLM-RoBERTa-large**: 2.2 GB (best quality, multilingual)
- API: `compress_prompt(text, { rate, target_token })` returns compressed text
- No built-in server — needs a JSON-RPC wrapper (like context-mode pattern)
- Single maintainer, marked experimental

---

## Architecture

### Overview

```
npm package: localrouter-compression (new, wraps @atjsh/llmlingua-2)
  ↕ STDIO JSON-RPC (same protocol as context-mode)
Rust: lr-compression crate (new, manages process lifecycle)
  ↕ called from
Rust: lr-server chat.rs (spawn compression task in parallel with guardrails)
```

### 1. Node.js Wrapper Package: `localrouter-compression`

A small npm package that wraps `@atjsh/llmlingua-2` with a STDIO JSON-RPC MCP-compatible interface. Same pattern as the `context-mode` package.

**Exposed tools via MCP protocol:**
- `compress_prompt` — Compress a single text string
  - params: `{ text: string, rate?: number, target_token?: number }`
  - returns: `{ compressed_text: string, original_tokens: number, compressed_tokens: number }`
- `compress_messages` — Compress an array of chat messages (high-level)
  - params: `{ messages: [{role, content}], preserve_recent?: number, compress_system?: boolean, rate?: number }`
  - returns: `{ compressed_messages: [{role, content}], stats: { original_tokens, compressed_tokens, ratio } }`

**Configuration via init params:**
- `model_size`: "tiny" | "mobile" | "bert" | "xlm-roberta" (default: "mobile")

**Lifecycle:**
- Long-lived process (spawned once, reused for all compression requests)
- Lazy model loading on first compress call (ONNX model downloaded + cached by Transformers.js)
- MCP protocol: `initialize` → `tools/list` → `tools/call`

**Location:** `packages/localrouter-compression/` in the repo (or separate npm package)

### 2. Rust Crate: `crates/lr-compression/`

Manages the Node.js compression process, following the ContextMode pattern from `crates/lr-mcp/src/gateway/context_mode.rs`.

```
crates/lr-compression/
├── Cargo.toml
└── src/
    ├── lib.rs           # Re-exports
    ├── engine.rs        # CompressionEngine: spawn, compress, lifecycle
    └── types.rs         # CompressionResult, config types
```

**CompressionEngine:**
- Owns a `StdioTransport` to the Node.js process (reuse from `crates/lr-mcp/src/transport/stdio.rs`)
- 3-tier spawn (adapted from context_mode.rs `spawn_context_mode_process()`):
  1. Check `which localrouter-compression` (global install — fastest)
  2. Try `npx --no-install localrouter-compression` (cached)
  3. Run `npm install -g localrouter-compression` then spawn directly
- Lazy init on first compression request
- `compress_messages()` sends `tools/call` with `compress_messages` tool
- Returns `CompressionResult { compressed_messages, original_tokens, compressed_tokens, ratio }`

**Key reuse:** `StdioTransport` from `crates/lr-mcp/src/transport/stdio.rs` — same spawn + JSON-RPC correlation logic used by ContextMode.

### 3. Config Types

Add to `crates/lr-config/src/types.rs`:

**Global** (in `AppConfig`, after `context_management` field at ~line 424):
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptCompressionConfig {
    /// Enable prompt compression globally (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Model size for LLMLingua-2: "tiny" (57MB), "mobile" (99MB), "bert" (710MB), "xlm-roberta" (2.2GB)
    #[serde(default = "default_compression_model_size")]
    pub model_size: String,  // default: "mobile"
    /// Run compression in parallel with guardrails (default: true)
    #[serde(default = "default_true")]
    pub parallel_compression: bool,
    /// Default compression rate (0.0-1.0, lower = more compression, default: 0.5)
    #[serde(default = "default_compression_rate")]
    pub default_rate: f32,
    /// Compress system prompts too (default: false)
    #[serde(default)]
    pub compress_system_prompt: bool,
}
```

**Per-client** (in `Client`, after `guardrails` field at line 1978):
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientPromptCompressionConfig {
    /// Enable compression for this client (None=inherit global, Some(bool)=override)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Minimum messages before compression activates (default: 6)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_messages: Option<u32>,
    /// Keep last N messages uncompressed (default: 4)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_recent: Option<u32>,
    /// Compression rate override (0.0-1.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
    /// Override global compress_system_prompt setting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compress_system_prompt: Option<bool>,
}
```

### 4. Pipeline Integration

In `crates/lr-server/src/routes/chat.rs` at line ~206:

```
Request arrives
  ↓
[Auth, validate, model firewall]         (existing, lines 54-204)
  ↓
[Spawn guardrail scan]                   (existing, lines 206-214)
[Spawn compression task]                 (NEW — parallel with guardrails)
[Check rate limits]                      (existing, line 217 — parallel with both)
  ↓
[Await compression result]               (NEW — must complete before convert)
[Apply compressed messages to request]   (NEW — replace request.messages)
  ↓
[Convert to provider format]             (existing, line 220)
  ↓
[Guardrails + LLM parallel/sequential]   (existing, lines 222-275)
```

**Critical details:**
- Guardrails scan runs on **original** uncompressed messages (safety must see full content)
- Compression runs on original messages, produces compressed replacement
- Compressed messages used only for the target LLM call
- tool_call and tool result messages are **always preserved** (structured data)
- If compression fails or is disabled, original messages pass through unchanged
- Only applies to `/v1/chat/completions`, not `/v1/completions` (single prompt, not multi-turn)

### 5. Frontend UI

#### New Sidebar Page: "Compression" (under LLM group)

Add `compression` to the sidebar under the **LLM** group heading, after "Strong/Weak" in `src/components/layout/sidebar.tsx`:

```
LLM group:
  - Providers (resources)
  - GuardRails (guardrails)
  - Strong/Weak (strong-weak)
  - Compression (compression)   ← NEW
```

**Steps:**
1. Add `'compression'` to the `View` type union (sidebar.tsx line 47-48)
2. Add nav entry to `resourceNavEntries` under LLM heading (sidebar.tsx ~line 77-80)
3. Import + add case in `renderView()` in `App.tsx` (line ~244-323)
4. Add keyboard shortcut (e.g., `⌘9`)

#### Compression Page: `src/views/compression/index.tsx`

Three tabs: **Info**, **Try it out**, **Settings**

**Info tab** (`src/views/compression/info-tab.tsx`):
- Explanation of what LLMLingua-2 compression does (extractive, no hallucination)
- Model sizes table (Tiny/Mobile/BERT/XLM-RoBERTa with sizes)
- Status indicator: is the compression service running? Model loaded?
- Compression stats: total requests compressed, average ratio, tokens saved

**Try it out tab** (`src/views/compression/try-it-out-tab.tsx`):
- Text input area for a sample prompt/conversation
- Compression rate slider
- "Compress" button
- Side-by-side display: original vs compressed, with token counts and ratio
- Model size selector for quick testing

**Settings tab** (`src/views/compression/settings-tab.tsx`):
- Enable/disable toggle (global)
- Model size dropdown (Tiny 57MB / Mobile 99MB / BERT 710MB / XLM-RoBERTa 2.2GB)
- Default compression rate slider (0.1-0.9)
- Compress system prompt toggle
- Parallel compression toggle
- Min messages threshold (global default)
- Preserve recent count (global default)

#### Per-Client Tab: in `src/views/clients/client-detail.tsx`

Add "Compression" tab to the LLM tab group (alongside Providers, GuardRails):
- Enable toggle (inherit / on / off tri-state)
- Min messages threshold override
- Preserve recent count override
- Rate override slider
- Compress system prompt override
- Link to global Compression page for model/service config

---

## Files to Create

| File | Purpose |
|------|---------|
| `packages/localrouter-compression/package.json` | npm package definition |
| `packages/localrouter-compression/index.js` | STDIO JSON-RPC MCP server wrapping @atjsh/llmlingua-2 |
| `crates/lr-compression/Cargo.toml` | New Rust crate |
| `crates/lr-compression/src/lib.rs` | Public API |
| `crates/lr-compression/src/engine.rs` | Process management, compress_messages() |
| `crates/lr-compression/src/types.rs` | CompressionResult types |
| `src/views/compression/index.tsx` | Main compression page (3 tabs) |
| `src/views/compression/info-tab.tsx` | Info tab with status & stats |
| `src/views/compression/try-it-out-tab.tsx` | Interactive compression testing |
| `src/views/compression/settings-tab.tsx` | Global compression settings |
| `src/views/clients/tabs/compression-tab.tsx` | Per-client compression config |

## Files to Modify

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add `lr-compression` member |
| `crates/lr-config/src/types.rs` | Add `PromptCompressionConfig`, `ClientPromptCompressionConfig`; add fields to `AppConfig` + `Client` |
| `crates/lr-server/Cargo.toml` | Add `lr-compression` dep |
| `crates/lr-server/src/routes/chat.rs` | Spawn compression task parallel with guardrails, await + apply result |
| `crates/lr-server/src/state.rs` | Add `compression_engine: Option<Arc<CompressionEngine>>` to `AppState` |
| `src-tauri/Cargo.toml` | Add `lr-compression` dep |
| `src-tauri/src/ui/commands.rs` | Tauri commands: `get_compression_config`, `update_compression_config`, `test_compression`, `get_compression_status` |
| `src/types/tauri-commands.ts` | TypeScript types for compression config + status + test results |
| `src/components/layout/sidebar.tsx` | Add `'compression'` to View type + nav entry under LLM group |
| `src/App.tsx` | Import CompressionView + add case in renderView() |
| `src/views/clients/client-detail.tsx` | Add compression tab to LLM tab group |
| `website/src/components/demo/TauriMockSetup.ts` | Mock handlers for compression commands |

## Key Patterns to Reuse

| Pattern | Source | Reuse For |
|---------|--------|-----------|
| 3-tier process spawn | `crates/lr-mcp/src/gateway/context_mode.rs:277-344` | Spawning localrouter-compression |
| StdioTransport + JSON-RPC | `crates/lr-mcp/src/transport/stdio.rs` | Communication with Node.js process |
| shell_env() | `crates/lr-mcp/src/manager.rs` | Finding npm/node in PATH |
| Parallel task spawn | `crates/lr-server/src/routes/chat.rs:206-214` | Spawning compression alongside guardrails |
| Per-client config pattern | `ClientGuardrailsConfig` at types.rs:1684 | ClientPromptCompressionConfig shape |
| Global config pattern | `GuardrailsConfig` at types.rs:1632 | PromptCompressionConfig shape |

---

## Verification

1. **Unit tests** in `crates/lr-compression/`: message splitting (system/compressible/preserved), min_messages threshold, tool message preservation
2. **Integration test** in `src-tauri/tests/`: mock Node.js process returning canned compression, verify full pipeline
3. **Manual test**:
   - `npm install -g localrouter-compression`
   - Enable compression in settings (model_size: "tiny" for fast testing)
   - Create a client with compression enabled, min_messages: 4
   - Send a long conversation via `/v1/chat/completions`
   - Verify: compressed messages sent to target LLM, original messages checked by guardrails
4. **Parallel test**: Enable guardrails + compression + auto-routing, verify all three complete before response
5. Run `cargo test && cargo clippy && npx tsc --noEmit`
