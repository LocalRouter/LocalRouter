# Model Integrity — research: every way a client can check it got the advertised model

**Status: research + adoption path (2026-11-04).** Companion to
`plan/2026-11-04-MODEL_INTEGRITY_VERIFICATION.md`. Where the two disagree, the **Adoption path**
section below overrides the plan's tier table and phases.

The question: for any model, any provider and any mechanism, what can a client like LocalRouter
check, today, to show a response came from the model the provider advertises?

Confidence labels:

- **[DOC]**: official documentation.
- **[RW]**: real-world error reports, such as GitHub issues or forum threads.
- **[PAPER]**: peer-reviewed or arXiv primary source.
- **[UNC]**: unconfirmed or inferred; needs an empirical test before we ship behaviour on it.

---

## 1. Why this matters (measured, not hypothetical)

- **"Real Money, Fake Models"** (arXiv 2603.01919, CISPA):
  - Covered 17 shadow APIs and 24 endpoints.
  - **45.83% failed LLMmap fingerprint verification**, and another 12.5% deviated.
  - "GPT-5" fingerprinted as glm-4-9b, and "GPT-4o-mini" as Qwen2.5-7B.
  - One shadow API billed full price for about 38% of the official output volume.
  - [PAPER] https://arxiv.org/html/2603.01919v2
- **GateScope** (arXiv 2604.21083, IMC 2026), across 10 gateways:
  - On one gateway, gpt-5 was identified in only 13.09% of responses.
  - Billing gaps reached +62.8%.
  - Multi-turn memory degraded.
  - MIT code. [PAPER] https://arxiv.org/html/2604.21083v1
- **KBF field audit:** flagged 7 of 28 endpoints, mostly ones claiming to be Claude.
  [PAPER] https://arxiv.org/abs/2605.29524
- **AgentProv:** 44% of 75 OpenRouter endpoints reported `prompt_tokens` that differed from the
  local chat-template count. That means hidden or injected context. [PAPER] https://arxiv.org/html/2609.00052
- **K2 Vendor Verifier:** Kimi K2-0905 tool-call F1 scored 50.6% on Nebius and 69.5% on Groq,
  against a threshold of at least 80%. [DOC] https://github.com/MoonshotAI/K2-Vendor-Verifier

The threat is concentrated in **resellers, relays and gateways** that claim closed models (GPT,
Claude, Gemini), and in **open-weight hosts** that differ in quantization, chat template or tool
parsing. First-party endpoints, such as OpenAI direct or Anthropic direct, are low-risk. For them,
the useful check is *change over time*, not substitution.

---

## 2. Mechanism catalog

### A. Vendor-minted tokens a client can replay to the vendor (cryptographic, free or almost free)

Reasoning models return opaque blobs that only the vendor can decrypt or verify. A client that has
its **own first-party key** can replay a blob it got from a reseller back to the real vendor. The
vendor's accept/reject answer is an oracle for "this blob was minted by us". These are the only
cryptographic checks that work for closed models today.

**Anthropic: thinking `signature` (strongest of the three)**

- **What it proves:**
  - Claude minted the block.
  - Claude 5.1+ models (the preserved-thinking check):
    - The exact `system` prompt, `tools` and earlier messages were what Claude saw.
    - So it catches a reseller that injects a system prompt or rewrites tools.
  - Coarse model binding through the "who reads whose blocks" graph.
- **What it doesn't prove:**
  - The answer text. The signature covers the thinking, not the final text.
  - The exact model inside a reading group. Opus 5.5 reads Sonnet 5.5 and Haiku 5.5 blocks, so
    those downgrades pass.
- **How:**
  - `POST /v1/messages/count_tokens` (free, rate-limited at 5k–20k RPM) [DOC] with:
    - the replayed history;
    - header `anthropic-beta: thinking-binding-controls-2026-08-01`;
    - `thinking.block_binding.prefix_mismatch_behavior: "error"`.
  - For model-binding detail (`input_transformations[].reason = model_binding_mismatch`), make a
    `/v1/messages` call with tiny output.
- **Responses:**
  - **Tampered or forged signature:** always `400 invalid_request_error`, message
    ``Invalid `signature` in `thinking` block`` with no suffix. [DOC]
  - **Prefix mismatch:** a 400 ending "bound to a different conversation", followed by a sentence
    naming what changed. [DOC]
  - Whether count_tokens also rejects a tampered signature is [UNC]; the docs say it "runs the same
    prefix check".
