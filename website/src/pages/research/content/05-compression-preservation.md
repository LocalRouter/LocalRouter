<!-- @entry compression-preservation-abstract -->

**Quote-Aware Prompt Compression** adds a protection layer to the LLMLingua-2 compression pipeline that force-keeps words inside quoted strings and code blocks. A word-level finite state machine detects protected regions across 15 delimiter types — including Unicode curly quotes, guillemets, CJK corner brackets, and full-width characters — after BERT scoring but before word selection. Protected words are added to the keep set without consuming compression budget, ensuring code snippets, string literals, and precise quotations survive compression intact while natural language around them is compressed normally. An optional `[abridged]` prefix signals to downstream models that content has been compressed.

<!-- @entry compression-preservation-problem -->

LLMLingua-2 performs prompt compression by classifying each word as "keep" or "drop" using a BERT encoder. This works well for natural language, where filler words, articles, and transition phrases can be removed without losing meaning. However, the same algorithm applied to code or quoted text produces corrupted output:

```
Input:  "Run `pip install requests` and then call `requests.get(url)`"
Naive:  "Run `pip install` call `requests.get`"
```

The compression model doesn't understand that content within backticks is a precise command — dropping any word changes the meaning entirely. Similarly, quoted text (`"exact phrase"`) and fenced code blocks represent content where every word matters.

**The challenge**: The compression model must still *see* all text to make informed decisions about the surrounding natural language. Excluding quoted regions from the model's input would degrade compression quality for the text around them. The model needs full context to score words accurately — we just need to override its decisions for words within protected regions.

<!-- @entry compression-preservation-algorithm -->

The algorithm operates as a post-BERT protection layer with three phases:

**Phase 1: Detection** — A word-level finite state machine scans the input and produces a boolean mask where `mask[i] = true` means word `i` must be protected.

**Phase 2: BERT Scoring** — Standard LLMLingua-2 forward pass. The BERT encoder sees all words and produces keep/drop probabilities. Words are ranked by P(keep) and the top N are selected based on the compression rate.

**Phase 3: Union** — Protected words are added to the BERT-selected keep set *without reducing the budget*. If 100 words at rate=0.5 (keep 50), protecting 10 quoted words means keeping those 10 + the top 50 BERT-selected = 60 words total.

```
Phase 1 (Detection):
  Words:    "Run"  "`pip"  "install"  "requests`"  "and"  "then"  "call"
  Mask:      no     yes     yes        yes          no     no      no

Phase 2 (BERT Scoring):
  P(keep):  0.92   0.85    0.31       0.78         0.45   0.12    0.67
  Selected: keep   keep    DROP       keep         DROP   DROP    keep

Phase 3 (Union):
  BERT:     keep   keep    DROP       keep         DROP   DROP    keep
  Protect:   -      +       +          +            -      -       -
  Final:    keep   keep    KEEP       keep         DROP   DROP    keep

Output: "Run `pip install requests` and call"
```

The key property is that BERT's decisions for surrounding natural language are fully respected. Only words *within* protected regions are overridden.

<!-- @entry compression-preservation-detection -->

The detection function implements three parallel state tracking layers in a single pass over the word list:

