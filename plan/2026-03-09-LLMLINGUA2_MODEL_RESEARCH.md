# LLMLingua-2 Model Research & Token-Classification Compression Alternatives

**Date**: 2026-03-09
**Goal**: Catalog all microsoft/llmlingua-2 models on HuggingFace and identify other token-classification-based prompt/context compression models.

---

## 1. Microsoft LLMLingua-2 Models (Complete List)

The HuggingFace API (`/api/models?search=llmlingua-2&author=microsoft`) returns **exactly 2 models**. There are no other microsoft/llmlingua-2-* models beyond the two we already support.

### 1a. microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank

| Property | Value |
|----------|-------|
| **Repo ID** | `microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank` |
| **Architecture** | `BertForTokenClassification` |
| **model_type** | `bert` |
| **Parameters** | 177,341,186 (177M, F32) |
| **model.safetensors** | ~709 MB (usedStorage: 709,388,104 bytes) |
| **License** | Apache-2.0 |
| **Downloads** | 124,652 |
| **Pipeline** | token-classification |

**config.json key values:**

| Key | Value |
|-----|-------|
| hidden_size | 768 |
| num_attention_heads | 12 |
| num_hidden_layers | 12 |
| intermediate_size | 3072 |
| vocab_size | 119,647 |
| max_position_embeddings | 512 |
| hidden_act | gelu |
| position_embedding_type | absolute |
| type_vocab_size | 2 |

**Tokenizer files:**
- `tokenizer.json` (fast tokenizer)
- `tokenizer_config.json`
- `vocab.txt` (WordPiece vocabulary)
- `special_tokens_map.json`
- Special tokens: `[CLS]`, `[SEP]`, `[PAD]`, `[MASK]`, `[UNK]`

### 1b. microsoft/llmlingua-2-xlm-roberta-large-meetingbank

| Property | Value |
|----------|-------|
| **Repo ID** | `microsoft/llmlingua-2-xlm-roberta-large-meetingbank` |
| **Architecture** | `XLMRobertaForTokenClassification` |
| **model_type** | `xlm-roberta` |
| **Parameters** | 558,945,282 (559M, F32) |
| **model.safetensors** | ~2.27 GB (usedStorage: 2,270,013,685 bytes) |
| **License** | MIT |
| **Downloads** | 42,582 |
| **Pipeline** | token-classification |

**config.json key values:**

| Key | Value |
|-----|-------|
| hidden_size | 1024 |
| num_attention_heads | 16 |
| num_hidden_layers | 24 |
| intermediate_size | 4096 |
| vocab_size | 250,102 |
| max_position_embeddings | 514 |
| hidden_act | gelu |
| position_embedding_type | absolute |
| type_vocab_size | 1 |

**Tokenizer files:**
- `tokenizer.json` (fast tokenizer, SentencePiece-based)
- `tokenizer_config.json`
- `special_tokens_map.json`
- Special tokens: `<s>` (BOS/CLS), `</s>` (EOS/SEP), `<pad>`, `<mask>`, `<unk>`
- NOTE: No `sentencepiece.bpe.model` file — only the fast `tokenizer.json`

### Key Differences Between the Two Models

