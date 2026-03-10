<!-- @entry json-healing-abstract -->

**Streaming JSON Response Healing** is a single-pass, character-by-character algorithm that repairs malformed JSON output from LLMs in real-time during streaming. Unlike post-hoc repair approaches that require buffering the complete response, this algorithm operates with O(1) memory relative to response size — buffering only the current token (typically under 100 bytes). It handles both syntax repair (trailing commas, markdown fences, unclosed brackets) and JSON Schema coercion (type casting, enum normalization, extra field removal, default injection) in a unified streaming pass.

<!-- @entry json-healing-problem -->

When LLMs generate JSON output (via `response_format: json_object` or `json_schema`), the output is frequently malformed. Common failure modes include:

- **Markdown wrapping**: Output wrapped in ` ```json ... ``` ` fences
- **Trailing commas**: `{"a": 1, "b": 2,}` — valid JavaScript, invalid JSON
- **Unclosed structures**: Stream terminates before all brackets are closed
- **Python keywords**: `True`, `False`, `None` instead of `true`, `false`, `null`
- **Single quotes**: `{'key': 'value'}` instead of `{"key": "value"}`
- **Unquoted keys**: `{key: "value"}` instead of `{"key": "value"}`
- **Type mismatches**: `"42"` (string) when the schema requires an integer

These failures are particularly problematic in streaming scenarios. Traditional JSON repair libraries operate on complete strings — they cannot process partial chunks as they arrive from the LLM. This forces applications to either buffer the entire response (defeating the purpose of streaming) or abandon structured output during streaming.

**The key insight**: JSON is a context-free grammar that can be parsed incrementally. By maintaining a minimal parsing state (nesting stack + current token buffer), we can repair each character as it arrives without ever needing to see the full response.

<!-- @entry json-healing-algorithm -->

The algorithm is implemented as a streaming state machine with three phases:

**Phase: PreJson** — Detects and strips non-JSON preamble (markdown fences, prose) by scanning for the first `{` or `[` character. Everything before is discarded.

**Phase: Parsing** — The core repair loop, processing one character at a time through a dispatch based on `TokenState`:

```
TokenState::Between     → Expecting a structural character or token start
TokenState::InString    → Accumulating string content (tracking escapes)
TokenState::InNumber    → Accumulating numeric digits, dots, exponents
TokenState::InKeyword   → Accumulating unquoted identifiers (keys or keywords)
```

**Phase: Done** — Stream complete. The `finish()` method closes any open containers by walking the nesting stack.

The nesting context is maintained as a stack of `Frame` objects:

```
Frame {
    kind: Object | Array
    position: ExpectingKey | ExpectingValue | ExpectingComma | ...
    schema: Option<JsonSchema>     // For type coercion
    skip_mode: bool                // For extra field removal
    skip_depth: u32                // Nested skip tracking
    seen_keys: HashSet<String>     // For default injection
}
```

<!-- @entry json-healing-buffering -->

The critical design constraint is **minimal buffering** to preserve streaming behavior. The algorithm achieves this through several techniques:

**Deferred comma emission**: Instead of immediately emitting commas, the algorithm sets a `comma_pending` flag. The comma is only emitted when the next non-whitespace character confirms it's needed. If the next character is `}` or `]`, the comma is suppressed (trailing comma removal). This requires buffering only 1 bit + any intervening whitespace.

**Token-scoped buffering**: Only the current token (string value, number, keyword) is buffered. Structural characters (`{`, `}`, `[`, `]`, `:`) are emitted immediately. The maximum buffer size is bounded by the longest token, not the response size.

**Zero-buffer skip mode**: When removing extra fields (not in the JSON Schema), the algorithm tracks nesting depth with a counter instead of buffering the skipped content. It increments on `{`/`[` and decrements on `}`/`]`, emitting nothing until depth returns to zero.

**Zero-buffer default injection**: When an object closes (`}`), the algorithm checks `seen_keys` against the schema's `required` fields. Missing fields with `default` values are injected directly into the output stream as `,\"key\":default_value` — no buffering of the object content is needed.

| Buffer Component | Purpose | Maximum Size |
|-----------------|---------|:---:|
| `buf` | Current token | ~100 bytes |
| `pending_ws` | Whitespace after deferred comma | Whitespace only |
| `comma_pending` | Trailing comma suppression | 1 bit |
| Stack frames | Nesting context | O(depth) |

<!-- @entry json-healing-schema -->

When a JSON Schema is provided (via `response_format: json_schema`), the algorithm performs **inline type coercion** during the same streaming pass:

