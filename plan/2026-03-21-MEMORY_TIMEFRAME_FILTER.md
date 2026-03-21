# Add Timeframe Filtering to MemorySearch Tool

## Context

The MemorySearch MCP tool currently has no way to filter results by time. When an LLM wants to answer "what did the user say yesterday?" or "find conversations from last week", it must search all memories and hope the right ones surface by relevance. Adding `after` and `before` parameters enables date-range filtering at the SQL level, so the LLM can precisely scope its search to a time window.

## Tool Schema Changes

Add two new optional parameters to the MemorySearch tool:

```json
"after": {
    "type": "string",
    "description": "Only include memories after this time. Accepts ISO 8601 date (\"2026-03-20\"), datetime (\"2026-03-20T14:00:00Z\"), or relative offset from now (\"2d\" = 2 days ago, \"6h\" = 6 hours ago, \"1w\" = 1 week ago, \"30m\" = 30 minutes ago)."
},
"before": {
    "type": "string",
    "description": "Only include memories before this time. Same formats as 'after'. Example: to search yesterday, use after=\"2026-03-20\" before=\"2026-03-21\"."
}
```

## Implementation

### Step 1: `crates/lr-context/src/types.rs` — Add `DateRange` struct

```rust
#[derive(Debug, Clone, Default)]
pub struct DateRange {
    pub after: String,   // "" = unbounded
    pub before: String,  // "9999-12-31 23:59:59" = unbounded
}
```

With `DateRange::new(after: Option<String>, before: Option<String>)` constructor that fills sentinels for `None` values. Export from `lr-context/src/lib.rs`.

### Step 2: `crates/lr-context/src/search.rs` — Add date filtering to SQL

Add `date_range: &DateRange` param to `search_porter()`, `search_trigram()`, and `search_with_fallback()`.

SQL queries gain two additional WHERE clauses:

```sql
-- With source filter (existing pattern + new date clauses)
WHERE chunks MATCH ?1 AND s.label LIKE ?2 ESCAPE '\'
  AND s.indexed_at > ?3 AND s.indexed_at < ?4
ORDER BY rank LIMIT ?5

-- Without source filter
WHERE chunks MATCH ?1
  AND s.indexed_at > ?2 AND s.indexed_at < ?3
ORDER BY rank LIMIT ?4
```

The sentinels (`""` and `"9999-12-31 23:59:59"`) make the clauses effectively no-ops when no date range is specified, keeping the existing 2-variant structure (with/without source) unchanged.

### Step 3: `crates/lr-context/src/lib.rs` — Thread `DateRange` through ContentStore

Update these methods to accept date range:
- `search()` — add `date_range: &DateRange` param
- `search_combined()` — add `after: Option<&str>, before: Option<&str>` (construct `DateRange` internally)
- `search_internal()` — add `date_range: &DateRange` param
- `batch_search_read()` — add `date_range: &DateRange` param
- `list_sources()` — add `after: Option<&str>, before: Option<&str>` and filter SQL

All existing callers pass `&DateRange::default()` or `None, None`.

### Step 4: `crates/lr-memory/src/lib.rs` — Thread through MemoryService

Update:
- `search_combined()` — add `after: Option<&str>, before: Option<&str>`, pass to store
- `list_sources()` — add `after: Option<&str>, before: Option<&str>`, pass to store

### Step 5: `crates/lr-mcp/src/gateway/virtual_memory.rs` — Tool schema + time parsing

**Time parsing** — add `resolve_time(input: &str) -> Result<String, String>`:
1. Try relative offset: regex `^(\d+)(m|h|d|w)$` → compute `Utc::now() - duration`, format as `"YYYY-MM-DD HH:MM:SS"`
2. Try ISO datetime: parse `YYYY-MM-DDTHH:MM:SS[Z]` → normalize to `"YYYY-MM-DD HH:MM:SS"`
3. Try ISO date: parse `YYYY-MM-DD` → return as-is (string comparison works correctly)
4. Else → return error with format hint

**handle_memory_search** — extract `after`/`before` from arguments, call `resolve_time()` if present, pass resolved strings to `memory_service.search_combined()`.

**build_summary_fallback** — accept and pass `after`/`before` to `list_sources()`.

**list_tools** — update `input_schema` JSON with the two new properties.

**build_instructions** — add note about `after`/`before` in system instructions.

### Step 6: Update remaining callers (pass-through `None`/default)

- `crates/lr-mcp/src/gateway/context_mode.rs:483` — `store.search_combined()` add `None, None`
- `src-tauri/src/ui/commands.rs:2329` — `store.list_sources()` add `None, None`
- `src-tauri/src/ui/commands.rs:2357` — `store.search_combined()` add `None, None`
- `src-tauri/src/ui/commands.rs:4743` — `store.search()` add `&DateRange::default()`
- `src-tauri/src/ui/commands.rs:4830` — `svc.list_sources()` add `None, None`
- `src-tauri/src/ui/commands.rs:4864` — `svc.search_combined()` add `None, None`
- `crates/lr-memory/src/lib.rs` — internal `list_sources` call in `get_compaction_stats` add `None, None`

### Step 7: Tests

- **Time parsing unit tests**: relative offsets (m/h/d/w), ISO dates, ISO datetimes, invalid inputs, edge cases (0d = now)
- **Date-filtered search test** in lr-context: index two sources, manually UPDATE one's `indexed_at` to a past date, verify `after`/`before` filters correctly
- **`list_sources` date filter test**: verify filtered source listing
- **Backward compatibility**: verify `DateRange::default()` returns all results

### Step 8: Mandatory final steps

1. **Plan review**: check all changes against this plan
2. **Test coverage review**: ensure all new code paths are tested
3. **Bug hunt**: check edge cases in time parsing, SQL comparison semantics

## Critical Files

- `crates/lr-context/src/types.rs` — `DateRange` struct
- `crates/lr-context/src/search.rs` — SQL query changes (deepest layer)
- `crates/lr-context/src/lib.rs` — ContentStore method signatures
- `crates/lr-memory/src/lib.rs` — MemoryService pass-through
- `crates/lr-mcp/src/gateway/virtual_memory.rs` — tool schema, time parsing, argument extraction
- `src-tauri/src/ui/commands.rs` — caller updates (pass-through None)

## Key Design Decisions

- **`after`/`before` over single `timeframe`**: two params allow arbitrary windows (e.g., "last Tuesday to Thursday")
- **Filter at SQL level**: ensures LIMIT applies within the date range (post-filtering would miss valid results)
- **Sentinel values**: avoids multiplying SQL query variants; `DateRange::default()` is no-op
- **`indexed_at` column**: already exists in the sources table; reflects when content was indexed (≈ conversation time)
- **`>` and `<` (exclusive bounds)**: bare dates like `"2026-03-20"` sort before `"2026-03-20 00:00:00"`, making `after: "2026-03-20"` correctly include all of March 20th. `before: "2026-03-21"` correctly excludes March 21st.

## Verification

1. `cargo test && cargo clippy && cargo fmt`
2. Run dev mode, enable memory on a client, have a conversation
3. Use MemorySearch with `after: "1h"` — should return only recent memories
4. Use MemorySearch with `after: "2026-03-20" before: "2026-03-21"` — should scope to that day
5. Use MemorySearch without date params — should behave exactly as before
