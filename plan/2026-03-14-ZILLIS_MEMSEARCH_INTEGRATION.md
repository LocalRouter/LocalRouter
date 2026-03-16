# Zillis Memsearch Integration Plan (Revised)

## Context

LocalRouter needs persistent memory so LLMs can recall past conversations. Zillis memsearch provides markdown-first memory with hybrid vector search. This plan uses the **memsearch CLI directly** as the interface, supporting both **MCP via LLM** (auto-capture) and **plain MCP** (explicit save via tool) modes.

**Key constraints:**
1. Shell out to `memsearch` CLI — no Python MCP server wrapper
2. Recall via MCP virtual server tool (`MemoryRecall`) — configurable tool name (like `IndexSearch`/`IndexRead`)
3. Auto-save in MCP via LLM and Both modes — no explicit save tool
4. Model: download ONNX via memsearch, or use Ollama — similar to guardrails pattern
5. Immediate availability: previous session's memories must be searchable when new session starts
6. Per-client memory isolation — each client gets its own memory directory + memsearch index
7. **Per-client enablement only** — memory configured globally but enabled per-client (no global toggle that enables for all)
8. Configurable LLM compaction — user picks provider/model; triggered by session end (3h inactivity / 8h max)
9. Both transcript and summary kept permanently — enables re-compaction
10. One file per session with conversation grouping — avoids index churn
11. **Privacy warning** in UI — conversations are fully recorded when memory is enabled
12. **UI links to memory folder** so user can review stored memories

---

## Architecture

```
Per-Client Memory Directory (~/.localrouter/memory/{client_id}/)
  ├─ .memsearch.toml                    # per-client memsearch config (generated)
  ├─ sessions/                          # WATCHED by memsearch — this is the indexed directory
  │   ├── {session_id}.md               # active/uncompacted transcript
  │   └── {session_id}-summary.md       # compacted summary (replaces transcript in index)
  ├─ archive/                           # NOT watched — permanent raw transcript storage
  │   └── {session_id}.md               # moved here after compaction (for re-compaction)
  └─ .memsearch/                        # memsearch internal state (Milvus Lite DB)
```

### Supported Modes

| Mode (`ClientMode`) | Save mechanism | Conversation detection |
|---------------------|---------------|----------------------|
| **McpViaLlm** | Auto-capture in orchestrator | Each MCP via LLM session = one conversation |
| **Both** | Auto-capture in chat.rs | Message history prefix matching (see below) |

Not supported: `LlmOnly` (no MCP = no `MemoryRecall` tool exposure), `McpOnly` (no conversation content visible — just a storage proxy, not memory).

**Per-client enablement**: Memory must be explicitly enabled per-client via `client.memory_enabled = true`. No global toggle. This ensures users consciously opt in to conversation recording for each client.

### Two-Level Grouping: Sessions and Conversations

```
Session (temporal grouping, per-client)
├── Conversation 1 (10:30 - 10:45)  ← e.g., one MCP via LLM session
│   ├── User: How do I configure rate limiting?
│   └── Assistant: Rate limiting can be configured...
├── Conversation 2 (10:50 - 11:05)  ← e.g., another MCP via LLM session
│   ├── User: What about caching?
│   └── Assistant: ...
└── Session ends after: 3h inactivity OR 8h max duration
```

**Session**: A temporal container grouping all conversations from one client within a time window.
- **Starts**: on first activity from a client (when no active session exists)
- **Ends**: when `last_activity` > `session_inactivity_timeout` (default 3h) OR `started_at` > `max_session_duration` (default 8h)
- One file per session, conversations separated by markdown headers
- On session end → trigger compaction of the entire session file

**Conversation**: A sequence of related exchanges within a session.
- **McpViaLlm**: each MCP via LLM session = one conversation (natural boundary)
- **Both mode**: detected via message history prefix matching (see below)
- New conversation = new `# Conversation` heading appended to the session file

### Conversation Detection for Both Mode

The OpenAI API is stateless — clients send the full message history each time. We detect conversation continuity using **message hash prefix matching**:

```
Request 1: messages = [sys, user1]                    → hashes = [H0, H1]
Request 2: messages = [sys, user1, asst1, user2]      → hashes = [H0, H1, H2, H3]
  → [H0, H1] is a prefix of [H0, H1, H2, H3] → SAME conversation
  → New messages: [asst1, user2]

Request 3: messages = [sys, userX]                    → hashes = [H0, H4]
  → No stored prefix matches → NEW conversation
```