**Type coercion** operates on completed tokens. When a string, number, or boolean token finishes, the algorithm checks it against the expected schema type for the current position:

```
"42" (string) + schema{type: "integer"} → 42
42 (number) + schema{type: "string"} → "42"
true (boolean) + schema{type: "integer"} → 1
0 (number) + schema{type: "boolean"} → false
```

**Enum normalization** performs case-insensitive matching against schema-defined enums:

```
"active" + schema{enum: ["Active", "Inactive"]} → "Active"
"PENDING" + schema{enum: ["pending", "active"]} → "pending"
```

**Extra field removal** activates when `additionalProperties: false` and an unrecognized key is encountered. The algorithm enters skip mode with a depth counter, consuming all characters for the field's value (including nested objects/arrays) without emitting any output. The preceding comma is also suppressed.

**Default injection** happens at object close. The algorithm maintains a `seen_keys` set per object frame. When `}` is encountered, any `required` fields not in `seen_keys` that have a `default` value in the schema are injected inline.

<!-- @entry json-healing-streaming-example -->

A concrete example showing how the algorithm processes streaming chunks:

```
Input arrives in 4 chunks:
  Chunk 1: "```json\n{"
  Chunk 2: "\"age\": \"30\",}"
  Chunk 3: "\n```"
  Chunk 4: (stream end → finish() called)

Schema: { "type": "object", "properties": { "age": { "type": "integer" } } }

Processing:
  Chunk 1:
    "```json\n" → PreJson phase, detect fence, strip
    "{"         → Emit "{", push Object frame

  Chunk 2:
    "\"age\""   → String token, buffer "age"
    ":"         → Emit key "age" + ":"
    " \"30\""   → String token "30", but schema says integer
                  → Type coerce: emit 30 (no quotes)
    ","         → Set comma_pending = true
    "}"         → comma_pending + "}" → suppress comma, emit "}"

  Chunk 3:
    "\n```"     → Phase is Done, ignore trailing fence

  Chunk 4:
    finish()    → Stack empty, nothing to close

Output: {"age":30}
Repairs: [StrippedMarkdownFences, TypeCoerced("age", "string", "integer"), SyntaxRepaired]
```

Note how chunk boundaries have no effect on correctness — the same output is produced regardless of how the input is split.

<!-- @entry json-healing-integration -->

The healing algorithm integrates with the chat completions pipeline at two points:

**Streaming path**: When `response_format` is `json_object` or `json_schema`, a `StreamingJsonRepairer` is created and wrapped in `Arc<Mutex<>>` for thread-safe chunk processing. Each SSE chunk's `delta.content` is pushed through the repairer, and the repaired text replaces the original. On stream end (`finish_reason` present), `finish()` is called to close any remaining open structures.

**Non-streaming path**: The complete response content is passed through the non-streaming `repair_content()` function, which internally creates a repairer, pushes all content, and calls finish.

**Configuration** supports global defaults with per-client overrides:

```yaml
json_repair:
  enabled: true
  syntax_repair: true      # Fix JSON syntax errors
  schema_coercion: true    # Coerce types to match schema
  strip_extra_fields: false # Remove non-schema fields
  add_defaults: true       # Inject missing required defaults
  normalize_enums: true    # Case-insensitive enum matching
```

<!-- @entry json-healing-results -->

The algorithm achieves single-pass O(n) time complexity with O(d) space complexity (where d is nesting depth). In practice, the buffer never exceeds a few hundred bytes regardless of response size.

| Property | Value |
|----------|-------|
| Time complexity | O(n) single pass |
| Space complexity | O(d) nesting depth |
| Max buffer size | ~100 bytes (one token) |
| Chunk boundary sensitivity | None |
| External dependencies | Zero (serde_json only) |
| Implementation | 1,803 lines (incl. 60+ tests) |

**Repair coverage** addresses 90%+ of real-world LLM JSON failures:

| Failure Mode | Repair |
|-------------|--------|
| Markdown fences | Strip ` ``` ` and language tag |
| Trailing commas | Suppress via deferred emission |
| Unclosed brackets | Auto-close via stack walk |
| Python keywords | `True`→`true`, `None`→`null` |
| Single quotes | Convert to double quotes |
| Unquoted keys | Quote and normalize |
| Type mismatches | Coerce per schema |
| Extra fields | Skip with depth counter |
| Missing defaults | Inject at object close |
| Leading dots | `.5` → `0.5` |

The zero-dependency, streaming-first design makes this suitable for high-throughput API gateways where buffering complete responses would add unacceptable latency and memory pressure.
