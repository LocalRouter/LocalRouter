# Model Integrity — check catalog, reliability and span

**Status: planned, not implemented (2026-11-04).**

This document is the authoritative list of the checks LocalRouter could run to warn that a
provider does not appear to serve the advertised model. For each check it records where it runs,
how reliable it is, and which models and providers it applies to.

It builds on two documents:

- `plan/2026-11-04-MODEL_INTEGRITY_RESEARCH.md`: the evidence, sources and confidence labels behind
  each row, and the detailed adoption path.
- `plan/2026-11-04-MODEL_INTEGRITY_VERIFICATION.md`: the verdict engine, warning modal, UI,
  config, Tauri commands and landing-page section that consume these checks.

## Legend

**Where it runs:**

- **Local:** on the user's machine, with no extra network calls.
- **Provider calls:** extra requests to the provider under test. These cost tokens; the analysis
  is local.
- **Vendor key:** calls the real model vendor with the user's own first-party key.
- **Third-party:** needs outside services such as Intel PCS, AMD KDS, NVIDIA NRAS or
  Sigstore/GitHub.

**Reliability** states what a check is reliable *for*. No single check covers everything.

**Wording rule for the UI:** only cryptographic or hardware checks (#7, #11a–d, #22, #23) may use
the words "verified", "signed" or "attested". Every other check is statistical or self-declared,
and is reported as "consistent", "suspicious" or "mismatch".

---

## 1. Passive checks on existing traffic (free)

| # | Check | Where | Reliability | Span |
|---|---|---|---|---|
| 1 | **Token-count accounting:** billed prompt and completion tokens vs a local tokenizer count, with the chat-template overhead calibrated per model | Local | **High** for a swap to a model with a different tokenizer, hidden injected context, and overbilling. **Blind** to swaps within one tokenizer family (GPT-5 vs GPT-5-mini, Llama 70B vs 8B) | All OpenAI models (`tiktoken-rs`: o200k_base / o200k_harmony) and all open-weight models (HF `tokenizers`, downloaded on opt-in). Not Claude or Gemini: no public tokenizer (see #12) |
| 2 | **Token-ID vocabulary check:** a returned token id outside the advertised vocabulary is a definitive mismatch | Local | **Very high** when ids are returned; rarely available | Hosts that return ids: Together (`logprobs.token_ids`), Fireworks (`return_token_ids`), Cohere (`token_ids`), vLLM/llama.cpp. Open-weight models only |
| 3 | **Chat-template check:** the prompt the host actually rendered vs the model's official template | Local | **High** for broken deployments | Fireworks only (`raw_output`) |
| 4 | **Billing vs published price** | Local | **Medium–high** for overbilling; says nothing about identity | Every provider that reports usage |
| 5 | **Metadata change detection:** `model` dated snapshot, `system_fingerprint` | Local | **Low**, because it can be faked. Only useful for noticing that an honest provider changed something | OpenAI Chat, xAI, Groq, Cerebras, DeepSeek, Gemini (`modelVersion`) |
| 6 | **Throughput and latency profile** vs the provider-model's own history | Local | **Low**, too noisy. Supporting evidence only; never raises a warning by itself | Everything |
| 7 | **C2PA image signatures:** check the vendor's certificate chain on generated images (`c2pa-rs`) | Local, offline | **High** that the vendor signed the image; does not say which model. Stripping the metadata defeats it | OpenAI gpt-image, Amazon Nova Canvas, Anthropic Files API media. Images only |
| 8 | **Local model file hash** | Local | **Complete:** the weights are on disk | Ollama, llama.cpp, LM Studio and embedded engines |

## 2. Provider-published information (free; the provider's own claim)

| # | Check | Where | Reliability | Span |
|---|---|---|---|---|
| 9 | **Declared quantization and served provider**, plus routing that enforces it | Provider calls (metadata only, free) | **Medium**: self-declared. Useful against honest-but-cheap hosts, not against fraud | OpenRouter, all listed models: endpoints `quantization`, the `provider.quantizations` / `only` routing rules, and `X-OpenRouter-Metadata`. Also Together, DeepInfra, Nebius, Fireworks and Chutes for their own models |
| 10 | **Bundled quality data:** OpenRouter benchmark export (CC BY 4.0) plus the Kimi and MiniMax vendor-verifier results | Local at runtime (downloaded during the build, like models.dev) | **Medium**: periodic aggregate scores, not a per-request check | Models OpenRouter benchmarks (mostly popular open-weight models), Kimi models, MiniMax models |

## 3. Vendor token replay (cryptographic)

| # | Check | Where | Reliability | Span |
|---|---|---|---|---|
| 11a | **Anthropic thinking `signature`:** replay through `count_tokens` with the `thinking-binding-controls-2026-08-01` beta and `prefix_mismatch_behavior: "error"` | Vendor key; free | **High** for forged output and for a changed system prompt, tool set or history. **Medium** for model identity (Opus 5.5 accepts Sonnet and Haiku 5.5 blocks). Sonnet 5.5 and Haiku 5.5 blocks are bound to the producing account, so only forgery is detectable for them | **Claude only.** Needs thinking on, and a reseller that passes the native Anthropic format through |
| 11b | **OpenAI `encrypted_content`:** replay with the user's key. "organization_id did not match" means OpenAI minted the blob; "could not be decrypted" means it is forged or not from OpenAI | Vendor key; probably free (rejections are 400s) | **Medium–high, pending the confirmation run.** Proves OpenAI minted it, not which model; not known to be bound to the prompt | **OpenAI reasoning models only**, through resellers that pass the Responses API or OpenRouter `reasoning_details` through |
| 11c | **Gemini `thoughtSignature`:** replay on function-call turns | Vendor key | **Medium**: shows a genuine Gemini signature and which platform issued it (AI Studio vs Vertex). Validated strictly only on function-call turns | **Gemini 2.5/3 only.** Resellers often strip it. LocalRouter must first carry the signature through `gemini.rs` |
| 11d | **xAI encrypted reasoning** | Vendor key | **Unknown**; needs the confirmation run | Grok only |
| 12 | **Official count-endpoint cross-check:** a reseller's billed usage vs the vendor's own count | Vendor key (free for Anthropic and Gemini) | **Medium–high** for a different family or tokenizer generation (Claude 4.7+ uses about 30% more tokens). Blind within one generation | Claude, Gemini, GPT, Mistral, Kimi and GLM through resellers. Fills the Claude/Gemini gap in #1 |

## 4. Active statistical audits (extra paid requests; analysis local)

| # | Check | Where | Reliability | Span |
|---|---|---|---|---|
| 13 | **KBF:** knowledge-boundary cloze probes (Apache-2.0) | Provider calls; ≤$0.67 per audit | **High** for substitution (155/155 caught, 0/16 false alarms). **Blind** to quantization | **All chat models, closed and open.** Each model needs a one-time enrolment, which we ship |
| 14 | **One-token fingerprint:** single-token Jensen–Shannon divergence (MIT; port to Rust) | Provider calls; cents | **Medium** (equal error rate 7–11%). The system prompt must match the reference | All models that sample (T>0). Reference data exists for 11 models; others need enrolment |
| 15 | **IRIS-style random-output features** | Provider calls; 9–80 calls | **Medium–high**; separates the same model across hosts. Nothing at T=0 | All models that sample. Reimplement from the paper (its code licence is non-commercial) |
| 16 | **Logprob drift:** compare the endpoint against its own earlier baseline | Provider calls; well under $0.01 per test | **High sensitivity** to any change, including quantization and updates. Detects *change*, not *identity* | Endpoints that return logprobs: GPT-4o/4.1 and GPT-5 with reasoning off, DeepSeek, Qwen, Together, Fireworks, Cerebras, Nebius, Ollama/llama.cpp. **Not** Claude, Gemini 3, Groq or Grok 4.20+ |
| 17 | **Tool-choice MMD** (AgentProv) | Provider calls; 500–1,000 short calls | **High** (630/630 caught; 7% false alarms when a reseller injects a system prompt). Misses 8-bit quantization | Models with tool calling. Blocked until the code is released |
| 18 | **Multi-turn retention test** (GateScope, MIT) | Provider calls; cost of one long-context conversation | **High** for context truncated or degraded | All chat models |
| 19 | **Tool-call and JSON canaries** against a noise floor from repeated reference runs | Provider calls; cents to ~$1 | **Medium**; catches broken deployments, not identity | All models. Baselines exist for Kimi and MiniMax; others need generating |
| 20 | **Model Equality Testing** (Hamming-kernel MMD) | Provider calls; under $1 | **Medium**; coarse swaps only | All models with reference samples. Practical for open-weight models ≤14B (local reference) or with a trusted endpoint |
| 21 | **Identity probe** | Provider calls; negligible | **Low**; a system prompt defeats it | All models. Supporting evidence only |

## 5. Hardware attestation

| # | Check | Where | Reliability | Span |
|---|---|---|---|---|
| 22 | **Tinfoil weights attestation:** a dm-verity root of the pinned Hugging Face commit inside the measured config. Verified with the `sev` and `dcap-qvl` crates, Sigstore, and a pinned attested TLS key | Third-party | **High**, except against a provider with physical control of the hardware (TEE.fail) | Tinfoil-hosted models only: gpt-oss-120b, Llama 3.3 70B, Kimi, Gemma, DeepSeek |
| 23 | **Phala (ACI) and NEAR AI:** an attested platform plus signed per-response receipts | Third-party | **High** for "ran in a genuine TEE"; **does not bind the weights** | Their open-weight catalogs: DeepSeek, Qwen, Kimi, GLM, gpt-oss, Llama |

## 6. Not possible today (watch list only, no code)

| Check | Blocker | Span if it ever works |
|---|---|---|
| Text watermarks (OpenAI textGrain, Anthropic, Google SynthID) | Detection keys and APIs are private or restricted | One vendor each |
| FlashProve zero-knowledge proofs | No provider ships proofs; Llama 4 Maverick only, greedy decoding, ≤1,024 tokens | MoE open-weight models served by a cooperating provider |
| TOPLOC, PoSP, Verde, IMMACULATE | No public API exposes the artifacts | Open-weight models with a cooperating provider |
| zkML (DeepProve, EZKL) | Not at LLM scale | Small models |

---

## Coverage by model family

| Model family | Strongest available | Fully-local fallback |
|---|---|---|
| Claude via a reseller | #11a + #12 | None for token counts; #13 by spending tokens |
| GPT via a reseller | #11b (after confirmation) | #1 tiktoken accounting |
| Gemini via a reseller | #11c + #12 | None; #13 by spending tokens |
| Grok | #11d (unknown) | #5 only |
| Open-weight on hosts | #22 if Tinfoil-hosted; otherwise #16 + #9 | #1, #2, #3, #10 |
| Kimi, MiniMax | #19 vendor baselines + #1 | #1 + #10 |
| First-party direct (OpenAI, Anthropic, Google) | Low risk; #16 / #5 for change detection | #1 where a tokenizer exists |
| Local models | #8 | #8 |

Three gaps follow from the tables:

- **Universal but statistical:** #1, #4, #13, #14, #18 and #21 apply to every model, but none is
  cryptographic.
- **Cryptographic but narrow:** each cryptographic check is limited to one vendor (#11a–d) or one
  host's catalog (#22, #23).
- **Quantization** is caught only by #16, #22, or the provider's own declaration (#9).

---

## Implementation order (when work starts)

Do not start implementation until the user approves it. When work begins, follow this order:

1. **Stage 0: passive, no network, no cost.** Checks #1–#10.
   - Plus the verdict engine, Integrity tab, badges and warning modal from the verification plan.
2. **Confirmation run, before Stage 1.**
   - About 20 requests against Anthropic, OpenAI and Gemini with the user's own keys, under
     **~$0.15 in total**.
   - **Needs explicit approval and keys.** It records the exact replay error bodies for #11a–d as
     test fixtures.
   - Parse structured `code`/`type` first. Any message-text parsing goes in one versioned table.
3. **Stage 1: vendor replay.** Checks #11a–d and #12.
   - Includes carrying Gemini `thoughtSignature` through `gemini.rs` and the OpenAI-compatible
     `extra_content` path.
4. **Stage 2: active audits.** Opt-in, with the cost shown first and a budget cap.
   - Order: #13 → #14 → #16 → #12 cross-check → #18 → #19 → #17 (once its code is released) →
     #15 / #20.
   - Probe banks are private, randomized and rotated each release.
5. **Stage 3: hardware attestation.**
   - ACI (Phala) → NEAR AI → Tinfoil.
   - UI states: "Platform-attested" for ACI and NEAR, "Weights-attested" for Tinfoil.
6. **Stage 4: watch list.** No code until an artifact exists to verify.

## Constraints

- **Privacy:** only Stage 0 runs without user action.
  - Every check that calls a provider, vendor or third party runs only when the user clicks
    "Run check", or after an explicit per-provider opt-in schedule.
  - Vendor replay (#11) uses **synthetic probe prompts by default**. Replaying sampled real
    requests sends conversation content to the vendor, so it is a separate opt-in that names that
    vendor.
- **Money:** any check that spends tokens shows its estimated cost before it runs and respects the
  per-provider monthly cap. Build-time costs need approval each time they are incurred: KBF
  enrolment (~$1 per model) and reference generation.
- **Licences:**
  - Reimplement IRIS (non-commercial code).
  - Attribute the OpenRouter export (CC BY 4.0).
  - Do not bundle Artificial Analysis data (its licence forbids redistribution).
  - The Tinfoil and `teep` verifiers are AGPL-3.0, which is compatible with LocalRouter's licence.
- **Config:** new settings use `#[serde(default)]` and never rename serde variants.

## Todo list (first step of implementation)

When implementation starts:

- Create one todo item per stage above, plus one for the confirmation run, one for the website
  section and docs, and one for each final step below.
- Keep the list updated as work proceeds.

## Final steps (mandatory, per stage)

1. **Plan review:**
   - Check each implemented row against this catalog: its "Where", reliability wording and span.
   - Check the UI wording rule.
2. **Test coverage review:**
   - Fixtures for every replay outcome.
   - Accounting residual maths.
   - Verdict rules.
   - Budget enforcement.
   - Opt-in gating for every networked check.
3. **Bug hunt:**
   - Hot-path blocking in request finalization.
   - Alert storms.
   - Leaking probe or replay content into logs.
   - Stale references after model updates.
4. **Validate and push:** run the local checks from AGENTS.md for every area touched, then commit
   and push.