### Session/Conversation Manager

```rust
struct SessionManager {
    /// client_id → active session
    active_sessions: DashMap<String, ActiveSession>,
    config: SessionConfig, // inactivity_timeout: 3h, max_duration: 8h
}

struct ActiveSession {
    session_id: String,
    file_path: PathBuf,
    started_at: Instant,
    last_activity: Instant,
    /// Current conversation for Both-mode (message hash tracking)
    current_conversation: Option<ConversationState>,
    conversation_count: u32,
}

struct ConversationState {
    conversation_id: String,
    message_hashes: Vec<u64>,  // FxHash of each (role, content)
}
```

**On each exchange**:
1. Get active session for client, or create new one
2. Check session bounds: if `last_activity > 3h` or `age > 8h` → close session (trigger compaction), create new
3. Determine if new conversation:
   - McpViaLlm: compare MCP via LLM session_id with last known → different = new conversation
   - Both: message hash prefix matching
4. If new conversation → append `# Conversation N (timestamp)` header to session file
5. Append `## User` / `## Assistant` exchange
6. Update `last_activity`

### File Strategy

One file per session, multiple conversations within:

```markdown
---
client_id: my-client
session_id: s-abc123
started: 2026-03-14T10:30:00Z
---

# Conversation 1 (10:30)

## User
How do I configure rate limiting?

## Assistant
Rate limiting can be configured per-client...

# Conversation 2 (10:50)

## User
What about caching?

## Assistant
...
```

- memsearch watch debounces at 1.5s — rapid appends only trigger one re-index
- SHA-256 dedup means only new/changed chunks get re-embedded
- Conversation headings help memsearch chunk along natural boundaries

### Compaction Lifecycle

Compaction is triggered by **session end** (3h inactivity or 8h max duration). This is the natural boundary where a group of conversations has stabilized.

```
1. Session active: sessions/{id}.md being appended to → memsearch indexes it
2. Session end detected: inactivity > 3h OR age > 8h
3. Compaction runs (if enabled): LLM summarizes session → writes sessions/{id}-summary.md
4. Archive: move sessions/{id}.md → archive/{id}.md (out of watched dir)
5. memsearch: detects deletion of original → drops old chunks
                detects new summary → indexes it
6. If new activity arrives for same client:
   a. Session has ended → create NEW session (not resume old)
   b. If same conversation detected (Both mode, message prefix match):
      - Restore archived session: move archive/{id}.md back to sessions/
      - Delete old summary
      - Append new content → reset session timers
```

**Why session-end, not conversation-end**: Individual conversations within a session are too granular for compaction — they may be only a few exchanges. Sessions (3h/8h) accumulate enough context for meaningful summarization.

**Why keep both transcript + summary**: Raw transcripts are permanently moved to `archive/` (never deleted) so we can:
- Re-compact if new messages arrive for the same conversation after session end
- Re-compact if the user changes their compaction LLM/settings
- Audit what was actually said

### Insights from memsearch Claude Code plugin

The ccplugin (`/Users/matus/Downloads/memsearch-ccplugin-README.md`) has several patterns worth adopting:

**What we can use:**
- **3-layer progressive disclosure**: `memsearch search` → `memsearch expand` (full section) → `memsearch transcript` (raw conversation). Our `MemoryRecall` tool should use search + expand for richer results rather than raw search alone.
- **Daily file format**: Plugin appends to `YYYY-MM-DD.md` files with `## Session HH:MM` / `### HH:MM` headings. Our session files follow a similar structure with `# Conversation N` headers.
- **Summarization via Haiku**: The Stop hook pipes transcript through `claude -p --model haiku` for third-person bullet-point summaries. Our compaction can follow the same pattern (call configured LLM for summarization).
- **Session anchors**: HTML comments `<!-- session:... turn:... -->` embedded in summaries for drill-down. We should include similar metadata in our session files.

**What we can't use** (hooks-based, we have MCP/LLM):
- Claude Code lifecycle hooks (SessionStart, UserPromptSubmit, Stop, SessionEnd) — we don't have these
- `additionalContext` injection — hooks-specific API
- Forked subagent skill (`context: fork`) — our virtual server runs in the gateway, not as a skill

**Our equivalent approaches:**
| ccplugin mechanism | Our equivalent |
|---|---|
| SessionStart hook → start watch | MCP via LLM session creation → start daemon |
| UserPromptSubmit hook → hint | `MemoryRecall` tool in virtual server system instructions |
| Stop hook → summarize last turn | Session monitor → compact on session end |
| SessionEnd hook → stop watch | Daemon lifecycle managed by MemoryService |
| memory-recall skill (fork) | `MemoryRecall` virtual server tool (search + expand) |

