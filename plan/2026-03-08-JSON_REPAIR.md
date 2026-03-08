# Automatic JSON Repair

## Context

LLMs frequently produce malformed JSON: trailing commas, unescaped characters, missing closing brackets, markdown wrappers around JSON, and schema non-compliance. OpenRouter recently launched "Response Healing" which fixes JSON syntax errors for non-streaming requests, reducing defects by 80-99%. Our implementation goes beyond OpenRouter by also supporting **streaming responses** and **JSON schema compliance fixing**.

**Two capabilities:**
- **Part A: JSON Syntax Repair** — Fix malformed JSON (trailing commas, unescaped chars, missing brackets, markdown wrappers)
- **Part B: JSON Schema Coercion** — Fix valid JSON that doesn't match the expected schema (type coercion, enum normalization, extra field removal, default insertion)

## Architecture Decisions

1. **New workspace crate `crates/lr-json-repair/`** — Follows existing pattern (`lr-compression`, `lr-guardrails`). Encapsulates all repair/coercion logic.

2. **Post-processing layer in chat.rs, NOT a feature adapter** — JSON repair is a gateway-level concern (like guardrails/compression), not a provider adaptation. Feature adapters validate-and-reject; repair replaces-and-passes.

3. **Crate: `jsonrepair`** for Part A — Has streaming `StreamRepairer` API with `push()`/`flush()`, handles trailing commas, unescaped chars, missing quotes/brackets. Low dependency footprint.

4. **Custom coercion for Part B** — `jsonschema 0.18` already in workspace for validation. Build focused coercion module (~200 lines) rather than pulling in `valico` (old draft v4, heavy).

5. **Streaming strategy for syntax repair (Part A):** Use `jsonrepair`'s `StreamRepairer` with `push()`/`flush()`. Feed content chunks in, get repaired chunks out. The crate handles buffering internally (e.g., holding back a comma until it knows if it's trailing). Near-zero latency — only buffers ambiguous tokens.

6. **Streaming strategy for schema coercion (Part B):** Custom incremental JSON parser + schema walker (see "Future: Streaming Schema Coercion" section below). Parses JSON token-by-token, walks the schema in parallel, and repairs values inline as they stream through. Buffers at most one primitive value at a time. **This is a separate implementation phase to be done later.**

7. **Auto-activation** — Activates when request uses `response_format: json_object` or `json_schema`, configurable per-client and globally.

## Crate Structure

```
crates/lr-json-repair/
  Cargo.toml          # deps: jsonrepair, serde, serde_json, jsonschema (workspace), tracing
  src/
    lib.rs            # Public API: JsonRepairer::repair_content()
    syntax_repair.rs  # Part A: wraps jsonrepair + pre/post processing
    schema_coerce.rs  # Part B: type coercion, defaults, extra fields, enums
    types.rs          # RepairResult, RepairStats, CoercionResult
    streaming.rs      # StreamingJsonRepairer wrapping StreamRepairer
```

## Config Schema

**Global** (`crates/lr-config/src/types.rs`, after `PromptCompressionConfig` ~line 1828):

```rust
pub struct JsonRepairConfig {
    pub enabled: bool,              // default: true
    pub syntax_repair: bool,        // default: true — fix JSON syntax errors
    pub schema_coercion: bool,      // default: false — coerce values to match schema
    pub strip_extra_fields: bool,   // default: false — remove fields not in schema
    pub add_defaults: bool,         // default: false — insert defaults for missing required fields
    pub normalize_enums: bool,      // default: true — case-insensitive enum matching
}
```

**Per-client** (after `ClientPromptCompressionConfig` ~line 1852):

```rust
pub struct ClientJsonRepairConfig {
    pub enabled: Option<bool>,          // None=inherit
    pub syntax_repair: Option<bool>,
    pub schema_coercion: Option<bool>,
}
```

Add `json_repair: JsonRepairConfig` to `ServerConfig` (~line 428) and `json_repair: ClientJsonRepairConfig` to `ClientConfig` (~line 2072).

## Implementation Steps

### Phase 1: Core Crate + Non-streaming

