# Prompt/Context Compression Research

**Date**: 2026-03-08
**Goal**: Evaluate practical alternatives for compressing chat messages before sending to an LLM, to reduce token usage. Must work locally, be well-maintained, and target a desktop app (macOS/Linux/Windows).

---

## 1. LLMLingua-2 (Python, Microsoft)

**Repo**: [microsoft/LLMLingua](https://github.com/microsoft/LLMLingua)
| Metric | Value |
|---|---|
| Stars | ~5,900 |
| Contributors | 17 |
| Last push | 2025-10-28 |
| License | MIT |
| Status | Active (published at EMNLP'23, ACL'24) |

**How it works**: Token classification using a fine-tuned XLM-RoBERTa (large, 355M params) or mBERT (base, 177M params). Each token gets a "preserve" probability; tokens below a threshold are dropped. This is **extractive** -- no hallucination risk.

**Models available on HuggingFace**:
- `microsoft/llmlingua-2-xlm-roberta-large-meetingbank` -- 2.24 GB (safetensors), 355M params
- `microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank` -- ~700 MB, 177M params (smaller, faster)

**Performance**: 3x-6x faster than LLMLingua-1. End-to-end latency improvement of 1.6x-2.9x at 2x-5x compression ratios. Retains 95-98% accuracy. Peak GPU memory ~2.1 GB.

**Python dependencies**: `transformers`, `accelerate`, `torch`, `tiktoken`, `nltk`, `numpy`. PyTorch alone is 700MB-2GB depending on platform.

**Integration with Rust/Tauri**: Would require spawning a Python subprocess or sidecar. Major downsides:
- Requires Python runtime installed (or bundled) -- **huge** increase in app size (500MB+)
- Cold start time: loading PyTorch + model = 5-15 seconds
- Memory overhead: 2-4 GB for the process
- Cross-platform Python packaging is painful (venv, pip, CUDA/CPU variants)

**Verdict**: Excellent algorithm, terrible fit for a desktop app distribution. The Python dependency chain is a dealbreaker for bundling in a Tauri app.

---

## 2. ONNX Runtime from Rust (via `ort` crate)

**Repo**: [pykeio/ort](https://github.com/pykeio/ort)
| Metric | Value |
|---|---|
| Stars | ~2,050 |
| Contributors | 30 |
| Last release | v2.0.0-rc.12 (2026-03-05) |
| License | Apache-2.0 |
| Status | Very active, approaching stable 2.0 |

**How it works**: Load the LLMLingua-2 ONNX model directly in Rust. The model is just a token classifier -- feed in tokenized text, get per-token preserve/drop predictions, filter tokens.

**ONNX model sizes** (from [atjsh/llmlingua-2-js](https://huggingface.co/atjsh/llmlingua-2-js-xlm-roberta-large-meetingbank)):
- `model_quantized.onnx` (int8): **562 MB** (XLM-RoBERTa large)
- `model_fp16.onnx`: 1.12 GB
- For mBERT small variant: ~180 MB quantized (estimated from param count)

**ONNX Runtime native library size**: ~15-20 MB (CPU-only, per platform). The `ort` crate can download prebuilt binaries automatically or you bundle the dylib.

**What you'd need to build**:
1. Tokenizer: Use `tokenizers` crate (HuggingFace's Rust tokenizer, ~2MB) to tokenize input with the XLM-RoBERTa/mBERT tokenizer
2. Model inference: Use `ort` to run the ONNX model, get per-token logits
3. Token filtering: Apply softmax, threshold, reconstruct text from kept tokens
4. This is roughly 200-500 lines of Rust code

**Production users of `ort`**: Bloop (semantic code search), SurrealDB, Google Magika, Wasmtime, Supabase edge functions.

**Performance estimate**: BERT-base inference on CPU is typically 5-20ms for a single sequence (512 tokens). For a 2000-token prompt split into 4 chunks: ~20-80ms total. Very fast.

**Cross-platform**: macOS (x86_64 + arm64), Linux (x86_64, arm64), Windows (x86_64). All supported natively by ort.

**Verdict**: **Best option for the project.** Native Rust, no Python dependency, fast inference, well-maintained crate with production users. Model size (180-562 MB) is the main cost, but can be downloaded on first use. The mBERT-small variant keeps it under 200 MB.

---

## 3. Summarization via Local LLM (e.g., Ollama)

**How it works**: Send older conversation messages to a local LLM with a "summarize this conversation" prompt. Replace older messages with the summary.

**Tradeoffs vs extractive compression**:

| Aspect | Extractive (LLMLingua-2) | Abstractive (LLM Summary) |
|---|---|---|
| Hallucination risk | **None** (keeps original tokens) | **High** (LLM generates new text) |
| Compression ratio | 2x-5x typical | 5x-20x possible |
| Latency | 20-80ms | 2-30 seconds (depends on model/hardware) |
| Faithfulness | Perfect (by definition) | Variable, may lose details or invent facts |
| Dependency | ONNX model (~200 MB) | Requires user to have Ollama/local LLM running |
| Controllability | Precise (set compression ratio) | Unpredictable output length |

**Key concern**: Hallucination. An LLM summarizing a coding conversation might subtly alter function names, parameter values, or error messages. This is particularly dangerous for coding agent contexts where precision matters.

**Advantage**: Can achieve much higher compression ratios while producing natural-sounding text. Good for very long conversations where you'd otherwise truncate.

**Hybrid approach**: Use extractive compression (LLMLingua-2) for recent context, and LLM summarization for very old messages (>10 turns back) where approximate recall is acceptable.

**Verdict**: Good as a supplementary strategy, but not reliable enough as the primary compression method due to hallucination risk. Best used as an opt-in "aggressive compression" mode. Since LocalRouter already connects to user LLMs, the integration cost is low -- but it adds latency and unpredictability.

---

## 4. Simple Extractive Methods (No ML)

### 4a. Message Truncation / Sliding Window

**Approach**: Drop messages older than N turns or beyond a token budget. Keep system message + last N messages.

- **Compression ratio**: Arbitrary (depends on window size)
- **Implementation**: ~50 lines of Rust
- **Pros**: Zero dependencies, instant, deterministic
- **Cons**: Loses all context beyond the window. Abrupt cutoff can confuse the LLM.
- **Hallucination**: None (no text modification)

### 4b. TF-IDF Sentence Scoring

**Approach**: Score each sentence/message by TF-IDF importance. Keep top-K sentences up to token budget.

**Rust crate**: [tfidf-text-summarizer](https://crates.io/crates/tfidf-text-summarizer) -- 13 stars, last updated 2024-04, requires nightly Rust. **Not recommended** (unmaintained, nightly-only).

**DIY**: TF-IDF is trivial to implement (~100-200 lines of Rust). No external dependencies needed.

- **Compression ratio**: 2x-5x
- **Pros**: Fast, no ML model needed, no hallucination
- **Cons**: Purely statistical, no semantic understanding. May drop contextually important but statistically unremarkable messages.

### 4c. Keyword/Entity Extraction

**Approach**: Extract key terms (function names, variable names, error codes) and bias retention toward messages containing them.

- Good for coding contexts where specific identifiers matter
- Can be combined with recency weighting
- ~200-300 lines of Rust

### 4d. Self-Information Scoring (SelectiveContext approach)

**Approach**: Use a small language model (GPT-2 level) to compute per-token self-information (negative log probability). Low self-information tokens are predictable/redundant and can be dropped.

This is what [Selective Context](https://github.com/liyucheng09/Selective_Context) does (411 stars, but **last updated 2024-02**, single maintainer, Python-only). The algorithm itself is simple and could be reimplemented in Rust using `ort` + a GPT-2 ONNX model (~500MB).

**Verdict**: Sliding window is the baseline everyone should have. TF-IDF is a cheap upgrade. Self-information scoring is interesting but requires a model anyway (at which point, use LLMLingua-2 which is purpose-built for this). **Recommend implementing sliding window as the default strategy, with LLMLingua-2 ONNX as the "smart" strategy.**

---

## 5. Other Libraries and Tools

### 5a. Selective Context (Python)

**Repo**: [liyucheng09/Selective_Context](https://github.com/liyucheng09/Selective_Context)
| Metric | Value |
|---|---|
| Stars | 411 |
| Last push | 2024-02-12 |
| Contributors | ~3 |
| Status | **Unmaintained** (2+ years without update) |

Uses GPT-2 for self-information scoring. Python-only, same bundling problems as LLMLingua.

**Verdict**: Not recommended. Unmaintained, Python-only, and the approach is superseded by LLMLingua-2.

### 5b. rust-bert (Rust)

**Repo**: [guillaume-be/rust-bert](https://github.com/guillaume-be/rust-bert)
| Metric | Value |
|---|---|
| Stars | ~3,050 |
| Last push | 2026-01-13 |
| License | Apache-2.0 |

Provides Rust-native NLP pipelines including summarization and token classification. Supports ONNX backend via `ort`. Has `RobertaForTokenClassification` which could theoretically run LLMLingua-2 models.

**However**: Using rust-bert adds a very heavy dependency tree (libtorch or ONNX). For our use case, going directly to `ort` is simpler -- we don't need rust-bert's pipeline abstractions since the LLMLingua-2 inference is just a single forward pass + threshold.

**Verdict**: Overkill. Use `ort` directly instead.

### 5c. llmlingua-2-js (JavaScript/ONNX)

**Repo**: [atjsh/llmlingua-2-js](https://github.com/atjsh/llmlingua-2-js)
| Metric | Value |
|---|---|
| Stars | 21 |
| Last push | 2025-09-14 |
| Status | Experimental, single maintainer |

JavaScript implementation using ONNX Runtime Web. Proves the ONNX approach works, and provides the ONNX-converted models on HuggingFace. However, too small/experimental to depend on directly.

**Useful for**: Reference implementation and **pre-converted ONNX models** we can use with `ort` in Rust.

### 5d. PCToolkit (Python)

**Repo**: [3DAgentWorld/Toolkit-for-Prompt-Compression](https://github.com/3DAgentWorld/Toolkit-for-Prompt-Compression)
| Metric | Value |
|---|---|
| Stars | 286 |
| Last push | 2025-02-11 |

Unified toolkit wrapping SelectiveContext, LLMLingua, LongLLMLingua, SCRL, and Keep it Simple. Python-only, same distribution problems.

### 5e. 500xCompressor (Python)

Interesting research (ACL 2025 Main) achieving extreme compression ratios, but requires fine-tuning an LLM. Not practical for a desktop app.

---

## Recommendation Summary

| Strategy | Recommended? | Complexity | Model Size | Latency | Hallucination Risk |
|---|---|---|---|---|---|
| **Sliding window (truncation)** | Yes (baseline) | Trivial | 0 | 0ms | None |
| **LLMLingua-2 via `ort` (ONNX)** | **Yes (primary)** | Medium | 180-562 MB | 20-80ms | None |
| **LLM summarization** | Optional add-on | Low | 0 (uses existing) | 2-30s | High |
| LLMLingua-2 Python | No | High | 2+ GB with deps | 200ms+ | None |
| Selective Context | No | High | 500 MB+ with deps | Variable | None |
| TF-IDF scoring | Maybe (interim) | Low | 0 | <1ms | None |
| rust-bert | No (overkill) | High | Same as ort | Same | None |

### Recommended Architecture

**Tier 1 -- Sliding Window** (always available, zero cost):
- Token budget per conversation
- Keep system message + last N messages
- Simple, predictable, no downloads needed

**Tier 2 -- LLMLingua-2 ONNX** (opt-in, download model on first use):
- Use `ort` crate with the mBERT-small quantized ONNX model (~180 MB download)
- Use `tokenizers` crate for XLM-RoBERTa/mBERT tokenization
- Extractive: zero hallucination risk
- Fast: 20-80ms for typical prompts
- User downloads model on first enable (like Ollama model pulls)

**Tier 3 -- LLM Summarization** (opt-in, requires local LLM):
- Use user's existing Ollama/local LLM to summarize very old messages
- Only for messages beyond Tier 1's window
- Mark as "approximate" in UI
- Opt-in with clear hallucination warning

### Integration Plan for `ort` + LLMLingua-2

1. Add `ort` (with `download-binaries` feature) and `tokenizers` to Cargo.toml
2. On first enable: download quantized ONNX model from HuggingFace to app data dir
3. At compression time:
   - Tokenize input with the model's tokenizer (from `tokenizers` crate)
   - Run ONNX inference via `ort` -- get per-token preserve probabilities
   - Filter tokens where p_preserve > threshold (configurable, default ~0.5)
   - Reconstruct compressed text from surviving tokens
4. The ONNX Runtime dylib (~15-20 MB) ships with the app or is downloaded
5. Total additional app size: ~20 MB (runtime) + ~180 MB (model, downloaded on demand)