---

## Phase 1: Core Infrastructure — `crates/lr-memory/`

### 1.1 New crate

**Create `crates/lr-memory/Cargo.toml`** — deps: `lr-types`, `lr-config`, `serde`, `serde_json`, `tokio`, `tracing`, `chrono`, `dashmap`

**Create `crates/lr-memory/src/lib.rs`** — `MemoryService`:
```rust
pub struct MemoryService {
    pub cli: MemsearchCli,
    pub session_manager: SessionManager,
    config: RwLock<MemoryConfig>,
    /// One daemon per client_id (per-client isolation)
    daemons: DashMap<String, MemsearchDaemon>,
}
```
Methods:
- `new(config) -> Result<Self>` — validates memsearch is installed
- `ensure_client_dir(client_id) -> PathBuf` — creates `memory/{client_id}/{sessions,archive}/` if needed, generates `.memsearch.toml`
- `start_daemon(client_id)` / `stop_daemon(client_id)` — per-client watch daemon (watches `sessions/` dir)
- `stop_all_daemons()` — shutdown all daemons (app exit)
- `search(client_id, query, top_k) -> Vec<SearchResult>` — runs memsearch search scoped to client's `sessions/` dir
- `update_config(config)` — hot-reload
- `start_session_monitor(self: &Arc<Self>)` — background task (every 60s) that checks active sessions for timeout → triggers compaction on ended sessions

**Create `crates/lr-memory/src/cli.rs`** — CLI wrapper:
- `search(dir, query, top_k) -> Vec<SearchResult>` — `memsearch search "{query}" --json-output --top-k {N}` in client's `sessions/` dir
- `expand(dir, chunk_hash) -> Result<String>` — `memsearch expand {chunk_hash}` — returns full markdown section around a chunk (for progressive disclosure)
- `index(dir) -> Result<()>` — `memsearch index {dir}`
- `compact(dir, source, llm_provider) -> Result<()>` — `memsearch compact --source {source} --llm-provider {provider}`
- `check_installed() -> Result<String>` — `memsearch --version`
- `init_config(dir, embedding_config)` — generates `.memsearch.toml` with embedding provider settings
- All via `tokio::process::Command` with timeouts (10s search, 10s expand, 60s index, 120s compact)

**Create `crates/lr-memory/src/daemon.rs`** — Watch daemon lifecycle:
- `start(sessions_dir)` — spawns `memsearch watch {sessions_dir}` as background child (watches only `sessions/`, not `archive/`)
- `stop()` — SIGTERM with timeout, fallback SIGKILL
- `is_running()` — checks child process status
- `Drop` impl kills child process

**Create `crates/lr-memory/src/transcript.rs`** — Session file writer:
- `create_session_file(client_dir, session_id) -> PathBuf` — creates `sessions/{session_id}.md` with YAML frontmatter (client_id, session_id, started timestamp)
- `append_conversation_header(path, conversation_id, timestamp)` — appends `# Conversation {id} ({time})\n\n`
- `append_exchange(path, user_content, assistant_content)` — appends `## User\n{content}\n\n## Assistant\n{content}\n\n` via `OpenOptions::append(true)`
- `restore_from_archive(archive_path, sessions_dir) -> PathBuf` — moves archived session back to sessions/, deletes old summary, returns restored path

**Create `crates/lr-memory/src/session_manager.rs`** — Session and conversation tracking:
```rust
pub struct SessionManager {
    active_sessions: DashMap<String, ActiveSession>,
    config: SessionConfig, // inactivity_timeout: 3h, max_duration: 8h
}
```
- `get_or_create_session(client_id) -> &ActiveSession` — returns active session or creates new one; closes expired sessions (triggers compaction)
- `record_exchange(client_id, conversation_key, user_text, assistant_text)` — determines conversation within session, appends to file
- `detect_conversation_for_both_mode(client_id, messages) -> ConversationState` — message hash prefix matching
- `close_session(client_id)` — finalizes session, triggers compaction
- `start_monitor_task()` — every 60s, check all active sessions for expiry (inactivity > 3h or age > 8h), close expired ones