| Aspect | BERT-base | XLM-RoBERTa-large |
|--------|-----------|-------------------|
| Parameters | 177M | 559M (3.1x larger) |
| File size | 709 MB | 2.27 GB (3.2x larger) |
| Hidden size | 768 | 1024 |
| Layers | 12 | 24 |
| Attention heads | 12 | 16 |
| Vocab size | 119K | 250K |
| Max seq len | 512 | 514 |
| Tokenizer type | WordPiece (vocab.txt) | SentencePiece (tokenizer.json) |
| License | Apache-2.0 | MIT |
| Quality | Good | Best (paper's primary model) |
| Speed | ~3x faster | Baseline |

---

## 2. Community ONNX Conversions

The `@atjsh/llmlingua-2-js` project has published ONNX conversions on HuggingFace that include additional distilled variants not available from Microsoft:

| Model | HuggingFace Repo | Quantized ONNX Size |
|-------|-------------------|---------------------|
| TinyBERT | `atjsh/llmlingua-2-js-tinybert-meetingbank` | ~57 MB |
| MobileBERT | `atjsh/llmlingua-2-js-mobilebert-meetingbank` | ~99 MB |
| BERT-base (mBERT) | `atjsh/llmlingua-2-js-bert-base-multilingual-cased-meetingbank` | ~710 MB |
| XLM-RoBERTa-large | `atjsh/llmlingua-2-js-xlm-roberta-large-meetingbank` | ~562 MB (int8) |

The TinyBERT and MobileBERT variants are **distilled by @atjsh** from the Microsoft models — they are NOT official Microsoft releases. They are useful for lightweight/fast inference but may have lower compression quality.

---

## 3. Other Token-Classification Compression Models

### 3a. Provence (NAVER Labs Europe) — ICLR 2025

**Approach**: Per-token binary classification for context pruning in RAG pipelines. Receives question + passage, outputs binary mask marking relevant vs irrelevant sentences. Also doubles as a reranker (dual-purpose: prune + rerank).

**Key difference from LLMLingua-2**: Query-aware (considers the question when deciding what to prune), whereas LLMLingua-2 is query-agnostic (compresses based on information content alone).

| Model | Repo ID | Architecture | Base Model | Params | License |
|-------|---------|-------------|------------|--------|---------|
| Provence DeBERTav3 | `naver/provence-reranker-debertav3-v1` | `Provence` (custom) | DeBERTa-v3-large | 435M | CC-BY-NC-ND-4.0 |
| xProvence BGE-M3 v1 | `naver/xprovence-reranker-bgem3-v1` | `XProvence` (custom) | XLM-RoBERTa (BGE-M3) | 568M | CC-BY-NC-ND-4.0 |
| xProvence BGE-M3 v2 | `naver/xprovence-reranker-bgem3-v2` | `XProvence` (custom) | XLM-RoBERTa (BGE-M3) | 568M | CC-BY-NC-ND-4.0 |

**Tokenizer files (Provence DeBERTav3)**: `tokenizer.json`, `tokenizer_config.json`, `added_tokens.json`, `special_tokens_map.json`
**Tokenizer files (xProvence BGE-M3)**: `tokenizer.json`, `tokenizer_config.json`, `sentencepiece.bpe.model`, `special_tokens_map.json`

**Verdict**: Excellent model but **not usable** for us:
- CC-BY-NC-ND-4.0 license (non-commercial, no derivatives) — incompatible with AGPL
- Query-aware design means it's best for RAG, not general prompt compression
- Custom model code (`modeling_provence.py`) — cannot load with standard AutoModel

### 3b. OpenProvence (hotchpotch) — MIT Licensed

Open-source reimplementation of the Provence approach with MIT license.

| Model | Repo ID | Architecture | Base Model | Params | License |
|-------|---------|-------------|------------|--------|---------|
| OpenProvence xsmall | `hotchpotch/open-provence-reranker-xsmall-v1` | `OpenProvenceForSequenceClassification` | Unknown (~30M) | ~30M | MIT |
| OpenProvence large | `hotchpotch/open-provence-reranker-large-v1` | `OpenProvenceForSequenceClassification` | ruri-v3-reranker-310m | ~310M | MIT |

**Tokenizer files (large)**: `tokenizer.json`, `tokenizer.model` (sentencepiece), `tokenizer_config.json`, `special_tokens_map.json`

**Verdict**: Interesting but **different use case**:
- Query-aware pruning (needs question + context), not general prompt compression
- Custom model code required
- Focus on Japanese + English bilingual
- MIT license is compatible
- Could complement LLMLingua-2 for RAG-specific compression

### 3c. Jasper-Token-Compression-600M (infgrad/NovaSearch)

| Property | Value |
|----------|-------|
| **Repo ID** | `infgrad/Jasper-Token-Compression-600M` |
| **Architecture** | `JasperV2Encoder` (custom, based on Qwen3) |
| **model_type** | `qwen3` |
| **Parameters** | 607M (BF16) |
| **License** | MIT |
| **Pipeline** | sentence-similarity (NOT token-classification) |

**Approach**: NOT a token classifier. Uses embedding-space compression — text goes through word_embedding, then Qwen3MLP, then `adaptive_avg_pool1d` to compress token count. Designed for retrieval/embedding tasks, not prompt compression for LLMs.

**Verdict**: **Not applicable** — this is an embedding compression model for search/retrieval, not a prompt text compression model. The compressed output is a dense vector, not readable text.

### 3d. PCRL and TACO-RL

**PCRL** (Prompt Compression with Reinforcement Learning): Uses a policy network to edit prompts. No public model weights found on HuggingFace.

**TACO-RL** (Task Aware Prompt Compression): RL-based task-specific compression built on top of LLMLingua-2 encoder. Published at ACL 2025 Findings. No public model weights found on HuggingFace — appears to be research code only.

**Verdict**: Neither has published model weights. TACO-RL is interesting as it fine-tunes an existing LLMLingua-2 model with task-specific RL rewards, but without weights it's not usable.

### 3e. Selective Context

**Repo**: `liyucheng09/Selective_Context` (411 stars, last updated 2024-02)
Uses GPT-2 for self-information scoring. Python-only, unmaintained, superseded by LLMLingua-2.

**Verdict**: Not recommended. Already covered in prior research.

---

## 4. Summary & Recommendations

### LLMLingua-2 Models: Complete

There are **only 2 official microsoft/llmlingua-2 models**, and we already support both:
1. `microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank` (177M, Apache-2.0)
2. `microsoft/llmlingua-2-xlm-roberta-large-meetingbank` (559M, MIT)

No additional Microsoft models exist in this namespace.

### Best Token-Classification Compression Models for Our Use Case

| Model | Type | Query-Aware? | License | Params | Recommended? |
|-------|------|-------------|---------|--------|-------------|
| **llmlingua-2 BERT-base** | Token classification | No | Apache-2.0 | 177M | **Yes (primary small)** |
| **llmlingua-2 XLM-RoBERTa** | Token classification | No | MIT | 559M | **Yes (primary large)** |
| atjsh TinyBERT distill | Token classification | No | MIT | ~15M | Maybe (quality unknown) |
| atjsh MobileBERT distill | Token classification | No | MIT | ~25M | Maybe (quality unknown) |
| OpenProvence | Sentence classification | Yes | MIT | 30-310M | No (different use case) |
| Provence (NAVER) | Sentence classification | Yes | NC-ND | 435-568M | No (license + use case) |
| Jasper-Token-Compression | Embedding compression | No | MIT | 607M | No (not text output) |
| PCRL / TACO-RL | Token classification (RL) | Task-specific | — | — | No (no weights available) |

### Key Takeaway

LLMLingua-2 remains the **only viable open-source token-classification model for general prompt compression**. The Provence family is promising but designed for query-aware RAG pruning (different problem). No new competitors have emerged with published weights as of March 2026.

The atjsh community distillations (TinyBERT: 57MB, MobileBERT: 99MB) are worth evaluating as lighter alternatives if the official BERT-base model (709MB) is too large for desktop distribution.