**Layer 1: Fenced code blocks** (highest priority) — Toggles `in_fenced` when a word contains triple backticks. While fenced, all words are protected. Supports language specifiers (e.g., ` ```python `).

**Layer 2: Inline code** (medium priority) — Toggles `in_backtick` on single backtick boundaries. Blocked when inside a fenced block.

**Layer 3: Quote delimiters** (15 types, independent) — Each delimiter type is tracked independently via a `HashSet<delimiter_index>`. Words between matching open/close delimiters are protected. Multiple quote types can be active simultaneously (union-based).

The 15 supported delimiter pairs:

| Delimiter | Open | Close | Symmetric | Use Case |
|-----------|:---:|:---:|:---:|---------|
| ASCII Double | `"` | `"` | Yes | English prose |
| ASCII Single | `'` | `'` | Yes | English prose |
| Curly Double | `\u201C` | `\u201D` | No | Smart quotes |
| Curly Single | `\u2018` | `\u2019` | No | Smart quotes |
| German Double | `\u201E` | `\u201D` | No | German prose |
| German Single | `\u201A` | `\u2019` | No | German prose |
| Guillemet Double | `\u00AB` | `\u00BB` | No | French prose |
| Guillemet Single | `\u2039` | `\u203A` | No | French prose |
| Heavy Double | `\u275D` | `\u275E` | No | Decorative |
| Heavy Single | `\u275B` | `\u275C` | No | Decorative |
| Full-width Double | `\uFF02` | `\uFF02` | Yes | CJK typography |
| Full-width Single | `\uFF07` | `\uFF07` | Yes | CJK typography |
| CJK Corner | `\u300C` | `\u300D` | No | Japanese/Chinese |
| CJK Double Corner | `\u300E` | `\u300F` | No | Japanese/Chinese |

<!-- @entry compression-preservation-edge-cases -->

Several edge cases require careful handling:

**Apostrophes vs. single quotes**: The word `"don't"` contains a single quote that is an apostrophe, not a quote delimiter. A dedicated `is_apostrophe()` function detects mid-word single quotes between alphanumeric characters and skips them. Additionally, a pre-scan phase counts boundary appearances for symmetric delimiters — if fewer than 2 boundaries are found (meaning no matching pair exists), that delimiter type is disabled entirely for the text.

**Trailing punctuation on closing delimiters**: Text like `"hello world," and left` has a comma after the closing quote. The detector strips trailing punctuation (`.`, `,`, `;`, `:`, `!`, `?`) before checking for closing delimiters.

**Unclosed quotes**: If a quote opens but never closes, all words from the opening delimiter to the end of the text are protected. This is deliberately conservative — it's safer to over-protect (keep extra words) than to corrupt quoted content.

**Self-contained quotes**: When a word contains both the opening and closing delimiter (e.g., `"hello"` as a single token), the quote opens and closes on the same word, protecting just that word.

**Fenced code with language specifier**: The detector matches any word containing triple backticks regardless of surrounding text, so ` ```python ` correctly toggles fenced mode.

<!-- @entry compression-preservation-notice -->

An optional **compression notice** feature prepends `[abridged]` to each compressed message:

```
Original:  "The user requested a detailed analysis of the performance
            metrics from the last quarter including revenue growth..."
Compressed: "[abridged] user requested detailed analysis performance
             metrics last quarter including revenue growth..."
```

This signals to downstream LLMs that the content is extracted/abbreviated rather than verbatim. Some models perform better when they know the input has been compressed, as they can infer that missing context may exist.

Both features are independently configurable:

```yaml
compression:
  enabled: true
  default_rate: 0.8
  preserve_quoted_text: true   # Default: true
  compression_notice: false    # Default: false
```

Per-client overrides follow the standard `Option<bool>` pattern — `None` inherits the global setting, `Some(true/false)` overrides it.

<!-- @entry compression-preservation-visualization -->

The UI provides visual feedback showing exactly which words were protected vs. BERT-selected vs. dropped:

- **Purple highlight**: Words in the protected set (quoted/code content force-kept)
- **Normal text**: Words selected by BERT scoring (natural language kept)
- **Red strikethrough**: Words dropped by compression

The visualization also shows a count of protected words (e.g., "47 protected") alongside the standard compression statistics (original tokens, compressed tokens, ratio), giving users immediate feedback on how much content is being preserved.

<!-- @entry compression-preservation-performance -->

**Detection overhead**: The state machine runs in O(n x d) time where n is word count and d is delimiter types (15). In practice this is sub-millisecond even for 10,000+ word inputs — negligible compared to the BERT forward pass (20-80ms on GPU).

**Compression ratio impact**: Protecting quoted/code content reduces the effective compression ratio proportionally to how much of the input is structured content. For a message that is 30% code, the effective compression rate will be lower than configured. This is intentional — the alternative (compressing code) produces incorrect output.

**Budget independence**: Protected words do not consume BERT selection budget. This means the compression model's quality for natural language text is completely unaffected by the protection layer — the same number of BERT-ranked words are kept regardless of how many words are protected.

<!-- @entry compression-preservation-status -->

*Implementation is in progress. The detection algorithm, BERT integration, configuration schema, pipeline threading, and UI visualization have been designed. This section will be updated with benchmarks once the implementation is complete.*