**Create `crates/lr-memory/src/compaction.rs`** — Session compaction:
- `compact_session(session_path, archive_dir, compaction_config)`:
  1. Read session file from `sessions/{id}.md`
  2. Call `memsearch compact --source {path} --llm-provider {provider}` — produces summary
  3. Write summary to `sessions/{id}-summary.md`
  4. Move original to `archive/{id}.md`
  5. memsearch watch detects: original deleted → drop chunks; summary created → index it

### 1.2 Configuration

**Modify `crates/lr-config/src/types.rs`**:
```rust
/// Global memory configuration. Memory is enabled per-client, not globally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryConfig {
    /// Embedding provider for memsearch indexing
    #[serde(default)]
    pub embedding: MemoryEmbeddingConfig,

    /// Auto-start memsearch watch daemon per client (default: true)
    #[serde(default = "default_true")]
    pub auto_start_daemon: bool,

    /// Number of search results to return (default: 5)
    #[serde(default = "default_memory_top_k")]
    pub search_top_k: usize,

    /// Session inactivity timeout in minutes (default: 180 = 3 hours)
    #[serde(default = "default_session_inactivity_minutes")]
    pub session_inactivity_minutes: u64,

    /// Max session duration in minutes (default: 480 = 8 hours)
    #[serde(default = "default_max_session_minutes")]
    pub max_session_minutes: u64,

    /// Tool name for recall (default: "MemoryRecall")
    /// Follows the same pattern as ContextManagementConfig.search_tool_name
    #[serde(default = "default_memory_recall_tool_name")]
    pub recall_tool_name: String,

    /// Compaction configuration (optional LLM summarization at session end)
    #[serde(default)]
    pub compaction: Option<MemoryCompactionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryEmbeddingConfig {
    /// Built-in ONNX bge-m3 (default, ~558MB auto-download on first use)
    Onnx,
    /// Ollama for embeddings
    Ollama { provider_id: String, model_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCompactionConfig {
    /// Enable compaction at session end (default: false)
    pub enabled: bool,
    /// LLM provider to use for summarization (memsearch --llm-provider value)
    pub llm_provider: String,
    /// Optional: specific model name for the LLM
    pub llm_model: Option<String>,
}

fn default_memory_recall_tool_name() -> String {
    "MemoryRecall".to_string()
}
```

- Add `pub memory: MemoryConfig` to `AppConfig` + `Default` impl
- Add `pub memory_enabled: Option<bool>` to `Client` struct — **per-client only** (no global enablement; `None` = disabled, `Some(true)` = enabled for this client)
- Memory is configured globally (embedding, compaction, thresholds) but must be explicitly enabled per-client

### 1.3 Memory directory

**Modify `crates/lr-utils/src/paths.rs`**: add `memory_dir()` → `config_dir()/memory/`

Per-client structure:
```
~/.localrouter/memory/
└── {client_id}/
    ├── .memsearch.toml
    ├── sessions/              # WATCHED by memsearch watch
    │   ├── abc123.md          # active transcript (appended to)
    │   └── abc123-summary.md  # compacted summary (replaces transcript in index)
    ├── archive/               # NOT watched — raw transcripts for re-compaction
    │   └── abc123.md
    └── .memsearch/
```

### 1.4 Workspace wiring

- Add `"crates/lr-memory"` to root `Cargo.toml` workspace members
- Add `lr-memory` dependency to `lr-mcp-via-llm`, `lr-server`, `lr-mcp` Cargo.toml files

---

## Phase 2: Session Transcript Writing & Compaction

### 2.1 AppState integration

**Modify `crates/lr-server/src/state.rs`**: add `pub memory_service: Arc<RwLock<Option<Arc<lr_memory::MemoryService>>>>` to `AppState`

### 2.2 Session fields (MCP via LLM auto-capture)

**Modify `crates/lr-mcp-via-llm/src/session.rs`**:
- Add `pub transcript_path: Option<PathBuf>` to `McpViaLlmSession`
- Rename `_session_id` → `session_id`, `_client_id` → `client_id` (needed for transcript filenames)

### 2.3 Manager integration (MCP via LLM auto-capture)

**Modify `crates/lr-mcp-via-llm/src/manager.rs`**:

Add field:
```rust
memory_service: Arc<RwLock<Option<Arc<lr_memory::MemoryService>>>>,
```

Add method:
```rust
pub fn set_memory_service(&self, service: Option<Arc<lr_memory::MemoryService>>)
```