1. Create `crates/lr-json-repair/` workspace crate with `Cargo.toml`
2. Add to workspace `Cargo.toml` members + dependencies
3. Implement `syntax_repair.rs` — wrap `jsonrepair`, add markdown fence stripping, validate output parses
4. Implement `schema_coerce.rs` — type coercion (`"42"`→`42`), defaults, extra field removal, enum normalization
5. Implement `types.rs` — `RepairResult { original, repaired, was_modified, repairs }`, `CoercionResult`
6. Implement `lib.rs` — public `repair_content(content, schema) -> RepairResult`
7. Add config types to `lr-config/src/types.rs` + defaults + integration into `ServerConfig`/`ClientConfig`
8. Integrate into non-streaming handler in `chat.rs` `build_non_streaming_response()` (~line 1681)
9. Add `lr-json-repair` dependency to `lr-server/Cargo.toml`

### Phase 2: Streaming Syntax Repair

10. Implement `streaming.rs` — `StreamingSyntaxRepairer` wrapping `jsonrepair::StreamRepairer`. API: `push_content(chunk) -> String` (returns repaired chunk), `finish() -> String` (flushes remaining buffered content). The `jsonrepair` crate buffers internally only for ambiguous tokens (e.g., holds back a trailing comma until it sees the next token).
11. Integrate into `handle_streaming()` (~line 1906) — wrap each `delta.content` chunk through the repairer, emit repaired content instead of raw content. On stream end, call `finish()` and emit any remaining buffered content as a final delta.
12. Integrate into `handle_streaming_parallel()` (~line 2238) — same pattern in buffer/flush worker

### Phase 3: Tauri Commands + UI

13. Add Tauri commands: `get_json_repair_config`, `update_json_repair_config`, `get_client_json_repair_config`, `update_client_json_repair_config`, `test_json_repair`
14. Register in `src-tauri/src/main.rs`
15. Add TypeScript types to `src/types/tauri-commands.ts`
16. Add demo mocks to `website/src/components/demo/TauriMockSetup.ts`
17. Create `src/views/json-repair/index.tsx` — Tabs: Info, Try it out, Settings (follows compression view pattern)
18. Create `src/views/clients/tabs/json-repair-tab.tsx` — per-client overrides (follows compression-tab pattern)
19. Add `'json-repair'` to `View` type in `sidebar.tsx` (line 48-49)
20. Add `{ id: 'json-repair', icon: Wrench, label: 'JSON Repair' }` after compression entry in `resourceNavEntries` (line 82)
21. Add case in `App.tsx` switch (~line 313)

### Phase 4: Tests

22. Unit tests in `crates/lr-json-repair/src/` — syntax repair (10+ patterns), schema coercion (per operation), streaming (chunk boundaries), edge cases
23. Integration tests in `src-tauri/tests/unified_api_tests.rs` — full request/response with json_object and json_schema response_format
24. Update `plan/2026-01-14-PROGRESS.md`

## Critical Files

| File | Change |
|------|--------|
| `crates/lr-json-repair/` (new) | New crate with all repair logic |
| `Cargo.toml` | Add workspace member + jsonrepair dep |
| `crates/lr-server/Cargo.toml` | Add lr-json-repair dep |
| `crates/lr-config/src/types.rs` | Add config types (~line 1828, 1852, 428, 2072) |
| `crates/lr-server/src/routes/chat.rs` | Post-processing in handlers (~lines 1681, 1906, 2238) |
| `src-tauri/src/ui/commands.rs` | Tauri commands |
| `src-tauri/src/main.rs` | Command registration |
| `src/types/tauri-commands.ts` | TypeScript types |
| `src/components/layout/sidebar.tsx` | Add view type + nav entry (lines 48-49, 82) |
| `src/App.tsx` | Add view routing (~line 313) |
| `src/views/json-repair/index.tsx` (new) | Global view |
| `src/views/clients/tabs/json-repair-tab.tsx` (new) | Per-client tab |
| `website/src/components/demo/TauriMockSetup.ts` | Demo mocks |

## Existing Code to Reuse

- `crates/lr-providers/src/features/json_mode.rs` — Reference for JSON validation activation logic
- `crates/lr-providers/src/features/structured_outputs.rs` — Reference for schema extraction from request
- `src/views/compression/index.tsx` — UI pattern (tabs, layout, commands)
- `src/views/clients/tabs/compression-tab.tsx` — Per-client override UI pattern
- `jsonschema 0.18` (already in workspace) — Schema validation after coercion

## Future: Streaming Schema Coercion (Separate Implementation)

This section documents the design for streaming JSON schema coercion — repairing JSON to match a schema **inline during streaming**, without buffering the whole response. To be implemented as a follow-up after the core feature ships.

### Approach: Incremental JSON Parser + Schema Walker