- **Limits:**
  - Sonnet 5.5 and Haiku 5.5 blocks are **bound to the producing account**. From any other account
    they are dropped as `organization_binding_mismatch`, so a reseller's Sonnet/Haiku 5.5 blocks
    can't be checked with the user's key. [DOC]
  - Opus 5.5, Opus 5, Fable 5.1 and earlier blocks are portable across the Claude API, Bedrock,
    Vertex and Foundry. [DOC]
- **Through resellers:** only those that keep the native Messages format, such as Claude Code
  relays, or OpenRouter `reasoning_details`. Chat-Completions translation drops the block.
- **Economic argument (probe requests):** LocalRouter writes the probe prompt itself. A genuine
  signature bound to that exact prefix means the reseller paid for a real Claude call on this
  prompt. Splicing a cheaper model's answer onto it saves the reseller nothing, so substitution is
  pointless on probes.

**OpenAI: Responses API `encrypted_content` (reasoning items)**

- **Where it appears:** returned by default when `store:false`. [DOC]
  https://developers.openai.com/api/docs/guides/reasoning
- **Replay outcomes:**
  - **Forged or modified blob:** `400 invalid_encrypted_content`, "could not be verified. Reason:
    Encrypted content could not be decrypted or parsed." [RW] (opencode #29879, codex #8121)
  - **Blob from another organization:** a 400 with the same code and "Reason: Encrypted content
    organization_id did not match the target organization." [RW] (LiteLLM incident, codex #12934,
    OpenAI forum)
  - **Blob from another model family:** silently omitted, no error. [DOC] "Persisted reasoning can
    be reused only within the same model family… the API omits incompatible reasoning."
  - **Azure:** blobs are bound to the resource, and decryption fails across resources. [RW]
- **The oracle [UNC, must test]:**
  - Replay the reseller's blob with the user's own OpenAI key.
  - "organization_id did not match" means OpenAI decrypted the blob, so it is genuine and was
    minted under someone's organization.
  - "could not be decrypted or parsed" means forged, Azure-minted, or another vendor's blob.
  - It's a 400, so presumably unbilled [UNC].
- **What it doesn't prove:** which model, or that the blob belongs to this answer. A reseller could
  reuse one genuine cheap blob. The blob is not known to be bound to the prompt.
- A May 2026 cross-account replay finding was "mitigated" by Aug 2026, in an undisclosed way (arXiv
  2608.09867). Binding may now be stricter, which would strengthen the oracle.
- **Through resellers:** only Responses-API passthrough or OpenRouter `reasoning_details[].data`
  (format `openai-responses-v1` or `azure-openai-responses-v1`). Chat Completions carries no
  reasoning items.
- **Cheaper probe:** whether `POST /v1/responses/input_tokens` validates blobs is [UNC]; it is in
  the test plan.

**Google: Gemini `thoughtSignature`**

- **Validation:** strict only on `functionCall` parts. A forged signature returns `400
  INVALID_ARGUMENT` "Thought signature is not valid". [DOC] Vertex thought-signatures page;
  [RW] Roo-Code #11716