**In `get_or_create_session()`** — when creating a new session, if memory service exists:
1. Call `memory_service.ensure_client_dir(client_id)` to ensure per-client dir exists
2. Start per-client daemon if not running: `memory_service.start_daemon(client_id)`
3. Via `session_manager.get_or_create_session(client_id)` get the active session file path
4. Set `session.transcript_path` — the MCP via LLM session_id serves as the conversation key within the session

**No changes to `cleanup_expired_sessions()`** — session lifecycle (3h/8h) is managed by the `SessionManager`'s own monitor task, independently of MCP via LLM session TTL.

### 2.4 Orchestrator transcript writing (MCP via LLM auto-capture)

**Modify `crates/lr-mcp-via-llm/src/orchestrator.rs`**:
- Add `memory_service: Option<Arc<lr_memory::MemoryService>>` parameter to `run_agentic_loop()` and `resume_after_mixed()`
- After final response (where history is stored, ~line 524), fire-and-forget:
  ```rust
  if let Some(ref svc) = memory_service {
      if let Some(path) = session.read().transcript_path.clone() {
          let svc = svc.clone();
          let user_text = /* extract last user message text */;
          let assistant_text = /* extract last assistant message text */;
          tokio::spawn(async move {
              let _ = svc.transcript.append_exchange(&path, &user_text, &assistant_text).await;
              svc.touch_session(&path); // update last_write time for inactivity tracking
          });
      }
  }
  ```

**Modify `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`**: Same pattern for streaming path

### 2.4b Chat pipeline integration (Both mode)

For `ClientMode::Both` clients making regular LLM requests (not intercepted by MCP via LLM), integrate conversation detection and auto-save into `chat.rs`.

**Modify `crates/lr-server/src/routes/chat.rs`**:
- After the MCP via LLM intercept (line ~438-450), but before the parallel/sequential split, for `Both` mode clients:
  ```rust
  // Memory: detect conversation for Both-mode clients (not MCP via LLM)
  let memory_ctx = if client.client_mode == ClientMode::Both {
      if let Some(ref svc) = *state.memory_service.read() {
          if client.memory_enabled.unwrap_or(true) {
              svc.session_manager.detect_conversation_for_both_mode(
                  &client.id, &provider_request.messages
              )
          } else { None }
      } else { None }
  } else { None };
  ```
- In each response handler (`handle_non_streaming_parallel`, `handle_streaming_parallel`, etc.):
  - After getting the LLM response text, fire-and-forget save:
    ```rust
    if let Some(ctx) = memory_ctx {
        let svc = state.memory_service.read().clone();
        tokio::spawn(async move {
            svc.session_manager.record_exchange(
                &ctx.client_id, &ctx.conversation_key,
                &user_text, &assistant_text
            ).await;
        });
    }
    ```
  - For streaming: buffer the final response text from chunks (or extract from the completion signal)

### 2.5 Startup wiring

**Modify `src-tauri/src/main.rs`** (~line 537-610):
```rust
// Initialize memory service (always init; per-client enablement checked at runtime)
let memory_config = config_manager.get().memory.clone();
{
    match lr_memory::MemoryService::new(memory_config.clone()) {
        Ok(service) => {
            let service = Arc::new(service);
            *app_state.memory_service.write() = Some(service.clone());

            // Set on MCP via LLM manager
            app_state.mcp_via_llm_manager.set_memory_service(Some(service.clone()));

            // Start session monitor (checks for expired sessions → triggers compaction)
            service.start_session_monitor();

            // Register _memory virtual server (Phase 3)
            let memory_vs = Arc::new(
                lr_mcp::gateway::virtual_memory::MemoryVirtualServer::new(service)
            );
            app_state.mcp_gateway.register_virtual_server(memory_vs);
            info!("Memory service initialized with memsearch");
        }
        Err(e) => {
            tracing::warn!("Failed to initialize memory service: {}", e);
        }
    }
}
```

---

## Phase 3: Memory Virtual MCP Server (`_memory`)

### 3.1 Virtual server implementation

**Create `crates/lr-mcp/src/gateway/virtual_memory.rs`** — follows `virtual_skills.rs` pattern:

```rust
pub struct MemoryVirtualServer {
    memory_service: Arc<lr_memory::MemoryService>,
}
```

**Single tool**: `MemoryRecall` — available in all MCP modes (McpViaLlm, Both, McpOnly). Auto-save happens transparently in the pipeline (Phase 2), not via explicit tool.

