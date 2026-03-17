# Unified Streaming JSON Repair + Schema Coercion

## Context

Replace the `jsonrepair` crate dependency with our own streaming JSON repair engine that handles **both syntax repair AND schema coercion in a single pass**. The current implementation has two stages (syntax repair via jsonrepair, then schema coercion via serde_json tree walk). The new design unifies these into one character-at-a-time state machine that minimizes buffering and state.

## Design: Fix Catalog + Buffering Requirements

Starting from what we can fix, sorted by how much state each needs:

### Zero buffer (emit character-by-character)
| Fix | How | State needed |
|-----|-----|-------------|
| Single quotes → double quotes | Replace `'` with `"` when it's a quote delimiter | `in_string` flag |
| Whitespace passthrough | Emit as-is | None |
| `{`, `[`, `]` | Emit immediately | `stack` push/pop |

### 1-bit buffer (boolean flag)
| Fix | How | State needed |
|-----|-----|-------------|
| Trailing commas | Hold `,` back; drop if next significant char is `}`/`]`; emit otherwise | `comma_pending: bool` |

### Short buffer (~20 chars max)
| Fix | How | State needed |
|-----|-----|-------------|
| Unquoted keys | Buffer identifier, emit as `"key"` when `:` follows | `buf: String` |
| Number fixing | Buffer number, fix leading dot/trailing dot/incomplete exponent | `buf: String` |
| Python/JS keywords | Buffer keyword, normalize `True`→`true`, `undefined`→`null`, etc. | `buf: String` |

### Value-level buffer (one JSON string/number)
| Fix | How | State needed |
|-----|-----|-------------|
| Type coercion | Buffer complete value, check against `value_schema.type`, coerce, emit | `buf: String` + `value_schema` |
| Enum normalization | Buffer complete string, case-insensitive match against `enum`, emit correct casing | `buf: String` + `value_schema` |

### Depth counter (no memory buffer)
| Fix | How | State needed |
|-----|-----|-------------|
| Extra field removal | When key not in schema + `additionalProperties:false`, skip key+value; track nesting depth to know when value ends | `skip_mode: bool, skip_depth: u32` on frame |
| Missing closing brackets | On EOF/flush, emit closers based on `stack` depth | `stack` |

### Per-object tracking
| Fix | How | State needed |
|-----|-----|-------------|
| Missing required defaults | Track seen keys; at `}`, inject `,"key":default` for missing required fields with defaults | `seen_keys: HashSet<String>` on frame |

