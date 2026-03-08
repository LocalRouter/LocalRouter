<!-- @entry prompt-compression-overview -->

Prompt Compression reduces input token count by 5-14x on multi-turn chat conversations before sending them to LLM providers. It uses LLMLingua-2, an extractive token classification approach that identifies and removes redundant tokens while preserving the original text — no paraphrasing, no hallucination risk.

Compression runs in parallel with guardrails scanning and model routing, adding no latency to the request pipeline. It only applies to `/v1/chat/completions` requests with multiple messages — single-turn completions and embeddings are passed through unmodified.

**Key benefits:**

- 5-14x input token reduction (empirically benchmarked)
- Zero hallucination risk — extractive only, keeps original tokens verbatim
- Fast inference: 20-80ms on CPU for typical prompts
- Parallel execution in the request pipeline
- Per-client granular control over compression behavior

<!-- @entry llmlingua-2 -->

LLMLingua-2 is a token classification model that assigns a "preserve" probability to each token in the input. Tokens below the threshold are dropped, and the remaining tokens are reassembled into coherent text.

Unlike generative compression (which paraphrases), extractive compression never introduces new tokens — only original tokens from your messages appear in the compressed output.

<!-- @entry extractive-classification -->

### Extractive Token Classification

The compression process:

1. **Tokenize** — Input messages are tokenized using the model's tokenizer
2. **Encode** — A single forward pass through the BERT encoder produces per-token embeddings
3. **Classify** — A classification head assigns preserve/drop probability to each token
4. **Filter** — Tokens below the compression rate threshold are dropped
5. **Reconstruct** — Remaining tokens are joined back into text

This runs as a single forward pass through a small BERT model (not a generative LLM), making it extremely fast — typically 20-80ms on CPU.

<!-- @entry compression-models -->

### Model Options

Four ONNX models are available, downloaded from HuggingFace on first use:

| Model | Size | Speed | Quality | Best For |
|-------|------|-------|---------|----------|
| TinyBERT | 57 MB | Fastest | Good | High-volume, cost-sensitive workloads |
| MobileBERT | 99 MB | Fast | Better | Default — good balance of speed and quality |
| BERT-base | 710 MB | Moderate | High | Quality-sensitive applications |
| XLM-RoBERTa-large | 2.2 GB | Slower | Best | Multilingual content, maximum quality |

Models are cached locally after download and use ONNX Runtime for inference (no PyTorch required).

<!-- @entry compression-pipeline -->

The compression pipeline is integrated into the chat request flow:

```
Request arrives
    ├── Guardrails scan (original messages) ──┐
    ├── Strong/Weak routing ──────────────────┤ parallel
    └── Prompt compression ───────────────────┘
                │
                ▼
    Compressed messages sent to provider
```

Key pipeline behaviors:

- **Guardrails always scan original messages** — safety checks see uncompressed content
- **Tool calls and tool results are never compressed** — structured data is preserved exactly
- **System prompts optionally compressed** — disabled by default to preserve instruction fidelity
- **Recent messages preserved** — the most recent N messages are kept uncompressed (default: 4)
- **Fallback on failure** — if compression fails or is disabled, original messages pass through

<!-- @entry prompt-compression-config -->

Prompt Compression is configured globally and can be overridden per client.

<!-- @entry compression-rate -->

### Compression Rate

The compression rate controls how aggressively tokens are dropped:

| Rate | Token Reduction | Use Case |
|------|----------------|----------|
| 0.8 | ~20% removed | Light compression, maximum fidelity |
| 0.5 | ~50% removed | Default — good balance |
| 0.3 | ~70% removed | Aggressive, best for long conversations |
| 0.1 | ~90% removed | Maximum compression, may lose nuance |

Lower values mean more aggressive compression. The default rate of 0.5 typically achieves 5x token reduction.

<!-- @entry compression-message-settings -->

### Message Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `min_messages` | 6 | Minimum message count before compression activates |
| `preserve_recent` | 4 | Number of most recent messages kept uncompressed |
| `compress_system` | false | Whether to compress system prompts |

The `min_messages` threshold prevents compression on short conversations where it provides little benefit. `preserve_recent` ensures the AI always has full context for the most recent exchange.

<!-- @entry compression-per-client -->

### Per-Client Override

Each client can override the global Prompt Compression settings:

- **Inherit** (default) — Uses global compression settings
- **Enabled** — Forces compression on with custom rate and thresholds
- **Disabled** — No compression for this client

Per-client overrides allow you to enable aggressive compression for cost-sensitive automated pipelines while keeping compression off for interactive coding sessions where every token matters.
