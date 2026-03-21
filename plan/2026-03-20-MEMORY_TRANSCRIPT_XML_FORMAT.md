# Improve Memory Conversation Storage Format

## Context
Memory conversation transcript files use markdown headings (`## User`, `## Assistant`) to delimit turns. When assistant responses contain markdown (especially headings), the structure becomes ambiguous and hard to parse. Additionally, conversations lack sortable timestamps.

## Approach
Switch from markdown headings to XML tags for conversation turns. Use XML comments for conversation boundaries. Add per-exchange timestamps.

### New Format

```
---
client_id: e8dd2d9f-...
session_id: 87286ef5-...
started: 2026-03-20T01:08:39.891052+00:00
---

<!-- conversation 2f7a1e9e 2026-03-20T01:08:39+00:00 -->

<user timestamp="2026-03-20T01:08:39+00:00">
recall a past convo
</user>

<assistant>
I'd be happy to help! Here's some **markdown**:

## Search Results
- Result 1
</assistant>

<user timestamp="2026-03-20T01:09:15+00:00">
tell me more
</user>

<assistant>
Sure! Here are the details...
</assistant>
```

## Files to Modify

### 1. `crates/lr-memory/src/transcript.rs`

- **`append_conversation_header`**: Change from `\n# Conversation {} ({})\n\n` to `\n<!-- conversation {} {} -->\n\n`
- **`append_exchange`**: Add a `timestamp: &str` parameter. Change format to XML tags.
- **`build_transcript`**: Update to match the new format.

### 2. `crates/lr-mcp-via-llm/src/orchestrator.rs` (~line 707)

- Pass `Utc::now().to_rfc3339()` as the timestamp to `append_exchange`

### 3. `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` (~line 735)

- Same change: pass timestamp to `append_exchange`

### 4. `crates/lr-mcp-via-llm/src/manager.rs` (lines 304 and 483)

- Change `chrono::Utc::now().format("%H:%M")` to `chrono::Utc::now().to_rfc3339()`

### 5. `crates/lr-memory/src/tests.rs`

- Update test assertions to match new XML format

## Verification

1. `cargo test -p lr-memory` — transcript tests pass with new format
2. `cargo test` — full test suite passes
3. `cargo clippy` — no warnings
