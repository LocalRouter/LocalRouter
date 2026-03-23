# Memory folder restructure: slug folders, human-readable session filenames

## Context
Three changes to make the memory folder structure human-readable:
1. **Client folders**: Use slugified client name instead of UUID (persisted in config)
2. **Directory rename**: `sessions/` → `active/`
3. **Session filenames**: Replace UUID with `{timestamp}-{content-slug}.md` derived from the first user message

No migration of existing files on disk.

## New folder structure
```
memory/
└── my-awesome-client/                              # slug from client name
    ├── memory.db
    ├── active/                                     # renamed from sessions/
    │   └── 2026-03-22T14-30-00-i-want-to-ask-about-x7k2m.md
    └── archive/
        ├── 2026-03-22T14-30-00-i-want-to-ask-about-x7k2m.md
        └── 2026-03-22T14-30-00-i-want-to-ask-about-x7k2m-summary.md
```

---

## Session UUID analysis — can we drop it?

**Yes.** The session UUID (`session_id`) is fully redundant:

| Usage | Needed? | Details |
|-------|---------|---------|
| DashMap key | No | `client_id` (UUID) is the key, not session_id |
| FTS5 label | No | Label is `session/{file_stem}` — file_stem can be anything |
| File lookup | No | `file_path` on ActiveSession is the true identifier |
| Monitor events | No | Uses first 8 chars for display — content slug is better |
| ConversationContext.session_id | No | Field is never read by any caller |

**The filename IS the identifier.** Code already extracts session_id from `file_path.file_stem()` everywhere.

## User message availability at session creation

At both call sites in `manager.rs` (lines 289, 458), `request: CompletionRequest` is available with `request.messages`. The session is created on the first request, so we have the user's first message.

The `get_or_create_session` function needs a new param: a content hint string (the slugified first user message). It's only used when creating a NEW session, ignored for existing ones.

---

## Change 1: `memory_folder` on Client struct

### `crates/lr-config/src/types.rs`
Add to `Client` struct (after `memory_enabled`):
```rust
/// Folder name for persistent memory storage (slug derived from client name).
/// Persisted so renaming the client doesn't lose memory data.
/// Generated once at creation; never changes.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub memory_folder: Option<String>,
```

Add slugify helper:
```rust
/// Convert a name to a filesystem-safe slug: "This is My Client!" → "this-is-my-client"
pub fn slugify(name: &str) -> String {
    // lowercase, non-alphanumeric → '-', collapse consecutive hyphens, trim
}
```

Add method:
```rust
impl Client {
    /// Memory folder name (slug if set, falls back to UUID for legacy).
    pub fn memory_folder_name(&self) -> &str {
        self.memory_folder.as_deref().unwrap_or(&self.id)
    }
}
```

### `crates/lr-config/src/lib.rs`
- `create_client_with_strategy()`: Generate slug, deduplicate (append `-2`, `-3` etc if taken), set `memory_folder: Some(slug)`
- Need to pass existing folders for dedup check

### `src-tauri/src/ui/commands_clients.rs`
- `clone_client()`: Generate new unique slug for the clone

---

## Change 2: Rename `sessions/` → `active/`

All occurrences of `"sessions"` directory:

| File | Lines |
|------|-------|
| `crates/lr-memory/src/lib.rs` | 122, 496, 534, 856 |
| `crates/lr-mcp-via-llm/src/manager.rs` | 290, 459 |
| `crates/lr-memory/src/tests.rs` | ~15 occurrences |

---

## Change 3: Human-readable session filenames (drop UUID)

### Filename format
```
{YYYY-MM-DDTHH-MM-SS}-{content-slug}-{5-random-chars}.md
```
Example: `2026-03-22T14-30-00-explain-how-auth-middleware-works-x7k2m.md`

- Timestamp: `chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S")`
- Content slug: First ~50 chars of first user message, slugified, truncated at word boundary
- Random suffix: 5 lowercase alphanumeric chars (avoids collisions)

### Summary files
Derived by replacing `.md` with `-summary.md`:
```
2026-03-22T14-30-00-explain-how-auth-middleware-works-summary.md
```

### `crates/lr-memory/src/session_manager.rs`