### Pre-JSON detection (only at start)
| Fix | How | State needed |
|-----|-----|-------------|
| Markdown fence stripping | Buffer until JSON start found; detect ` ```json\n `, skip fence + language tag | `pre_buf: String` (cleared after detection) |
| Prose stripping | Buffer until `{` or `[` found; skip everything before it | Same `pre_buf` |

## State Machine

### Core State

```rust
struct StreamingJsonRepairer {
    // Current parsing state
    phase: Phase,
    token: TokenState,

    // Nesting context with schema info
    stack: Vec<Frame>,

    // Token accumulation buffer (reused across tokens)
    buf: String,           // Current string/number/keyword being accumulated
    in_escape: bool,       // String escape state
    quote_char: u8,        // b'"' or b'\'' for current string

    // Trailing comma handling
    comma_pending: bool,

    // Pre-JSON detection buffer (only used at start, then cleared)
    pre_buf: String,

    // Schema & options
    schema: Option<Value>,
    options: RepairOptions,
}

enum Phase {
    PreJson,     // Looking for JSON start (fence/prose detection)
    Parsing,     // Active JSON parsing
    Done,        // Root value completed
}

enum TokenState {
    Between,       // Between tokens (expecting value, key, comma, closer)
    InString,      // Inside "..." or '...'
    InNumber,      // Inside -?[0-9.eE+-]+
    InKeyword,     // Inside true/false/null/undefined/True/False/None/NaN/Infinity
    InUnquotedKey, // Inside bare_key before ':'
}

struct Frame {
    ctx: ContextType,              // Object or Array
    position: FramePosition,       // Where in the structure we are

    // Schema context (cloned sub-schema for this level)
    schema: Option<Value>,

    // Object-specific
    current_key: Option<String>,   // Key being processed
    value_schema: Option<Value>,   // Schema for current key's value
    seen_keys: Option<HashSet<String>>,  // Track seen keys (only if add_defaults)
    skip_mode: bool,               // Skipping unknown field + value
    skip_depth: u32,               // Nesting depth within skipped value
    had_value: bool,               // Whether at least one key-value pair was emitted
}

enum ContextType { Object, Array }

enum FramePosition {
    // Object positions
    ExpectKeyOrClose,  // After `{` or `,` - expecting key or `}`
    ExpectColon,       // After key - expecting `:`
    ExpectValue,       // After `:` - expecting value
    AfterValue,        // After value - expecting `,` or `}`
    // Array positions
    ExpectItemOrClose, // After `[` or `,` - expecting value or `]`
    AfterItem,         // After value - expecting `,` or `]`
}
```

### Processing Algorithm

```
push(chunk: &str) -> String:
    output = ""
    for c in chunk.chars():
        if phase == PreJson:
            handle_pre_json(c) → may append to output, may transition to Parsing
            continue
        if phase == Done:
            continue  // Ignore trailing content

        match token:
            Between → handle_between(c)
            InString → handle_string_char(c)
            InNumber → handle_number_char(c)
            InKeyword → handle_keyword_char(c)
            InUnquotedKey → handle_unquoted_key_char(c)
    return output

handle_between(c):
    skip whitespace (emit it)
    match c:
        '{' → push Object frame, emit '{'
        '[' → push Array frame, emit '['
        '}' → handle_close_object()
        ']' → handle_close_array()
        '"' or '\'' → start string (quote_char = c, always emit '"')
        '-', '.', digit → start number (buf = c)
        letter → start keyword (buf = c)
        ',' → handle_comma()
        ':' → emit ':'

handle_comma():
    // Don't emit yet - might be trailing
    comma_pending = true

handle_close_object():
    // Drop trailing comma
    comma_pending = false
    // Inject missing defaults if needed
    if add_defaults && frame.seen_keys:
        for each required field with default not in seen_keys:
            emit ',' + '"key":' + default_value
    emit '}'
    pop stack
    set parent frame to AfterValue position

handle_string_char(c):
    if in_escape:
        in_escape = false
        buf.push(c)
    elif c == '\\':
        in_escape = true
        buf.push(c)
    elif c == quote_char:
        // String complete - decide what to do with it
        finish_string()
    else:
        buf.push(c)

finish_string():
    // Determine if this is a key or value based on frame.position
    if frame.position == ExpectKeyOrClose:
        // This is an object key
        emit_pending_comma_if_needed()
        handle_key(buf)
    else:
        // This is a value - apply coercion
        handle_string_value(buf)

handle_key(key: String):
    frame.current_key = key
    // Look up in schema
    if let Some(props) = schema.properties:
        if props.contains(key):
            frame.value_schema = props[key]
            emit '"' + key + '"'
        elif !additional_properties:
            // Unknown field - skip it and its value
            frame.skip_mode = true
            frame.skip_depth = 0
            // Don't emit key
        else:
            emit '"' + key + '"'
    frame.position = ExpectColon

handle_string_value(s: String):
    if frame.skip_mode: return  // Skipping

    // Enum normalization
    if value_schema.enum:
        s = normalize_enum(s, value_schema.enum)

    // Type coercion
    match value_schema.type:
        "integer" → if parseable as int: emit int
        "number" → if parseable as float: emit float
        "boolean" → if "true"/"false"/"1"/"0": emit bool
        _ → emit '"' + escape(s) + '"'

    frame.position = AfterValue

handle_number_char(c):
    if c is digit, '.', 'e', 'E', '+', '-':
        buf.push(c)
    else:
        // Number complete
        finish_number()
        reprocess(c)

finish_number():
    if frame.skip_mode: return

    // Fix number issues
    fix leading dot: ".5" → "0.5"
    fix trailing dot: "1." → "1.0"
    fix incomplete exponent: "1e" → "1"

    // Type coercion
    match value_schema.type:
        "string" → emit '"' + num_str + '"'
        "integer" → emit truncated int
        _ → emit fixed number

    frame.position = AfterValue

finish_keyword():
    if frame.skip_mode: return

    // Normalize
    match buf.to_lowercase():
        "true" | "yes" → emit "true" (or coerce to schema type)
        "false" | "no" → emit "false" (or coerce)
        "null" | "none" | "undefined" → emit "null"
        "nan" | "infinity" | "-infinity" → emit "null"
        _ → emit as quoted string (unrecognized → treat as unquoted string value)

    frame.position = AfterValue

emit_pending_comma_if_needed():
    if comma_pending:
        comma_pending = false
        emit ','

flush() -> String:
    // Stream ended - close any open structures
    output = ""
    // Finish any in-progress token
    if token == InString: buf.push('"'), finish_string()
    if token == InNumber: finish_number()
    if token == InKeyword: finish_keyword()
    // Close open containers
    while stack not empty:
        if top is Object:
            handle missing defaults
            emit '}'
        else:
            emit ']'
        pop
    return output
```

### Pre-JSON Detection

```
handle_pre_json(c):
    pre_buf.push(c)

    // Check for markdown fence
    if pre_buf ends with "```":
        // Skip until newline (language tag)
        set scanning_fence = true
        return

    if scanning_fence && c == '\n':
        // Fence header complete, now look for JSON start
        pre_buf.clear()
        return

    // Check for JSON start
    if c == '{' or c == '[':
        // Found JSON start - everything before was prose/fence
        phase = Parsing
        // Process this char as JSON
        handle_between(c)
        pre_buf.clear()  // Free memory
```

## Files to Modify

| File | Change |
|------|--------|
| `crates/lr-json-repair/Cargo.toml` | Remove `jsonrepair` dependency |
| `crates/lr-json-repair/src/lib.rs` | Update public API to use new unified repairer |
| `crates/lr-json-repair/src/streaming.rs` | **Replace entirely**: new `StreamingJsonRepairer` state machine |
| `crates/lr-json-repair/src/syntax_repair.rs` | **Remove**: no longer needed (merged into streaming) |
| `crates/lr-json-repair/src/schema_coerce.rs` | **Remove**: no longer needed (merged into streaming) |
| `crates/lr-json-repair/src/types.rs` | Keep as-is (RepairResult, RepairOptions, RepairAction) |
| `crates/lr-server/src/routes/chat.rs` | Update `maybe_repair_json_content()` to use new unified API; update streaming handlers to pass schema |

## Public API (new)

```rust
// lib.rs - Unchanged public API, different internals
pub fn repair_content(
    content: &str,
    schema: Option<&Value>,
    options: &RepairOptions,
) -> RepairResult;

// streaming.rs - Enhanced with schema support
pub struct StreamingJsonRepairer { ... }

impl StreamingJsonRepairer {
    pub fn new(schema: Option<Value>, options: RepairOptions) -> Self;
    pub fn push(&mut self, chunk: &str) -> String;
    pub fn finish(&mut self) -> String;
}
```

The non-streaming `repair_content()` just creates a `StreamingJsonRepairer`, pushes the entire content, and calls finish. One implementation for both paths.

## Implementation Order

1. Implement `StreamingJsonRepairer` in `streaming.rs`:
   a. Pre-JSON detection (fence/prose stripping)
   b. Core tokenizer (strings, numbers, keywords, structure chars)
   c. Trailing comma handling
   d. Unquoted keys + single quote conversion
   e. Python/JS keyword normalization
   f. Missing closer injection (flush)
   g. Schema context tracking (stack with schemas)
   h. Type coercion on values
   i. Enum normalization
   j. Extra field skipping
   k. Missing default injection

2. Rewrite `lib.rs` to use `StreamingJsonRepairer` for `repair_content()`

3. Remove `syntax_repair.rs` and `schema_coerce.rs`

4. Remove `jsonrepair` from `Cargo.toml`

5. Update streaming integration in `chat.rs` to pass schema to `StreamingJsonRepairer`

6. Port + adapt all 37 existing tests

## Verification

1. `cargo test -p lr-json-repair` — All tests pass (ported from existing + new streaming schema tests)
2. `cargo clippy -p lr-json-repair` — No warnings
3. `cargo check -p localrouter` — Full build succeeds
4. `npx tsc --noEmit` — TypeScript still valid
5. Manual: test via JSON Repair "Try it out" tab