Implement `VirtualMcpServer`:
- `id()` → `"_memory"`
- `display_name()` → `"Memory"`
- `owns_tool(name)` → `name == state.tool_name` (configurable, default `"MemoryRecall"`)
- `is_enabled(client)` → `client.memory_enabled.unwrap_or(false)` — **disabled by default**, must be explicitly enabled per-client
- `list_tools(state)` → single tool with configurable name:
  ```json
  {
    "name": "{config.recall_tool_name}",
    "description": "Search past conversation memories for relevant context. Use when the current conversation would benefit from information discussed in previous sessions.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "query": { "type": "string", "description": "Search query describing what to recall" }
      },
      "required": ["query"]
    }
  }
  ```
- `check_permissions()` → `Handled(Proceed)` — no firewall popup, always allowed when enabled
- `handle_tool_call(state, tool_name, args, client_id, _)`:
  1. Extract `query` from arguments
  2. Ensure client dir + daemon
  3. Call `memory_service.cli.search(client_dir, &query, top_k)` — get initial results
  4. For top results, call `memory_service.cli.expand(chunk_hash)` — get full section context (3-layer progressive disclosure from ccplugin)
  5. Format combined results with sources; return `"No relevant memories found."` if empty
  6. Return `Success(json content)`
- `build_instructions(state)` → system prompt section:
  ```
  You have access to persistent memory from past conversations via the {tool_name} tool.
  Use it when the conversation would benefit from historical context.
  If you have access to a subagent or forked context, prefer using {tool_name}
  within a subagent to avoid polluting the main conversation with search results.
  ```
- `create_session_state(client)` → `MemorySessionState { enabled: client.memory_enabled.unwrap_or(false), tool_name: config.recall_tool_name }`
- `update_session_state(state, client)` → update enabled flag + tool name from config
- `all_tool_names()` → `vec![config.recall_tool_name.clone()]`

Session state:
```rust
#[derive(Clone)]
pub struct MemorySessionState {
    pub enabled: bool,
    pub tool_name: String,  // configurable, default "MemoryRecall"
}
impl VirtualSessionState for MemorySessionState { /* as_any, as_any_mut, clone_box */ }
```

### 3.2 Registration

**Modify `crates/lr-mcp/src/gateway/mod.rs`**: add `pub mod virtual_memory;`

Registration in `main.rs` covered in Phase 2.5.

---

## Phase 4: Model Management & UI

### 4.1 Model management

**ONNX (default)**: memsearch auto-downloads bge-m3 (~558MB) on first use. We trigger this explicitly during setup.

**Ollama**: Reuse existing `pull_provider_model()` to pull `nomic-embed-text`. Generate `.memsearch.toml` pointing to Ollama:
```toml
[embedding]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://localhost:11434"
```

### 4.2 Tauri commands

**Modify `src-tauri/src/ui/commands.rs`**:

```rust
// --- Memory Setup Commands (3-step incremental setup) ---

#[tauri::command]
pub async fn memory_check_python() -> Result<MemorySetupStepResult, String>
// Checks: python3 --version available, pip available
// Returns: { success: bool, version: Option<String>, error: Option<String> }

#[tauri::command]
pub async fn memory_check_memsearch() -> Result<MemorySetupStepResult, String>
// Checks: memsearch --version
// If not installed, attempts: pip install memsearch[onnx] (or memsearch[ollama])
// Returns: { success: bool, version: Option<String>, error: Option<String> }

#[tauri::command]
pub async fn memory_check_model(app_handle: AppHandle) -> Result<(), String>
// For ONNX: runs memsearch search --provider onnx "warmup" to trigger model download
// For Ollama: uses existing pull_provider_model() for nomic-embed-text
// Streams progress via Tauri events: "memory-setup-progress"
// Returns when model is ready

// --- Memory Config/Status Commands ---

#[tauri::command]
pub async fn get_memory_config() -> Result<Value, String>

#[tauri::command]
pub async fn update_memory_config(config_json: String) -> Result<(), String>
// Saves config, restarts daemons if needed, re-generates .memsearch.toml per client

#[tauri::command]
pub async fn get_memory_status() -> Result<MemoryStatus, String>
// Returns: { python_ok, memsearch_installed, memsearch_version, model_ready, daemon_count, active_daemons }

#[tauri::command]
pub async fn memory_reindex(client_id: String) -> Result<(), String>
// Triggers manual memsearch index for client
```

Register in Tauri invoke handler alongside other commands.

### 4.3 Settings UI — 3-step incremental setup

**Create `src/views/settings/memory-tab.tsx`** — following guardrails-tab pattern with a 3-step setup checklist:

```
┌─────────────────────────────────────────────────────┐
│ Memory                                              │
│ Persistent memory for LLM conversations             │
│                                                     │
│ ⚠️ When enabled for a client, full conversations    │
│ are recorded and stored locally. Review stored       │
│ memories at: [📂 Open memory folder]                │
│                                                     │
│ ┌─ Setup ────────────────────────────────────────┐  │
│ │ ✅ Python environment        python 3.11.5     │  │
│ │ ✅ memsearch CLI             v0.1.7            │  │
│ │ ⏳ Embedding model           Downloading 45%   │  │
│ │                                                │  │
│ │              [ Setup ]                         │  │
│ └────────────────────────────────────────────────┘  │
│                                                     │
│ ┌─ Configuration ────────────────────────────────┐  │
│ │ Tool name: [MemoryRecall]                      │  │
│ │                                                │  │
│ │ Embedding provider:                            │  │
│ │ ○ Built-in ONNX (bge-m3, ~558MB)              │  │
│ │ ○ Ollama                                       │  │
│ │   Provider: [dropdown]  Model: [text field]    │  │
│ │                                                │  │
│ │ Session grouping:                              │  │
│ │ Inactivity timeout: [180] min  Max: [480] min  │  │
│ └────────────────────────────────────────────────┘  │
│                                                     │
│ ┌─ Compaction (optional) ────────────────────────┐  │
│ │ [Toggle] LLM-based summarization               │  │
│ │ Provider: [text]  Model: [text]                │  │
│ └────────────────────────────────────────────────┘  │
│                                                     │
│ ┌─ Status ───────────────────────────────────────┐  │
│ │ Active daemons: 2 (claude-code, cursor)        │  │
│ │ [ Reindex All ]                                │  │
│ └────────────────────────────────────────────────┘  │
│                                                     │
│ ℹ️ Memory is enabled per-client in client settings. │
│ No global toggle — each client must opt in.         │
└─────────────────────────────────────────────────────┘
```

**Key UI elements:**
- **Privacy warning** (top): prominent notice that conversations are recorded when memory is enabled
- **"Open memory folder" link**: uses Tauri's `shell.open()` to open `~/.localrouter/memory/` in file manager
- **No global enable toggle**: memory is configured globally but enabled per-client only
- **Per-client toggle**: Add a "Memory" toggle to the client editor (`src/views/clients/`) — mirrors the guardrails per-client toggle pattern
- **Configurable tool name**: text field with default "MemoryRecall" (same pattern as `IndexSearch`/`IndexRead` in context management settings)

**Setup checklist component** — 3 steps shown as a vertical list, each with status icon:
- `CheckCircle2` (green) = done
- `Loader2` (spinning) = in progress
- `XCircle` (red) = error with message
- `Circle` (gray) = not started

**"Setup" button** runs all 3 steps incrementally:
1. Check Python → if missing, show error with install instructions
2. Check/install memsearch → `pip install memsearch[onnx]` (or `[ollama]`)
3. Download model → stream progress via `memory-setup-progress` Tauri event → show Progress bar

Each step only runs if the previous succeeded. Errors stop the flow and display inline. Already-completed steps show green checkmarks and are skipped on re-run.

**Reuse patterns from**:
- `src/components/guardrails/SafetyModelList.tsx` — progress events, Badge status indicators
- `src/components/guardrails/SafetyModelPicker.tsx` — Ollama model pull flow
- `src/components/ui/progress.tsx` — Radix progress bar
- `src/components/ui/Badge.tsx` — status badges (success, destructive, warning, info)
- Icons: `CheckCircle2`, `XCircle`, `Loader2`, `Circle` from lucide-react

**Modify `src/views/settings/index.tsx`**: add Memory tab trigger + content

### 4.4 TypeScript types

**Modify `src/types/tauri-commands.ts`**:
```typescript
export interface MemoryConfig {
  embedding: MemoryEmbeddingConfig
  auto_start_daemon: boolean
  search_top_k: number
  recall_tool_name: string             // default "MemoryRecall"
  session_inactivity_minutes: number   // default 180 (3h)
  max_session_minutes: number          // default 480 (8h)
  compaction: MemoryCompactionConfig | null
}

export type MemoryEmbeddingConfig =
  | { type: "onnx" }
  | { type: "ollama"; provider_id: string; model_name: string }

export interface MemoryCompactionConfig {
  enabled: boolean
  llm_provider: string
  llm_model: string | null
  inactivity_threshold_minutes: number
}

export interface MemorySetupStepResult {
  success: boolean
  version: string | null
  error: string | null
}

export interface MemoryStatus {
  python_ok: boolean
  memsearch_installed: boolean
  memsearch_version: string | null
  model_ready: boolean
  daemon_count: number
  active_daemons: string[]
}

export interface MemorySetupProgress {
  step: "python" | "memsearch" | "model"
  status: string
  completed: number | null
  total: number | null
}

export interface UpdateMemoryConfigParams { configJson: string }
export interface MemoryReindexParams { clientId: string }
```