**`get_or_create_session()`** — add `content_hint: &str` param:
```rust
pub fn get_or_create_session(
    &self,
    client_id: &str,
    sessions_dir: &Path,
    content_hint: &str,       // NEW: slugified first user message
) -> (String, PathBuf, bool) {
    // ... existing session check ...

    // Create new session — no UUID needed
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    let slug = slugify_content(content_hint, 50); // truncate to ~50 chars
    let rand_suffix = generate_random_suffix(5); // 5 lowercase alphanumeric
    let file_stem = if slug.is_empty() {
        format!("{}-{}", timestamp, rand_suffix)
    } else {
        format!("{}-{}-{}", timestamp, slug, rand_suffix)
    };
    let file_path = sessions_dir.join(format!("{}.md", file_stem));
    // ...
}
```

**`ActiveSession`** — drop `session_id` field, add `memory_folder: String`:
```rust
pub struct ActiveSession {
    pub file_path: PathBuf,        // the true identifier
    pub memory_folder: String,     // slug folder name (for session monitor)
    pub started_at: Instant,
    pub last_activity: Instant,
    pub current_conversation_key: Option<String>,
    pub conversation_state: Option<ConversationState>,
    pub conversation_count: u32,
}
```

**`detect_conversation_for_both_mode()`** — add `content_hint` param, pass through to `get_or_create_session`.

**`ConversationContext`** — drop `session_id` field (never read by callers).

### `crates/lr-memory/src/transcript.rs`

**`create_session_file()`** — change to take `file_path: &Path` directly instead of constructing from session_id:
```rust
pub async fn create_session_file(&self, file_path: &Path) -> Result<PathBuf, String> {
    fs::write(file_path, "").await.map_err(...)?;
    Ok(file_path.to_path_buf())
}
```

**`restore_from_archive()`** — takes a file_stem now instead of session_id. Logic stays the same (construct archive/sessions paths from stem).

### `crates/lr-memory/src/compaction.rs`

**`compact_session()`** — currently extracts `session_id` from filename via `trim_end_matches(".md")`. Now the "session_id" is actually the full file_stem (timestamp-slug). Used for:
- `short_id` logging: Take first ~20 chars instead of first 8
- Archive path: `archive_dir.join(format!("{}.md", file_stem))` — works as-is
- Summary path: `archive_dir.join(format!("{}-summary.md", file_stem))` — works as-is

No structural change needed — just variable naming cleanup (rename `session_id` to `file_stem`).

**`recompact_session()`** — takes a file_stem, constructs `{file_stem}.md` and `{file_stem}-summary.md`. Works as-is.

### `crates/lr-memory/src/lib.rs`

**Helper to extract a short display ID** (replaces current `&session_id[..8]` pattern):
```rust
/// Get a short display ID from a file stem for logging.
/// "2026-03-22T14-30-00-explain-auth" → "explain-auth"
/// "87286ef5-abcd-1234" → "87286ef5" (legacy)
fn short_display_id(file_stem: &str) -> &str {
    // If starts with timestamp pattern (YYYY-MM-DDTHH-MM-SS-), return the slug part
    if file_stem.len() > 20 && file_stem.as_bytes()[19] == b'-' {
        &file_stem[20..]
    } else {
        &file_stem[..8.min(file_stem.len())]
    }
}
```

**`ensure_client_dir()`**: `"sessions"` → `"active"`

**`start_session_monitor()`** line 327: Use `session.memory_folder` instead of `client_id` for directory path:
```rust
let client_dir = service.memory_dir.join(&session.memory_folder);
```

**FTS5 label**: Continue using `session/{file_stem}` and `session/{file_stem}-summary` — these are opaque labels.

**Monitor event rel paths**: Use `{memory_folder}/archive/{file_stem}.md` format.

**`get_compaction_stats()`**, **`force_compact()`**, **`recompact_all()`**, **`reindex()`**: Change `"sessions"` → `"active"`. File stem extraction works the same.

**`count_raw_archive_files()`**, **`count_summary_files()`**, **`collect_raw_archive_files()`**: No change needed — they work on file_stems regardless of format.

### `crates/lr-mcp-via-llm/src/manager.rs`

**Both call sites** (lines 289, 458):
- Pass `client.memory_folder_name()` to `ensure_client_dir`
- Change `"sessions"` → `"active"`
- Extract first user message from `request.messages`, slugify it, pass as `content_hint`
- Store `memory_folder` on the MCP-via-LLM session for later use

### `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`