A state machine that processes JSON character-by-character, tracks position in both the JSON structure and the corresponding JSON schema, and applies repairs inline.

### State Machine

```
States:
  ObjectStart   → expecting `{`
  ObjectKey     → expecting a key string (look up in schema.properties)
  ObjectColon   → expecting `:`
  ObjectValue   → expecting a value (schema type known from key lookup)
  ObjectNext    → expecting `,` or `}` (check missing required fields at `}`)
  ArrayStart    → expecting `[`
  ArrayValue    → expecting a value (schema from items)
  ArrayNext     → expecting `,` or `]`
  InString      → accumulating string characters
  InNumber      → accumulating number characters
  InLiteral     → accumulating true/false/null
```

### Schema Context Stack

Maintain `Vec<SchemaContext>` to track position in the schema:

```rust
enum SchemaContext {
    Object {
        schema: &Value,           // the object's schema
        seen_keys: HashSet<String>, // track which required keys appeared
        current_key: Option<String>, // key being processed
        value_schema: Option<&Value>, // schema for current key's value
    },
    Array {
        schema: &Value,           // the array's schema
        item_schema: Option<&Value>, // schema.items
    },
}
```

- `{` → push `Object` context, note `required` and `properties` from schema
- key seen → look up `properties[key]`, set `value_schema`
- `[` → push `Array` context, set `item_schema` from `schema.items`
- `}` or `]` → pop context

### Inline Repair Actions

Each repair works on buffered tokens (at most one value), never the whole response:

**1. Type coercion** (buffer: one primitive value)
- When a complete value is accumulated, check against `value_schema.type`:
  - String `"42"` but schema says `integer` → emit `42`
  - Number `42` but schema says `string` → emit `"42"`
  - String `"true"` but schema says `boolean` → emit `true`
- Emit immediately after coercion, no further buffering needed

**2. Extra field removal** (buffer: zero — uses skip counter)
- When key is not in `properties` and `additionalProperties: false`:
  - Set `skip_mode = true, skip_depth = 0`
  - In skip mode: `{`/`[` increments depth, `}`/`]` decrements; depth 0 → exit skip mode
  - Nothing is emitted while skipping — zero memory overhead regardless of value size
  - Also suppress the preceding comma if present

**3. Enum normalization** (buffer: one string value)
- When value is a string and schema has `enum`:
  - Compare case-insensitively against enum values
  - Emit the correctly-cased version

**4. Missing required field injection** (buffer: zero — injected at `}`)
- Track `seen_keys` in each `Object` context
- When `}` is encountered, check `required` fields against `seen_keys`
- For missing fields that have `default` in schema: inject `,"key": default` before `}`
- Fields without defaults: skip (can't invent values)

**5. Trailing comma** (buffer: 1 character)
- When `,` is seen, hold it
- If next non-whitespace is `}` or `]`: omit the comma
- Otherwise: emit the comma then the next token

### Buffering Guarantees

| Repair type | Max buffer size |
|---|---|
| Type coercion | One primitive value (typically <100 bytes) |
| Extra field removal | Zero (skip counter, no buffering) |
| Enum normalization | One string value |
| Missing field injection | Zero (emitted at `}`) |
| Trailing comma | 1 character + whitespace |

**Worst case buffer**: one JSON string/number value. Never buffers objects, arrays, or the whole response.

### Integration

The streaming schema coercer wraps the syntax repairer in a pipeline:

```
LLM chunks → StreamRepairer (syntax) → StreamingSchemaCoercer → client
```

The syntax repairer ensures valid JSON structure first, then the schema coercer fixes semantic issues.

### Potential Crate Dependencies

- `json-event-parser` or custom char-by-char parser for token extraction
- Reuse `jsonschema 0.18` (already in workspace) for schema introspection utilities

### Why This Is Separate

- The syntax repair (Part A) covers 90%+ of real-world LLM JSON failures
- Schema coercion during streaming is significantly more complex
- It can be tested and validated independently
- The non-streaming schema coercion (already in Phase 1) covers the use case for clients that don't need streaming

---

## Verification

1. `cargo test -p lr-json-repair` — Unit tests pass
2. `cargo test` — All tests pass including integration
3. `cargo clippy` — No warnings
4. `npx tsc --noEmit` — TypeScript types valid
5. Manual: Send malformed JSON via Try-it-out tab, verify repair
6. Manual: Send request with `response_format: json_object` through API with a provider that returns trailing commas, verify repair
7. Manual: Test streaming with JSON repair enabled, verify correction chunk appears