### 4.5 Demo mocks

**Modify `website/src/components/demo/TauriMockSetup.ts`**: add mock handlers for all memory commands

---

## Critical Files Summary

| File | Action | Purpose |
|------|--------|---------|
| `crates/lr-memory/src/lib.rs` | Create | MemoryService struct, per-client orchestration, inactivity tracking |
| `crates/lr-memory/src/cli.rs` | Create | memsearch CLI wrapper (search, index, compact, version) |
| `crates/lr-memory/src/daemon.rs` | Create | Per-client watch daemon lifecycle |
| `crates/lr-memory/src/transcript.rs` | Create | Session file create, append exchanges/headers, restore from archive |
| `crates/lr-memory/src/session_manager.rs` | Create | Session/conversation tracking, timeout monitor, message hash matching |
| `crates/lr-memory/src/compaction.rs` | Create | Session compaction + archive logic |
| `crates/lr-config/src/types.rs` | Modify | MemoryConfig, MemoryCompactionConfig, Client.memory_enabled |
| `crates/lr-utils/src/paths.rs` | Modify | memory_dir() path helper |
| `crates/lr-mcp-via-llm/src/session.rs` | Modify | transcript_path field, rename _session_id/_client_id |
| `crates/lr-mcp-via-llm/src/manager.rs` | Modify | memory_service field, transcript init on session create |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Modify | Thread memory_service, append exchanges + touch timer |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Modify | Same for streaming path |
| `crates/lr-server/src/routes/chat.rs` | Modify | Conversation detection + auto-save for Both mode |
| `crates/lr-mcp/src/gateway/virtual_memory.rs` | Create | _memory virtual server (MemoryRecall tool) |
| `crates/lr-mcp/src/gateway/mod.rs` | Modify | pub mod virtual_memory |
| `src-tauri/src/main.rs` | Modify | Wire up MemoryService + compaction task + virtual server |
| `src-tauri/src/ui/commands.rs` | Modify | Tauri commands for memory config/status |
| `src/views/settings/memory-tab.tsx` | Create | Settings UI |
| `src/views/settings/index.tsx` | Modify | Add Memory tab |
| `src/types/tauri-commands.ts` | Modify | TypeScript types |
| `website/src/components/demo/TauriMockSetup.ts` | Modify | Demo mocks |

---

## Verification

1. **Phase 1**: `cargo build` compiles with new crate; unit tests for CLI wrapper (mock memsearch binary)
2. **Phase 2a (MCP via LLM)**: `cargo tauri dev` → enable memory → MCP via LLM conversation → verify session `.md` file in `~/.localrouter-dev/memory/{client_id}/sessions/` with conversation headers and exchanges appended incrementally
3. **Phase 2b (Both mode)**: Both-mode client → send chat completions → verify conversation detected + exchanges saved to session file; send follow-up (with previous messages) → verify same conversation section appended to; start unrelated conversation → verify new `# Conversation` header appended
4. **Session grouping**: Multiple conversations within 3h → all in same session file; wait 3h+ → new session file created on next activity
5. **Phase 3 (recall)**: MCP via LLM or Both-mode session → LLM sees `MemoryRecall` tool → search returns results from past sessions scoped to the client
6. **Compaction**: Enable compaction → have session → session expires (3h inactivity) → verify:
   - `sessions/{id}-summary.md` created
   - `sessions/{id}.md` moved to `archive/{id}.md`
   - New session can recall content from summary via `MemoryRecall`
7. **Re-compaction**: In Both mode, send new message matching a compacted conversation → verify:
   - Session restored from `archive/` to `sessions/`
   - Old summary deleted
   - New content appended
   - Session timers reset → re-compacts when session expires again
8. **Phase 4**: Settings UI → 3-step setup (Python/memsearch/model) → config → status
9. **Cross-client isolation**: Two clients with memory enabled → each only sees their own memories