Line 894: `session_id = path.file_stem()` — already works (now returns content-slug stem instead of UUID).
Line 893: `cid` used for `index_transcript` — needs to use memory_folder slug.

### `crates/lr-mcp/src/gateway/virtual_memory.rs`

Line 183-187: Receives `client_id` from gateway. Need to also receive/resolve `memory_folder`. The `call_tool` signature includes `client_id` — need to check if we can get the client config here to resolve the folder.

Actually, this virtual server already has access to the `Client` struct in `create_session_state()` (line 234). But `call_tool()` only receives `client_id: &str`. Two options:
- Store the memory_folder in the session state
- Look up client from config

**Approach**: Store `memory_folder: String` in `MemorySessionState` (set in `create_session_state` from `client.memory_folder_name()`), pass it through to `call_tool`.

### `src-tauri/src/ui/commands.rs`

All Tauri commands that take `client_id` and call memory service: look up client from config to get `memory_folder_name()`. Affected commands:
- `open_client_memory_folder` (line 4947)
- `clear_memory` (line 4938)
- `get_memory_compaction_stats` (line 4985)
- `force_compact_memory` (line 5010)
- `force_recompact_memory` (line 5069)
- `reindex_memory` (line 5127)
- `read_memory_archive_file` (line 5177)
- Memory search/read commands (lines 4858, 4899, 4924)

Add a helper:
```rust
fn resolve_memory_folder(state: &AppState, client_id: &str) -> Result<String, String> {
    let config = state.config.current();
    config.clients.iter()
        .find(|c| c.id == client_id)
        .map(|c| c.memory_folder_name().to_string())
        .ok_or_else(|| format!("Client not found: {}", client_id))
}
```

### Frontend — no changes needed
- Monitor event-detail.tsx: paths are opaque
- sessions-tab.tsx: no filename references
- TauriMockSetup.ts: mock data stays

---

## Content slug helper

```rust
/// Slugify the first user message into a filename-safe string.
/// "What database should we use for auth?" → "what-database-should-we-use-for-auth"
/// Truncates at ~50 chars on a word boundary.
fn slugify_content(text: &str, max_len: usize) -> String {
    let slug: String = text.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    // Collapse hyphens, trim, truncate at word boundary
    let collapsed = collapse_hyphens(&slug);
    truncate_at_word_boundary(&collapsed, max_len)
}
```

---

## Files to modify (complete list)

### Config layer
1. **`crates/lr-config/src/types.rs`** — `memory_folder` field, `slugify()`, `Client::memory_folder_name()`
2. **`crates/lr-config/src/lib.rs`** — Set `memory_folder` in `create_client_with_strategy()`

### Memory crate
3. **`crates/lr-memory/src/session_manager.rs`** — Drop `session_id` from ActiveSession, add `memory_folder` + `content_hint` param, content-slug filename generation
4. **`crates/lr-memory/src/transcript.rs`** — `create_session_file` takes path directly, update `restore_from_archive`
5. **`crates/lr-memory/src/compaction.rs`** — Rename `session_id` vars to `file_stem`, update logging
6. **`crates/lr-memory/src/lib.rs`** — `"sessions"` → `"active"`, `short_display_id()` helper, use `memory_folder` in monitor, update all extraction points
7. **`crates/lr-memory/src/tests.rs`** — Update directory names, filenames, drop session_id assertions

### Callers
8. **`crates/lr-mcp-via-llm/src/manager.rs`** — Pass slug + content_hint, `"sessions"` → `"active"`
9. **`crates/lr-mcp-via-llm/src/orchestrator_stream.rs`** — Use memory_folder for index_transcript
10. **`crates/lr-mcp-via-llm/src/orchestrator.rs`** — Same pattern as orchestrator_stream
11. **`crates/lr-mcp/src/gateway/virtual_memory.rs`** — Store memory_folder in session state
12. **`src-tauri/src/ui/commands.rs`** — `resolve_memory_folder()` helper, update all memory commands
13. **`src-tauri/src/ui/commands_clients.rs`** — Set `memory_folder` on create/clone

---

## Verification
1. `cargo test -p lr-memory` — memory tests pass
2. `cargo test -p lr-config` — config tests pass
3. `cargo clippy` — no warnings
4. `cargo build` — compiles
5. Manual: create client → verify slug folder name; send request with memory → verify `active/` dir with human-readable timestamped filename