- **Platform binding:** AI Studio signatures are rejected by Vertex [RW] (vercel/ai #14196), so the
  check tells you which platform minted the signature.
- **Weaknesses:**
  - There is a documented bypass sentinel, `skip_thought_signature_validator`.
  - Model and project binding are undocumented [UNC]; one report hints signatures aren't bound to
    the conversation.
- **What it proves:** "a genuine Gemini signature from platform X". Weaker than Anthropic.
- **Through resellers:** the OpenAI-compatible field `tool_calls[].extra_content.google.thought_signature`.
  Resellers often strip it [RW].
- Whether `countTokens` (free) validates signatures is [UNC].

**xAI: `reasoning.encrypted_content`** on the Responses API (always on for grok-4.7) [DOC]. Replay
behaviour is undocumented [UNC]. Treat it like OpenAI until tested.

**Not available** from Mistral (its Responses preview has an `encrypted_content` field but its docs
say the reasoning is plaintext [UNC]), Cohere, DeepSeek (plaintext `reasoning_content`), Moonshot,
Zhipu, Qwen or MiniMax (Anthropic-compatible thinking, signature behaviour [UNC]).

### B. Hardware attestation (TEE: Intel TDX or AMD SEV-SNP + NVIDIA confidential GPU)

| Provider | Weights bound? | Client verifies | Rust path |
|---|---|---|---|
| **Tinfoil** | **Yes.** Modelwrap puts a pinned HF commit in an EROFS image with a dm-verity root hash, inside the measured config. This covers disk reads, not GPU memory. [DOC] https://tinfoil.sh/blog/2026-02-03-proving-model-identity | SEV-SNP/TDX quote against measurements published through Sigstore; attested TLS key pinned; GPU checked at boot, so covered indirectly | Port the Go verifier (AGPL, compatible) with the `sev` and `dcap-qvl` crates, `sigstore`, and a rustls custom verifier |
| **Phala / ACI gateway** | No. Only a typed claim, `model_weights_provenance` | TDX quote (DCAP), keyset digest, RTMR3 replay, compose hash, TLS SPKI pin; per-response Ed25519 receipts (`X-Receipt-Id`, `GET /v1/aci/receipts/{id}`) | Rust reference gateway, Apache-2.0: https://github.com/Dstack-TEE/private-ai-gateway (spec `spec/aci.md`) |
| **NEAR AI Cloud** | No; one signing key is shared per model | TDX quote, `app_compose` hash, GPU check via NRAS, per-response signature (`GET /v1/signature/{chat_id}`) | `bitrouter-attestation` (Rust, Apache-2.0, alpha) |
| Venice, Chutes, Privatemode, Secret AI, io.net, Nillion | No or partial | Varies; several verifier paths are incomplete or not fail-closed | Go (`teep`, AGPL, multi-provider) or nothing |
| OpenAI "Private Inference", Anthropic | Announced or research only | — | — |

**Caveats:**

- **Physical attacks:** TEE.fail (under $1k of DDR5 interposer hardware) can extract TDX
  attestation keys and forge both TDX and NVIDIA CC attestations. A provider that **controls the
  hardware can forge attestation**. Attestation defends against a compromised operator, not a
  malicious one.
- **Network calls:** verification needs Intel PCS, AMD KDS, NVIDIA NRAS, and Sigstore or GitHub.
  These have to be user-enabled and cached.
- **Survey source:** https://amiller.github.io/awesome-private-inference (a daily provider probe)
  shows no provider attesting weights end-to-end except through Tinfoil-style measurement.

### C. Provider-declared metadata (cheap, honest-provider signal; enforceable through routing)

- **OpenRouter:**
  - `GET /api/v1/models/{author}/{slug}/endpoints` returns per-endpoint `quantization` (int4, int8,
    fp4, mxfp4, nvfp4, fp6, fp8, mxfp8, fp16, bf16, fp32, unknown), `provider_name`,
    `context_length`, `supported_parameters` and `uptime`. [DOC]
  - The routing setting `provider.quantizations` / `only` / `require_parameters` **enforces** this
    per request.
  - The `X-OpenRouter-Metadata: enabled` header, or `GET /api/v1/generation?id=`, reveals which
    provider actually served the request. [DOC]
  - Auto Exacto deranks endpoints on tool-call error rate plus GPQA/Tau2. [DOC]
- **Other hosts that declare precision or revision:**
  - Together: `quantization`, plus `certifiedModelRevisionId` on dedicated endpoints.
  - DeepInfra: `quantization` and `version`, with `MODEL:VERSION` pinning.
  - Nebius: public `models_info` has `flavors[].quantization` and `huggingface_url`.
  - Fireworks: `precision`.
  - Chutes: `quantization` and `root`, the HF repo.
  - Replicate: a 64-char `version`, which pins an image, not weights.
  - Cloudflare: the quantization suffix in model ids.

  Groq says only "TruePoint", and Featherless serves FP8 across the board. [DOC] (links in the
  agent survey, summarised in §5)

### D. Passive accounting on real traffic (zero extra requests)

- **Tokenizer accounting:** compare billed `usage.prompt_tokens` and `completion_tokens` with a
  local count.
  - OpenAI uses `o200k_base` (gpt-5\*, gpt-4.1, gpt-4o, o-series); gpt-oss uses `o200k_harmony`.
    Both are in tiktoken. [DOC/source]
  - Open-weight models use HF `tokenizers` plus the chat template.
  - What it catches:
    - A **different tokenizer family**, i.e. substitution across vendors (for example "GPT-5"
      answered by GLM).
    - **Injected hidden context** (AgentProv's 44% finding).
    - **Billing inflation** (GateScope, Real Money).
  - It cannot tell same-tokenizer models apart, such as gpt-5 vs gpt-5-mini.
- **Official count endpoints** give exact counts for closed tokenizers. They need the user's
  first-party key and a network call per check, so they are user-enabled:
  - Anthropic `count_tokens` (free). Claude 4.7+ and Fable use a tokenizer that produces about 30%
    more tokens, so this separates model generations.
  - OpenAI `POST /v1/responses/input_tokens`.
  - Gemini `countTokens` (free).
  - Mistral `/v1/responses/input_tokens`.
  - Moonshot `/v1/tokenizers/estimate-token-count`.
  - Zhipu `/paas/v4/tokenizer`. [DOC]
- **Exact token ids, where returned:**
  - Sources: Together `logprobs.token_ids`, Fireworks `return_token_ids` and `raw_output` (the
    rendered prompt), Cohere `token_ids`, llama.cpp/vLLM `return_token_ids`.
  - A token id outside the advertised vocabulary means a mismatch.
  - Fireworks' `raw_output` also exposes the **chat template actually applied**.
- **Kimi Vendor Verifier `tests/prompt_tokens/`** ships expected `prompt_tokens` constants for
  fixed requests. That gives a cheap, vendor-blessed fingerprint for Kimi models.
- **Metadata consistency:**
  - `model` dated snapshot and `system_fingerprint` (OpenAI Chat, xAI, Groq, Cerebras, DeepSeek)
    are forgeable, so use them only as change detectors.
  - Response id formats prove nothing.

### E. Active black-box audits (statistical; cost money; opt-in)

Ranked for a desktop gateway (per-token billing, laptop CPU, closed models through resellers):

| Method | Access | Reference | Budget | Detects | Misses | Code / licence |
|---|---|---|---|---|---|---|
| **KBF**, knowledge-boundary cloze probes (2605.29524) | text | enrolled once against a trusted endpoint (~$7 for 6 models); bundleable | ≤$0.67/audit, probes batched 10 per request | 155/155 substitutions, 0/16 false positives, 95% at 15% dilution; robust to role prompts and temperature | **quantization** (0/9) | Apache-2.0 https://github.com/Ooo0ption/KBF; pure binomial statistics |
| **One Token Is Enough**, single-token JSD (2607.10252) | text, T=1 | bundled reference distributions (CC-BY data) | ~100–400 one-word calls | EER 7–11% | quantization lands in the "uncertain" band; system prompt must match | MIT TS library `ToseaAI/llm-fingerprint-detector`; port to Rust |
| **IRIS**, random-output features (2607.20860) | text, T>0 | labelled samples (trusted + candidates) | m≈9–80 calls | AUROC 0.99 on a size ladder; dilution power 0.85 / FPR 0.017; 14/15 same-model provider pairs separable | nothing at T=0 | **non-commercial licence**, so reimplement from the paper; 179 handcrafted features plus logistic regression |
| **Log Probability Tracking** (2512.03816) | top-k logprobs, 1 token | the endpoint's own history | 28 in / 20 out tokens per test (~$0.14/yr hourly) | changes, including a single fine-tuning step and quantization | needs logprobs (only ~23% of OpenRouter endpoints; none on Anthropic or Gemini 3.x; OpenAI only with reasoning off) | method only; repo unlicensed |
| **AgentProv**, tool-choice MMD (2609.00052) | text + tools | official API or checkpoint | 500–1,000 short calls | 630/630 checkpoint pairs; **7% false positives under system-prompt injection** (MET: 67%) | 8-bit quantization | code "will release" |
| **GateScope** multi-turn retention (2604.21083) | text | none | one 25-turn conversation | context truncation, memory degradation | — | MIT `CyberAINet/GateScope` |
| Model Equality Testing (MMD) | text | local or trusted samples | ~250–1,250 completions, under $1 | coarse substitution | fine-grained changes | MIT-ish |
| API-TAIL, RUT, TRAP | full-vocabulary logits locally | open weights + GPU | — | strong | — | not for laptops |

**Evasion:**

- GhostPrint (2606.16100) fine-tunes a weak model to mimic a stronger one and beats static
  fingerprints: about 95% against LLMmap, about 40% against MET [UNC figures].
- So **probe sets must be private, randomized and rotated**. KBF regenerates probes cheaply, and
  AgentProv rotates subsets.

### F. Third-party measurement data, bundled at build time (no runtime calls)

- **OpenRouter benchmark export:** daily CSV/JSON with checksums. Gives per (model × provider)
  GPQA-Diamond and tau2-airline accuracy, std-dev and run count, about 2,250 rows. **CC BY 4.0.**
  [DOC] https://openrouter.ai/api/v1/datasets/exports/README.md. Fetch it at build time, like
  models.dev, and attribute it.
- **Kimi Vendor Verifier** (MIT) per-vendor tables for K3/K3.1. **MiniMax Provider Verifier**
  (tool-call match and schema accuracy).
- **Not bundleable:** Artificial Analysis (licence forbids redistribution), Vals (proprietary), LT
  monitoring data (unpublished and unlicensed).

### G. Watermarks and media provenance

- **Text:**
  - OpenAI textGrain: the detector is for approved researchers only, and it is off by default in
    the API.
  - Anthropic text watermark: the detection API is a private preview for regulators, media and
    researchers, and enterprises checking their own output.
  - Google SynthID Text: needs Google's keys, which are private.

  **None of these is client-checkable today.** LocalRouter could apply for Anthropic's detection
  API.
- **Media:**
  - **C2PA** manifests are verifiable offline with `c2pa-rs`.
  - Who signs them: OpenAI gpt-image, Amazon Nova Canvas, and Anthropic files retrieved through the
    Files API.
  - A valid manifest proves the vendor signed the image, not which model made it unless the
    manifest names it. Stripping the metadata defeats the check.
  - Cheap and free for LocalRouter's `/v1/images` passthrough.

### H. Not usable now (watch list)

- **FlashProve (Anthropic, 2026-11-02):**
  - It is real: official post https://www.anthropic.com/research/provable-inference and repo
    https://github.com/anthropics/flashprove.
  - No deployment: research prototype only, Llama 4 Maverick only, and the post says nothing about
    proofs for Claude.
  - Anthropic met its roadmap goal through an unnamed project that "signs our inference traffic and
    recomputes inference on a trusted server". No client-verifiable artifact is described.
- **TOPLOC, PoSP, Verde, IMMACULATE, SVIP:** need provider cooperation, and no API exposes their
  artifacts to end clients.
- **EigenAI:** gives an operator ECDSA signature, which proves who answered, not what ran.
- **zkML** (DeepProve, EZKL): not at LLM scale.
- **Hollow-LLM** (2607.28884): ZK proofs over committed weights can be gamed with "ghost weights".
- **Finlayson** logit-image checks (thousands of calls with `logit_bias`), timing fingerprints (F1
  ~0.74, too noisy), owner-embedded fingerprints (need vendor keys), the Idiosyncrasies classifier
  (neural, family-level, spoofable).
- **Withdrawn:** committed SAE-feature traces (2604.18179).

---

## 3. Per-provider: the best check available today

| Provider (as configured in LocalRouter) | Strongest check | Next best | Notes |
|---|---|---|---|
| Anthropic direct | change detection only (A-replay is moot) | `count_tokens` accounting | first party; low risk |
| **Claude through any reseller or relay** (Anthropic-compatible base URL, OpenRouter, generic) | **A: signature replay** to the user's own Anthropic key (prefix + model + forgery) | `count_tokens` vs billed usage (catches tokenizer generation and injected context); KBF | Sonnet/Haiku 5.5 blocks are account-bound, so only forgery shows as a 400 |
| OpenAI direct / Azure | change detection: `system_fingerprint` + logprobs drift (reasoning off) | tiktoken accounting | — |
| **GPT through a reseller** | **A: `encrypted_content` org-oracle** [UNC] if Responses passthrough | tiktoken accounting (catches non-o200k models); KBF / One-Token | Chat-only resellers: accounting plus statistics only |
| Gemini direct | change detection (no logprobs on 3.x) | `countTokens` | — |
| **Gemini through a reseller** | **A: `thoughtSignature` replay** on function-call turns (genuine + platform) | `countTokens` accounting; KBF | signature often stripped by resellers |
| xAI direct or reseller | encrypted-reasoning replay [UNC] | `system_fingerprint`; KBF | logprobs ignored on grok-4.20+ |
| DeepSeek, Moonshot, Zhipu, Qwen, MiniMax direct or reseller | official count endpoints (Moonshot, Zhipu) + vendor verifier data (Kimi KVV, MiniMax) | logprob drift (DeepSeek, Qwen); KBF | no signed artifacts |
| **OpenRouter (open-weight models)** | **C: declared `quantization` + `provider.quantizations` enforcement + served-provider metadata** | F: OpenRouter benchmark export; LT drift where logprobs exist | add `X-OpenRouter-Metadata` |
| Together, Fireworks, DeepInfra, Nebius, Chutes | C: declared precision / revision | D: token ids / `raw_output` template check; LT drift | Fireworks `raw_output` reveals the template |
| Groq, Cerebras, SambaNova, Featherless | `system_fingerprint` change detection | KBF / One-Token; tool-call canaries | Groq has no logprobs |
| **Tinfoil** | **B: weights-attested** (dm-verity root → HF repo@commit) | — | the only provider that binds weights |
| Phala/ACI, NEAR AI | B: platform-attested + signed receipts | — | weights not bound |
| Ollama / llama.cpp / LM Studio / embedded | local: GGUF hash + quantization known | — | already trusted |

---

## 4. Adoption path for LocalRouter

LocalRouter already has the plumbing that matters:

- **Native `/v1/messages`** passthrough keeps thinking `signature`s byte-exact
  (`crates/lr-providers/src/messages.rs`, signature tests at `:1179`, `:1454`).
- **`/v1/responses`** with typed `encrypted_content` (`crates/lr-providers/src/openai_responses/types.rs:128`).
- An **OpenRouter** provider.
- **Logprobs** plumbing (`features/logprobs.rs`).

The gap: **Gemini thought signatures are not carried** (`gemini.rs` has none).

### Status vocabulary (UI)

- **Weights-attested:** TEE plus a weights hash (Tinfoil).
- **Platform-attested:** TEE, weights not bound.
- **Vendor-authenticated:** a replay oracle passed (Anthropic, OpenAI, Gemini).
- **Consistent:** statistics and accounting passed.
- **Not checked**
- **Suspicious**
- **Mismatch**

Statistical results never use the words "verified" or "proof".

### Stage 0: passive, no network, no money (do first)

1. **Accounting engine:**
   - Use `tiktoken-rs` for o200k / o200k_harmony, and HF `tokenizers` for open-weight models
     (downloaded on opt-in only).
   - Calibrate template overhead per model.
   - Track prompt and completion residuals and **billed-cost vs. published-rate** drift. This feeds
     Suspicious/Mismatch and a "billing discrepancy" signal.
2. **Token-id vocabulary check** wherever ids come back (Together, Fireworks, Cohere, llama.cpp).
   Also run a **Fireworks `raw_output` chat-template check**.
3. **OpenRouter:**
   - Send `X-OpenRouter-Metadata: enabled` and record the served provider for each request.
   - Show the endpoint's declared `quantization`.
   - Add a per-model routing rule "minimum precision" that maps to `provider.quantizations`.
   - Fetch the endpoints list only when the user opens a model's detail view, which counts as a
     user action.
4. **Build-time data:** add the OpenRouter benchmark export (CC BY 4.0) and the KVV/MiniMax tables
   to `lr-catalog`'s build step, next to models.dev.
   - Shows a "provider fidelity" column: provider accuracy vs. the model's best provider.
   - Add attribution in About → Licenses.
5. **C2PA check on `/v1/images` responses** with `c2pa-rs`, offline. Badge: "Signed by OpenAI" /
   "Signed by Amazon".
6. The verdict engine, Integrity tab, badges, warning modal and landing section from the plan,
   using the status vocabulary above.

**Cost:** ~0.1–1 ms of CPU per request and ~10–40 MB per loaded tokenizer. Build time grows by
one download. No runtime network calls.

### Stage 1: vendor replay oracles (needs the user's own first-party key; ~free)

- **Prerequisite: empirical confirmation run.** It needs the user's keys and **your approval**.
  - Anthropic A1–A6: ~$0.05; the count_tokens steps are free.
  - OpenAI R1–R8: ~$0.01–0.05.
  - Gemini G1–G8: ~$0.01.
  - **Total under ~$0.15.**
  - Output: a pinned fixture set of exact error bodies, used as test fixtures. This avoids
    string-matching heuristics; parse the structured `code` / `type` first. Messages are used only
    where the vendor gives no structured reason (OpenAI's org vs decrypt distinction), and that
    parsing is isolated in one versioned table.
- **Anthropic verifier:**
  - Sends the replay to `count_tokens` with the binding beta header and `"error"` mode.
  - An optional `/v1/messages` call (`max_tokens` minimal) gives model-binding detail.
  - Results: Vendor-authenticated / prefix changed (the reseller injected context) / forged.
- **OpenAI verifier:**
  - Replays `encrypted_content` with the user's key.
  - "org mismatch" means authenticated as OpenAI-minted. "decrypt failure" means forged or
    non-OpenAI.
  - Applies to resellers that pass the Responses API or OpenRouter `reasoning_details` through.
- **Gemini:**
  - First **carry `thoughtSignature`** through `gemini.rs` and the OpenAI-compatible
    `extra_content` path. This is also a correctness fix for Gemini 3 function calling.
  - Then replay on function-call turns. AI Studio vs Vertex shows up as a platform mismatch.
- **xAI:** after a test run, same shape as OpenAI.
- **Privacy rule:**
  - Replaying sends conversation content to the first-party vendor.
  - **Default: synthetic probe requests only**, where LocalRouter writes the prompt.
  - "Also verify sampled real requests" is a separate, explicit opt-in per provider, with a
    disclosure naming the vendor that will see the content.

### Stage 2: active statistical audits (opt-in; cost shown before running; budget-capped)

**Order of implementation:**

1. **KBF** for closed and open models: Apache-2.0, pure statistics.
   - Ship enrolled probe banks plus reference answers per model in the bundle.
   - Enrolment is a build-time cost of about $1 per model (paid API calls), so **each refresh
     needs approval**.
2. **One-Token JSD** fingerprints: port the MIT library, and use the CC-BY reference distributions.
3. **Logprob drift (LT)** on endpoints that return logprobs. It runs against the endpoint's own
   history, needs no reference, and is very cheap. It is the best quantization and change detector
   available.
4. **Official count-endpoint cross-check** (Anthropic, Gemini, OpenAI, Mistral, Moonshot, Zhipu)
   for resellers of those vendors' models.
5. **GateScope retention probe** (MIT) for context truncation. It costs as much as the context
   length tested.
6. **AgentProv** tool-choice MMD once its code is released, for agent and MCP-heavy users.
7. Tool-call canaries against the KVV/MiniMax datasets for Kimi and MiniMax models.

**Probe hygiene:** private, randomized banks, rotated each release; probes shaped like real
traffic; never a fixed public list (GhostPrint).

**Cost per audit:** KBF ≤$0.67 (often cents), One-Token ~100–400 one-word calls (cents), LT <$0.001
per test, retention probe ≈ the cost of one long-context call.

### Stage 3: hardware attestation providers

1. **ACI client** (Phala and any ACI-speaking aggregator), from the Rust/Apache-2.0 reference
   gateway.
   - Verifies the TDX quote (`dcap-qvl`, collateral pointed at Intel PCS rather than Phala's
     PCCS), the keyset digest, the compose hash and the TLS pin.
   - Verifies per-response receipts.
   - Status: Platform-attested.
2. **NEAR AI** through `bitrouter-attestation`, or modelled on it: TDX + NRAS + per-response
   signatures. Status: Platform-attested.
3. **Tinfoil:** port the Go verifier.
   - Uses `sev` + `dcap-qvl`, Sigstore bundle verification, and a pinned attested TLS key through a
     custom rustls verifier.
   - Shows **HF repo@commit** from the attested dm-verity root.
   - Status: **Weights-attested**.
   - Optional offline audit: recompute the root hash from the HF commit. This is a manual action
     and can mean downloading hundreds of GB.

**Network:** Intel PCS, AMD KDS, NVIDIA NRAS/RIM/OCSP, Sigstore/GitHub. All are user-enabled, cached
with TTL, and listed in the privacy docs.

**UI caveat text:** "Hardware attestation protects against a compromised operator, not against a
provider with physical control of the machine (TEE.fail)."

### Stage 4: watch list (no code until there is an artifact to verify)

- FlashProve proofs from any provider (the plan's Tier 3 design is kept for this).
- OpenAI Private Inference.
- Anthropic's text-watermark detection API (apply for access).
- EigenAI operator signatures.
- IMMACULATE or TOPLOC behind a public API.

---

## 5. Sources (by mechanism)

- **Anthropic:**
  - https://platform.claude.com/docs/en/build-with-claude/thinking#thinking-encryption
  - https://platform.claude.com/docs/en/build-with-claude/preserved-thinking
  - https://platform.claude.com/docs/en/build-with-claude/thinking-troubleshooting
  - https://platform.claude.com/docs/en/build-with-claude/token-counting
  - https://platform.claude.com/docs/en/models/fable-5-1/whats-new-fable-5-1#content-provenance
  - https://www.anthropic.com/news/claude-text-watermark
  - https://www.anthropic.com/research/provable-inference
  - https://github.com/anthropics/flashprove
- **OpenAI:**
  - https://developers.openai.com/api/docs/guides/reasoning
  - https://developers.openai.com/api/docs/guides/token-counting
  - https://github.com/anomalyco/opencode/issues/29879
  - https://github.com/openai/codex/issues/12934
  - https://docs.litellm.ai/blog/responses-api-encrypted-content-incident
  - https://arxiv.org/html/2608.09867
  - https://blog.cryptographyengineering.com/2026/05/29/fooling-around-with-encrypted-reasoning-blobs/
  - https://openai.com/index/eu-text-provenance/
- **Google:**
  - https://docs.cloud.google.com/vertex-ai/generative-ai/docs/thought-signatures
  - https://github.com/RooCodeInc/Roo-Code/issues/11716
  - https://github.com/vercel/ai/issues/14196
  - https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/get-token-count
- **xAI:** https://docs.x.ai/developers/model-capabilities/text/reasoning,
  https://docs.x.ai/docs/guides/fingerprint
- **TEE:**
  - https://amiller.github.io/awesome-private-inference
  - https://docs.tinfoil.sh/verification/attestation-architecture
  - https://github.com/Dstack-TEE/private-ai-gateway
  - https://docs.near.ai/cloud/verification
  - https://docs.rs/bitrouter-attestation
  - https://github.com/Phala-Network/dcap-qvl
  - https://docs.rs/sev
  - https://thehackernews.com/2025/10/new-teefail-side-channel-attack.html
- **Hosts and aggregators:**
  - https://openrouter.ai/docs/api/api-reference/endpoints/list-endpoints
  - https://openrouter.ai/docs/guides/routing/provider-selection
  - https://openrouter.ai/docs/guides/features/router-metadata
  - https://openrouter.ai/docs/guides/routing/auto-exacto
  - https://openrouter.ai/api/v1/datasets/exports/README.md
  - https://docs.together.ai/reference/chat-completions-1
  - https://docs.fireworks.ai/api-reference/post-chatcompletions
  - https://docs.deepinfra.com/api-reference/models/models-info.md
  - https://tokenfactory.nebius.com/model-catalog.md
- **Vendor verifiers:** https://github.com/MoonshotAI/Kimi-Vendor-Verifier,
  https://github.com/MoonshotAI/K2-Vendor-Verifier, https://github.com/MiniMax-AI/MiniMax-Provider-Verifier
- **Black-box papers:**
  - KBF 2605.29524
  - IRIS 2607.20860
  - API-TAIL 2610.36801
  - AgentProv 2609.00052
  - One Token 2607.10252
  - LT 2512.03816
  - RUT 2506.06975
  - GateScope 2604.21083
  - Real Money Fake Models 2603.01919
  - Cai et al. 2504.04715
  - MET 2410.20247
  - GhostPrint 2606.16100
  - Hollow-LLM 2607.28884
