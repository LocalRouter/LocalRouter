# Model Integrity — warn when a provider does not appear to serve the advertised model

**Status: plan only (2026-11-04). Nothing implemented.**

**The mechanism catalog, per-provider matrix and staged adoption path are in
`plan/2026-11-04-MODEL_INTEGRITY_RESEARCH.md`, which overrides this plan's tier table and
phases.** Its main additions:

- Vendor replay oracles: Anthropic thinking signatures, OpenAI `encrypted_content`, and Gemini
  `thoughtSignature`.
- Hardware attestation: Tinfoil binds the weights; ACI/Phala and NEAR attest the platform.
- Provider-declared quantization and routing enforcement (OpenRouter).
- Bundled third-party fidelity data.
- Better audits: KBF, One-Token JSD, logprob drift, AgentProv, GateScope.

This document remains the reference for the verdict engine, UI, warning modal, config, Tauri
commands, and website section, and for FlashProve as a watch-list item.

Providers and resellers sometimes serve something other than the model they bill for: a cheaper
model, an aggressive quantization, an old revision, or a broken deployment (wrong chat template,
broken tool-call parsing). LocalRouter sits on every request, so it can gather evidence about what
it is actually talking to, and warn the user when that evidence points away from the advertised
model.

This plan adds a **Model Integrity** feature. It collects four kinds of evidence, at very
different costs:

| Tier | Evidence | Extra network | Strength | Availability today |
| --- | --- | --- | --- | --- |
| 0 | **Passive signals** on real traffic (tokenizer fingerprint, response `model` field, `system_fingerprint`, throughput profile) | none | catches a switch of model family; misses a switch inside one family | every provider |
| 1 | **On-demand statistical audits** (identity probe, Model Equality Testing MMD, behavioral canary suite) | a few hundred requests per audit | catches gross substitution, broken deployments and heavy quantization; weak against FP8/INT8 | every provider, against a reference |
| 2 | **Logprob probes** (teacher-forced next-token distributions vs a reference) | ~20 requests | strongest black-box signal | only providers that return logprobs (OpenAI some models, Together, llama.cpp, Ollama) |
| 3 | **Cryptographic proof** (Anthropic FlashProve, ZK proof the reply came from committed weights) | ~19 MB download per proof | proof, not a heuristic | **no provider ships proofs yet**; Llama 4 Maverick only, greedy decoding only |

The UI collapses all of this into a per-provider-model **integrity status** (Verified /
Consistent / Not checked / Suspicious / Mismatch). Raising a warning **modal** takes either one
strong signal or two independent weak signals, and a confirmation re-run must agree.

---

## Todo list (first step of implementation)

Create a todo item per phase below (P0–P3), plus one each for the website section, docs, and the
mandatory final steps. Keep it updated as work proceeds.

---

## Research summary (verified 2026-11-04)

### Anthropic FlashProve (released 2026-11-02)

- Official post: https://www.anthropic.com/research/provable-inference. Repo:
  https://github.com/anthropics/flashprove (Apache-2.0, v0.16.0, "a research artifact: not
  maintained, not accepting contributions", **no binary releases**).
- Authors: Anthropic (Roy Rinberg, Alex Allain, Anthropic Fellows), EPFL, MIT, StarkWare,
  Succinct.
- **What it proves:** a zkSNARK showing that a reply's tokens are the **greedy** choice of an
  **integer** version of a **committed** set of MoE weights, with the weights kept hidden. The
  commitment ID is a 64-hex hash (config, Merkle root, masked weights, OOD seeds). Committing a
  model takes about 39 minutes, and one commitment covers about 16M requests.
- **Scope limits:** MoE only. Llama 4 Maverick (400B, 4-bit) is the only model run. Context is
  ≤1,024 tokens. Greedy decoding only (`temperature: 0`); sampling is not proved. The integer
  model's top token differs from the 16-bit model's at 6.81% of positions, so 0 of 96 stock vLLM
  replies passed exact match. A provider must serve through the repo's integer-model vLLM
  plug-in. A "clipped gap" acceptance policy exists, with client-chosen limits, and passes
  "WITHIN LIMITS" rather than "YES".
- **Prover cost (provider side):** about 70–95× the GPU time of normal inference. About 110
  tok/s of *prefill* at batch 24 on one H100, with ~270–300 GB of host RAM streamed to an 80 GB
  GPU. A batch waits about 223 s. This cost is why no provider will prove every request; at
  best, proofs will be sampled or sold at a premium.
- **Wire format (vLLM middleware, `integrations/vllm/REFERENCE.md`):**
  - Request: `POST /v1/chat/completions` (or `/v1/completions`) with header
    `x-proof-policy: <POLICY>`. The body must contain `"temperature": 0` and
    `"return_token_ids": true`, and messages may hold only `role` and `content`. A request that
    breaks these rules is still served, but its job reads `not_provable`.
  - Response: the normal body, plus `choices[0].token_ids` and the echoed `prompt_token_ids`,
    plus the header `x-proof-id: <64 hex>`.
  - Fetch (all under `/plugins/proof/`): `GET jobs/<id>` returns
    `{state: waiting|proved|no_proof|not_provable|expired, request: {total_tokens, prompt_tokens, reply_tokens, padding_token}}`.
    Then `GET jobs/<id>/proof.bin` and `GET jobs/<id>/y.bin`, then `DELETE jobs/<id>`.
    `GET published/<POLICY>/{key.bin,layout.txt,leaf/K.bin}` serves the public files. Status
    codes are 401/404/409/500/503 (on hold). The proof ID acts as a bearer handle.
  - **The commitment ID must never be taken from the server being checked.** There is no
    public registry; it must come from a party the user trusts.
- **Verifier cost (our side):** the C++ `cp_verify` takes ~17 s on 24 threads and ~21 s on 1
  thread, with a peak of ~3.53 GB RAM. Proofs are **18.78 MB each**, plus ~6.5 MB of public key
  material once per commitment. No GPU and no weights are needed. The verifier is **Linux-only
  as documented** (g++ 13+). There is no macOS, Windows or WASM build and no library API. Exit
  code 0 *and* a last line of `YES` / `WITHIN LIMITS: …` count as a pass. Inputs are files:
  `REQUEST_FILE` (LE u32: prompt IDs, reply IDs, padding up to `total_tokens`, then
  `START=P-1`, `END=P+R-1`) and `TOKEN_FILE` (`y.bin`). The client needs no tokenizer, but it
  must trust the chat template that produced the prompt IDs.

### Black-box detection research

- **Cai et al., "Are You Getting What You Pay For?"** (arXiv 2504.04715,
  code `sunblaze-ucb/llm-api-audit`):
  - Text classifiers stay at ~50% accuracy against FP8/INT8.
  - Identity prompting fails within a model family, and a system prompt defeats it.
  - MMD detects randomized substitution when p ≥ 0.6, but is at chance when p ≤ 0.2.
  - Benchmark deltas from quantization fall within one standard deviation.
  - Logprob checks suffer from batch nondeterminism: "even at T=0, greedy decoding can diverge
    after only tens of tokens".
  - Logprob availability: Together full; OpenAI depends on the model; Anthropic and Mistral
    none.
  - Conclusion: software-only methods are unreliable against *subtle* substitution.
  - Implication for us: never present a statistical verdict as proof, and calibrate for
    nondeterminism.
- **Gao, Liang, Guestrin, "Model Equality Testing"** (arXiv 2410.20247, `i-gao/model-equality-testing`):
  - Two-sample MMD with a Hamming kernel over 50-token completions at T=1.
  - N = 10 samples × m prompts (m = 20–25) gives ~77% median power. N = 50m reaches ~1.0
    against nf4/int8/watermarks.
  - Audited 31 endpoints and flagged 11, at **under $1 per audit**.
  - Needs a reference sample from trusted weights.
- **Moonshot K2 / Kimi Vendor Verifier** (`MoonshotAI/K2-Vendor-Verifier`):
  - 4,000 tool-call requests, compared against the official API on tool-call trigger F1 and
    schema accuracy.
  - Thresholds are set **from repeated runs of the official API** (e.g. min 75.81% → accept
    ≥73%).
  - Some vendors measured 68% vs a 76% baseline.
  - This is our model for "broken deployment" detection and for noise-floor calibration.
- **LLMmap** (arXiv 2407.15847): identifies 42 model versions at >95% accuracy from **8
  queries**. It needs pre-trained fingerprints per candidate, and it answers "which model is
  this", not "is this degraded".
- **TOPLOC** (arXiv 2501.16007): an activation-hash proof at 258 B per 32 tokens. It needs
  provider cooperation, and the verifier must re-run the model. Not usable here.

### Codebase facts this plan rests on

- **Request path:**
  - `crates/lr-server/src/routes/chat.rs:53` → `Router::complete`/`stream_complete`
    (`crates/lr-router/src/lib.rs:2597`/`:2712`) → `execute_request_inner` (`:1408`).
  - Finalization: `crates/lr-server/src/routes/finalize.rs:275`
    (`finalize_metrics_and_monitor`), and `:505` for streaming.
- **Provider trait:** `ModelProvider` (`crates/lr-providers/src/lib.rs:181`).
  `supports_feature("logprobs")` returns true for openai, togetherai, ollama, llamacpp and
  embedded llamacpp. `CompletionRequest.logprobs/top_logprobs/seed` are at `lib.rs:1693+`.
- **Existing direct-probe pattern:** System One letter mode (`crates/lr-router/src/systemone.rs:586`)
  builds its own logprob request and calls the provider directly. Probes copy this.
- **Periodic job pattern:** the provider health loop in `src-tauri/src/main.rs:2120-2260`
  (interval, enable flag, timeout, per-provider multiplier) and `HealthCheckConfig`
  (`crates/lr-config/src/types.rs:~1070`).
- **Catalog:** `CatalogModel` has `open_weights` but no HF repo, parameter count or
  quantization. The models.dev snapshot has `canonical_model_id` and `family`, which are not yet
  parsed (`crates/lr-catalog/buildtools/models.rs:45`).
- **Popup windows:** `src/views/firewall-approval.tsx` plus the shared
  `src/components/shared/FirewallApprovalCard.tsx`. Popup types are listed in
  `SamplePopupButton.tsx` and opened via `debug_trigger_firewall_popup`. The website reuses the
  shared card (`website/src/components/FreeTierFallbackDemo.tsx`).
- **Global event listeners:** `src/App.tsx:95-240`. Events are emitted through
  `AppState::emit_event` (`crates/lr-server/src/state.rs:1292`).
- **Monitor events:** `MonitorEventType` (`crates/lr-monitor/src/types.rs:121`).
- **Per-provider config:** the free-form `provider_config` JSON has a precedent key,
  `localrouter_privacy_attestation` (`crates/lr-providers/src/privacy.rs:8`). Global config
  lives in `AppConfig` (`types.rs:420`), at `CONFIG_VERSION = 33`.
- **Local inference** for references: embedded llama.cpp (`crates/lr-providers/src/embedded/llamacpp.rs`,
  which supports logprobs) and HF download in `crates/lr-local-models`.

---

## Design

### Privacy constraints (CRITICAL)

AGENTS.md only permits network requests that come from user actions or optional update checks.

- **Tier 0 is purely passive.** It analyses traffic the user already sent, and makes no new
  requests.
- **Tiers 1–3 run only when the user starts them**: a "Run integrity check" button, or an
  explicit opt-in schedule that the user turns on per provider, with the cost shown first.
- **Default is off for scheduled checks and on for passive signals.**
- **Reference data** (prompt pools, reference sample bundles, tokenizer fingerprints) is
  **bundled at build time** or downloaded only when the user enables checks for a model. Each
  download states its size and source.
- **Probe prompts are synthetic**, never user content.
- **Telemetry:** nothing is reported anywhere. Results stay in the local DB.

### New crate: `crates/lr-integrity/`

```
lr-integrity/
├── src/lib.rs             # IntegrityService: status store, verdict engine, public API
├── src/status.rs          # IntegrityStatus, Signal, Evidence, Verdict types (serde + ToSchema)
├── src/passive/
│   ├── tokenizer.rs       # prompt-token-count fingerprint (Tier 0)
│   ├── metadata.rs        # response model field, system_fingerprint, OpenRouter endpoint metadata
│   └── throughput.rs      # tokens/s + TTFT profile vs catalog expectation
├── src/probes/
│   ├── identity.rs        # LLMmap-style 8-query probe (Tier 1)
│   ├── mmd.rs             # Model Equality Testing, Hamming-kernel MMD + permutation test (Tier 1)
│   ├── behavior.rs        # canary suite: tool-call F1, schema accuracy, JSON mode, exact-answer set (Tier 1)
│   └── logprob.rs         # teacher-forced next-token distribution comparison (Tier 2)
├── src/reference/
│   ├── bundle.rs          # bundled reference samples + noise floors (zstd JSON)
│   └── local.rs           # generate reference via embedded llama.cpp
├── src/proof/flashprove.rs# x-proof-policy client, job polling, file assembly, cp_verify subprocess (Tier 3)
├── src/budget.rs          # cost estimator + per-provider monthly cap
└── src/store.rs           # SQLite tables integrity_runs, integrity_signals
```

### Tier 0: passive signals (P0)

Every finalized request is sent to `IntegrityService::observe(...)` from `finalize.rs`. The
handoff uses a **bounded mpsc channel with try_send**: the hot path never blocks, and drops are
counted.

1. **Tokenizer fingerprint (strongest passive signal).**
   - For models with a known public tokenizer, re-count the prompt locally and compare with the
     provider's `usage.prompt_tokens`. Known tokenizers: open-weights models via HF
     `tokenizer.json` and the `tokenizers` crate; OpenAI via `o200k_base`/`cl100k_base`.
   - The chat template adds a constant overhead per message, calibrated per model:
     `expected = a + b·messages + count(content)`.
   - A residual beyond tolerance on ≥5 of the last 20 requests means a *different tokenizer*,
     i.e. a different model family.
   - It cannot see a switch inside a family that shares a tokenizer (e.g. Llama 70B → 8B).
   - Tokenizers are not bundled (5–10 MB each). They download only when the user enables
     integrity checks for that model, through the existing hub client in `lr-local-models`.
   - Anthropic models: if the user also has a first-party Anthropic key, the free
     `count_tokens` endpoint is the reference. It is optional and user-enabled, because it is a
     network call.
2. **Metadata.**
   - The response `model` field disagrees with the requested model, after normalizing aliases
     and dated snapshots through `lr-catalog` matching.
   - The OpenAI `system_fingerprint` changes. This is informational only.
   - OpenRouter's served provider and declared quantization (endpoint metadata; **verify the
     current field names during implementation**) differ from the user's expectation.
3. **Throughput profile.** Output tokens/s and TTFT far outside the provider-model's own history.
   This is a weak signal, used only to corroborate another signal and never to alert on alone.

### Tier 1: on-demand audits (P1)

All probes call the provider directly, bypassing routing, like `systemone.rs:586`. They use the
`internal-integrity` client id, so they are excluded from user metrics and routing, but they
appear in Monitor as `ModelIntegrity` events. On OpenRouter, probes pin the provider with
`provider.only`.

1. **Identity probe.** 8 fixed queries, scored against bundled fingerprints, LLMmap-style. It
   is cheap and catches gross substitution ("gpt-4o" answering like a small Llama). Its result
   is shown as "Looks like: X (confidence)".
2. **Model Equality Testing (MMD).**
   - Draw m = 25 prompts at random from a bundled pool of ~500 prompts, so a provider cannot
     simply detect and special-case an audit prompt set.
   - Request N = 10m completions of 50 tokens at T = 1 (escalating to N = 50m on a borderline
     result).
   - Run a Hamming-kernel MMD with 1,000 permutations against the reference sample.
   - Repeat over 10 random subsamples, and reject if the mean rejection rate is ≥ 0.5 (as in Gao
     et al.).
   - The null is calibrated by comparing the reference with a *second* reference draw. That
     estimates the endpoint-nondeterminism floor Cai et al. warn about.
3. **Behavioral canary suite.** It catches broken deployments, not identity.
   - 60 tool-call prompts (trigger F1 + schema accuracy), 20 JSON-mode prompts, and 40
     exact-answer prompts (arithmetic, string manipulation, factual, with deterministic answers).
   - Scores are compared with the reference noise floor. Following K2VV, the floor is the
     minimum over 3 reference runs, minus a margin.
   - The suite also checks the advertised context window with a single needle at 90% of the
     window. That check is the most expensive part, so it is opt-in.

**Where references come from (most to least trusted):**

1. **A first-party endpoint the user already has a key for**, e.g. Moonshot for Kimi, DeepSeek
   for DeepSeek, OpenAI for GPT. The user picks "compare against" in the UI.
2. **Local weights** through embedded llama.cpp.
   - Only realistic for ≤8–14B models; see the cost table.
   - The reference must be F16/BF16 to detect quantization. A Q8 reference still detects
     substitution, and the UI says so.
3. **A bundled reference bundle** shipped with the release, for the top ~30 models. Per model:
   the reference samples, behavioral noise floor and identity fingerprint.
   - Generated by `scripts/integrity/build-references.py` against first-party APIs.
   - **This script spends money on API calls, so every run needs the user's approval.**
   - The bundle records the date and endpoint it was generated against. References go stale
     when the upstream model is updated, so the UI shows the reference date.

Closed models (Claude, GPT, Gemini) reached through a reseller can only use references 1 and 3.
Closed models from their own first party are out of scope by default (there is nothing to
compare against), but the passive checks still run.

### Tier 2: logprob probes (P2)

Teacher-forced checks avoid the divergence problem: send fixed prefixes and compare the
**next-token top-k distributions** at fixed positions, rather than comparing generations.

- Statistics: top-1 agreement rate and mean KL over the shared top-k support.
- Runs only where `supports_feature("logprobs")` holds on both the endpoint and the reference.
  The reference can be local llama.cpp or a logprob-capable first-party endpoint.
- Thresholds are calibrated from repeated reference runs (batch nondeterminism, Cai §4.1.6).
- Repeats at `seed` / `temperature: 0` where the provider supports them.
- Cost: ~20 prompts of ~256 tokens with `max_tokens: 1`. Very cheap, and the local reference
  needs only prefill.

### Tier 3: FlashProve verifier (P3, experimental)

The value is future-facing, because no provider ships proofs today. The aim is to be ready when
providers adopt it, and to be credible on the website.

- **Per-provider setting "Proof policy":**
  - Policy name, commitment ID, and the source of the commitment ID.
  - The UI requires a source other than the provider itself, and warns that values from
    `GET /plugins/proof/policies` must not be trusted.
  - Exact match is the default. "Clipped gap" requires the user to enter C/s/theta/cap, and is
    labelled weaker.
- **Flow:**
  1. A **dedicated probe request** goes out with `x-proof-policy`, `temperature: 0`,
     `return_token_ids: true`, and a prompt of ≤1,024 total tokens. User requests usually break
     the provable constraints (sampling, tools, long context), so a dedicated probe is needed.
     Opt-in "prove eligible user requests" can come later.
  2. Keep the `x-proof-id`, the prompt IDs and the reply IDs. Check the echoed
     `prompt_token_ids` against what was sent.
  3. Poll `jobs/<id>` with back-off: 5 s, doubling to a 60 s cap, giving up after 30 min.
  4. When the state is `proved`, download `proof.bin` (check Content-Length) and `y.bin`, then
     `DELETE` the job.
  5. Build `REQUEST_FILE` and run `cp_verify` in a temp dir as a subprocess, at low priority
     (`nice`), one at a time, with a 5-minute timeout.
  6. A pass requires exit code 0 **and** a `YES`/`WITHIN LIMITS` line. A proof failure is a
     **strong** signal (a single one can raise the alert).
- **Platform:**
  - Linux: build `cp_verify` from a pinned commit in CI and ship it as a sidecar
    (verify its license notice and reproducibility).
  - macOS: **spike first.** Attempt a Homebrew g++-13/clang build. If it does not port cleanly,
    the feature is Linux-only, or the user points LocalRouter at their own `cp_verify` path.
  - Windows: user-supplied path (WSL) only.
- **Test fixture:** the repo's synthetic-weight fixtures, not a real Maverick deployment. A real
  prover needs an H100 and ≥330 GB of host RAM, and **renting one costs money, so it needs the
  user's approval first.**

### Verdict engine

Each signal produces `{kind, strength: weak|medium|strong, outcome: pass|inconclusive|fail, value, threshold, evidence}`.

| Status | Rule |
| --- | --- |
| **Verified** | FlashProve pass within the last 24 h. This status is reserved for proofs and never shown for statistics. |
| **Consistent** | At least one medium-or-better check passed recently, and nothing failed. |
| **Not checked** | No signals yet. |
| **Suspicious** | 1 medium fail, or ≥2 weak fails. Shown as a badge only, with no modal. |
| **Mismatch** | 1 strong fail, or ≥2 independent medium fails, **and** the automatic confirmation re-run agrees. Raises the modal. |

Strong signals: FlashProve fail, tokenizer mismatch over ≥5/20 requests, and an MMD reject
confirmed on N = 50m. Medium signals: MMD reject at N = 10m, behavioral score below the noise
floor, and a logprob divergence above threshold. Weak signals: identity, metadata and
throughput.

A confirmation run (e.g. a fresh MMD draw) happens automatically only if the user has opted into
scheduled checks for that provider. Otherwise the badge shows "Suspicious — confirm?" with a
button.

Alerts are de-duplicated per (provider, model, signal-set). After the user picks "Keep using",
the modal is snoozed for 7 days.

### Warning modal

It surfaces through the **existing popup window** (`firewall-approval.tsx`) as a new popup type
`model_integrity`, so it appears even when the main window is closed (a tray app). When the main
window is focused, `App.tsx` shows the same content in a Radix `AlertDialog`. Both render the new
shared `src/components/shared/ModelIntegrityWarningCard.tsx`. The website demo renders the same
component.

```
┌──────────────────────────────────────────────────────────────┐
│ [ShieldAlert]  This model may not be what the provider says  │
│                                                              │
│ budget-provider  ·  advertised: llama-3.3-70b-instruct       │
│                                                              │
│ Evidence (3 checks, last 2 h)                                │
│  ✗ Tokenizer       prompt tokens off by +18% on 14/20 reqs   │  strong
│  ✗ Output distribution  MMD p<0.001 (N=250, confirmed N=1250)│  strong
│  ~ Tool calling    F1 64% vs reference ≥ 73%                 │  medium
│  ✓ Response model field matches                              │
│                                                              │
│ Looks like: a smaller model from a different family (62%)    │
│ Statistical evidence — not proof. Reference: first-party     │
│ API, generated 2026-10-28.  Audit cost so far: $0.04         │
│                                                              │
│ When this happens:  (•) Warn  ( ) Ask before each request   │
│                     ( ) Stop routing to this model           │
│                                                              │
│ [Pause model]  [Re-test now]  [Keep using (7 days)]  [Details]│
└──────────────────────────────────────────────────────────────┘
```

Icons come from lucide (`ShieldAlert`, `ShieldCheck`, `ShieldQuestion`), with no emoji. The
✓/✗ glyphs above are illustrative and render as lucide `Check`/`X`/`CircleDashed`.

**Actions:**

- **Pause model:** disables the provider-model in routing, reusing the existing per-model
  enabled setting in `ProviderModelSettings`.
- **Re-test now:** runs the Tier 1 suite. The estimated cost is shown on the button.
- **Keep using:** snoozes the warning.
- **Details:** opens Provider → Integrity tab.
- **"Ask before each request":** reuses the firewall hold path, like the free-tier fallback
  popup. Requests to this provider-model are held for approval until the status clears.
- **"Stop routing":** marks the provider-model unhealthy for routing, so auto-router and
  prioritized lists fall through to the next model.

The modal text always says **"Statistical evidence — not proof"** unless the evidence is a
FlashProve failure, in which case it says **"Cryptographic proof failed"**.

### Other UI

- **Provider detail:** a new **Integrity** tab (`src/views/resources/providers-panel.tsx:523`).
  It holds:
  - The status per model.
  - The signal history.
  - "Run check" with a cost estimate.
  - The reference source picker.
  - The schedule opt-in (off, daily, weekly) and the monthly budget cap (default $1).
  - The FlashProve policy settings.
- **Model lists:** a status badge next to `ModelPricingBadge` (`src/views/resources/models-panel.tsx:269/360`)
  and in `ProviderModelsTab.tsx`.
- **Settings → Model Integrity:**
  - The global switches: passive signals (on), scheduled audits (off), default action
    (Warn), tokenizer downloads (ask).
  - The FlashProve verifier binary path, and its status (found, version).
- **Monitor:** a new `MonitorEventType::ModelIntegrity` (`crates/lr-monitor/src/types.rs:121`)
  with run and verdict details.
- **Debug:** add `model_integrity` to `debug_trigger_firewall_popup` and `SamplePopupButton`.

### Config

- `AppConfig.model_integrity: ModelIntegrityConfig`, `#[serde(default)]`, so **no migration**:
  - `passive_enabled: bool = true`
  - `scheduled_enabled: bool = false`
  - `default_action: Warn|Ask|Block`
  - `monthly_budget_usd: f64 = 1.0`
  - `flashprove_verifier_path: Option<String>`
- Per provider: stored in `provider_config` under the key `localrouter_model_integrity`, the same
  approach as privacy attestation.
  - `schedule`
  - `reference: {kind: first_party|local|bundled, provider_instance?, model?}`
  - `action_override`
  - `flashprove: {policy, commitment_id, commitment_source, acceptance: exact|clipped_gap{c,s,theta,cap}}`
- New enum variants: `#[serde(rename_all = "snake_case")]`, and never rename them later
  (see memory: config compatibility).

### Tauri commands (update Rust, `src/types/tauri-commands.ts`, `TauriMockSetup.ts`)

- `get_model_integrity_status(provider_instance: Option<String>) -> Vec<ModelIntegrityStatus>`
- `get_model_integrity_history(provider_instance, model, limit) -> Vec<IntegrityRun>`
- `estimate_model_integrity_check(provider_instance, model, tiers) -> IntegrityCostEstimate`
  (tokens, USD, local CPU-seconds, download MB)
- `run_model_integrity_check(provider_instance, model, tiers) -> IntegrityRunId` (async; emits
  `model-integrity-updated` events)
- `set_model_integrity_settings(...)`, `set_provider_integrity_settings(...)`
- `acknowledge_model_integrity_alert(alert_id, action: pause|snooze|keep|ask|block)`
- `test_flashprove_verifier(path) -> FlashproveVerifierInfo`

**Events:**

- `model-integrity-updated`: badges refresh.
- `model-integrity-alert`: the modal or popup opens.

### Website

**Landing page section** in `website/src/pages/Home.tsx`, placed **directly after "Free-Tier
Mode" (`:535`)**. Both sections are about getting what you pay for. Swap the `bg-muted/30`
alternation of the following sections so the shading keeps alternating.

- Layout follows the Free-Tier section: the visual on the left (`order-2 lg:order-1`), the text
  on the right.
- **Visual:** a dark "Integrity report" panel (header dot + "Model Integrity" label + "passive
  checks: on" chip) listing three generic provider rows:
  - `first-party` · Verified · `ShieldCheck`, shown with a "proof" chip to illustrate FlashProve.
  - `fast-host` · Consistent · tokenizer ✓ / MMD ✓.
  - `budget-reseller` · Mismatch · tokenizer ✗ / distribution ✗ / tool-calls 64%.
  - Overlaid bottom-right at `scale-[0.8]`: a **`ModelIntegrityWarningDemo`**
    (`website/src/components/ModelIntegrityWarningDemo.tsx`). It renders the real
    `@app/components/shared/ModelIntegrityWarningCard` with no-op actions, the same way
    `FreeTierFallbackDemo` does.
  - **Use generic provider names**, never a real provider's name or logo, in a "mismatch"
    illustration. Showing a real company as substituting models would be an unfounded public
    accusation.
- **Text:**
  - Eyebrow: `ShieldCheck` icon + "Trust & Verification".
  - H2: **"Know the model you're paying for"**.
  - Lede: "Providers and resellers can quietly swap in a cheaper model, a heavier
    quantization, or a broken deployment. LocalRouter checks what's actually answering — and
    warns you when it doesn't match."
  - Bullets (`Check` icons):
    1. **Passive checks on every response.** Tokenizer fingerprints and response metadata, with
       no extra requests.
    2. **On-demand audits.** Statistical model-equality tests and tool-calling canaries against
       a trusted reference, typically for cents.
    3. **Cryptographic proofs, where offered.** Verifies Anthropic's open-source FlashProve
       zero-knowledge proofs locally, with no weights needed.
  - "Learn more →" links to `/docs/model-integrity`.
- **Wording rule:** never claim statistical checks "prove" anything. Only the FlashProve bullet
  uses "proof".
- **Docs:** a new `model-integrity` page in `website/src/pages/docs-content.ts`. It covers:
  - What each tier detects and misses.
  - The cost table below.
  - Privacy (all checks local, all probes opt-in).
  - How to configure a FlashProve policy.
  - The FlashProve limits (MoE/Maverick, greedy, 1,024 tokens).
- **Demo:** mock handlers for every new command in `TauriMockSetup.ts`, so the `/demo` Integrity
  tab shows one Verified, one Consistent and one Mismatch model. A mock `model-integrity-alert`
  emit fires behind a demo button.
- **Optional:** a short Research page entry (`research-content.ts`) summarising the four papers
  and FlashProve, with links.
- Run `npm --prefix website run build:check`, because the website embeds the shared card from
  root `src/`.

---

## Computational and monetary cost

Local numbers are estimates for an Apple-silicon laptop (M2/M3 Pro class) unless stated.
FlashProve numbers are Anthropic's measurements on x86 Linux.

| Check | Runs | Extra provider tokens | Provider $ (cheap / premium model¹) | Local CPU / RAM | Download |
| --- | --- | --- | --- | --- | --- |
| Tier 0 tokenizer count | every request (async) | 0 | $0 | ~0.1–1 ms per request; ~10–40 MB RAM per loaded tokenizer | 5–10 MB per tokenizer, once |
| Tier 0 metadata / throughput | every request | 0 | $0 | negligible | 0 |
| Identity probe | on demand | ~8 req, ~3k tokens | <$0.001 / ~$0.03 | negligible | 0 |
| MMD audit, N=10m (250 completions × 50 tok) | on demand | ~25k in + 12.5k out | ~$0.01 / ~$0.26 | permutation test <1 s | 0 |
| MMD confirm, N=50m (1,250 completions) | only on borderline | ~125k in + 62k out | ~$0.06 / ~$1.30 | ~2–5 s | 0 |
| Behavioral canary suite (120 prompts) | on demand | ~150k in + 30k out | ~$0.04 / ~$0.90 | negligible | 0 |
| Context-window needle (opt-in) | on demand | ≈ 0.9 × context window, e.g. 115k | ~$0.01–0.35 | negligible | 0 |
| Logprob probe (20 prefixes, max_tokens 1) | on demand | ~5k in | <$0.001 / ~$0.02 | — | 0 |
| **Local reference** for MMD (12.5k tokens generated) | once per model, cached | — | $0 | 8B F16: ~16 GB RAM, ~5–10 min at batch 8. 14B F16: ~28 GB, ~15–25 min. **70B+: not feasible on a laptop** (~140 GB) | 16 GB (8B F16) |
| **Local reference** for logprobs (prefill only) | per audit | — | $0 | 8B F16: ~10–30 s | (same weights) |
| **FlashProve verify** (one proof) | per sampled probe | 1 probe ≤1,024 tok | ~$0 (the provider bears 70–95× GPU cost and may charge for it) | **~17 s on 24 threads, ~21 s on 1 thread, peak ~3.5 GB RAM** (x86, Anthropic). Laptop estimate: 20–45 s. | **18.78 MB per proof** + ~6.5 MB once per commitment |
| Bundled reference bundles (build time) | per release | — | ~30 models × N=50m against first-party APIs ≈ **$10–20 per refresh** (needs approval each run) | — | +~2–5 MB app size (zstd) |

¹ "Cheap" ≈ $0.20 in / $0.60 out per M tokens (8–70B open-weights host). "Premium" ≈ $3 / $15 per M.

**What this adds up to under the defaults:**

- **Passive only (default):** effectively zero. The added work is microseconds of tokenizing
  per request on a background task, plus ~50–150 MB of RAM if several tokenizers are loaded.
  There is no network traffic and no cost.
- **Weekly scheduled audit of 5 models** (MMD + canary): about 5 × $0.05–$1.20 per week, i.e.
  **≈ $1–25/month** depending on model price. The default $1/month cap per provider stops
  premium-model audits at roughly one per month. Local CPU is a few seconds per run.
- **FlashProve, if a provider offered it**, at 1 probe per provider-model every 6 h: ~2–3 CPU
  minutes/day, ~75 MB/day downloaded, and transient 3.5 GB RAM spikes. Run one at a time, at
  low priority, never on battery (opt-in override). Disk: proofs are deleted after
  verification by default; "keep last 10" (~190 MB) is optional.
- **The heaviest local option is the local F16 reference model.** It needs ~16 GB RAM for 8B and
  is impractical above ~14B. For large models, prefer a first-party reference or the bundled
  reference.

---

## Honest limitations (shown in docs and the Integrity tab)

- Black-box tests **cannot reliably detect FP8/INT8 quantization** or low-rate random
  substitution (Cai et al.). A "Consistent" status is not a guarantee.
- A provider can recognise audit traffic and serve it correctly. Mitigations: a randomized
  prompt pool, probes shaped like real traffic, and Tier 0 running on *real* traffic.
- **False positives:** batch nondeterminism, provider-side system prompts, and legitimate model
  updates. Mitigations: reference noise floors, confirmation re-runs, the two-signal rule, and
  showing the reference date.
- FlashProve today covers greedy decoding, ≤1,024 tokens and MoE (Maverick) only. It needs the
  provider's cooperation and a commitment ID from a trusted third party. No public registry
  exists.

---

## Phases

- **P0: passive signals, verdict engine, UI and website.**
  - Work: the `lr-integrity` crate skeleton, Tier 0, SQLite store, the
    `model-integrity-updated`/`-alert` events, `ModelIntegrityWarningCard`, the popup type,
    the Integrity tab (status and history), badges, settings, Monitor event type, Tauri
    commands, types, mocks, the landing section, the docs page and the demo.
  - Tests: tokenizer-residual math; the verdict rules table; alert de-duplication and snooze;
    bounded channel drops; a serde round-trip of config defaults; mock and type sync
    (`npx tsc --noEmit`).
- **P1: on-demand audits.**
  - Work: the identity probe, MMD (port the MET statistic and validate it against the paper's
    reference implementation on fixture samples), the behavioral suite, the prompt pool, the
    reference sources (first-party and bundled), the budget and cost estimator, and the
    opt-in scheduler (modelled on the health loop in `main.rs:2120`).
  - Tests: MMD rejects synthetic shifted distributions and keeps the false-positive rate ≈ α on
    identical ones; cost-estimator snapshots; budget-cap enforcement.
- **P2: logprob probes and local references.**
  - Work: teacher-forced logprob comparison, and local reference generation via embedded
    llama.cpp with cached results.
- **P3: FlashProve verifier (experimental).**
  - Work: a macOS build spike for `cp_verify`, the proof client, REQUEST_FILE assembly, the
    subprocess runner, and the policy settings UI.
  - Tests use the repo's synthetic-weight fixtures (any GPU rental needs approval first).

---

## Final steps (mandatory)

1. **Plan review:** re-read this plan against the implementation, and implement anything
   missed (privacy gating, wording rules, the generic names on the website).
2. **Test coverage review:** cover every verdict rule, each probe's error and timeout path,
   budget exhaustion, the FlashProve exit-code/result-line matrix, and config defaults.
3. **Bug hunt:** hot-path blocking in `finalize.rs`, alert storms, races between scheduled and
   manual runs, stale references after model updates, and proof-ID leakage into logs.
4. **Validate and push:**
   - Rust: `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`,
     and targeted `cargo test -p lr-integrity -p lr-server -p lr-config`.
   - Frontend: `npm run test:unit`, `npm run build`, `npx tsc --noEmit`.
   - Website: `npm --prefix website run build:check`.
   - Then commit and push.

## Sources

- FlashProve: https://www.anthropic.com/research/provable-inference · https://github.com/anthropics/flashprove
  (`guide/performance.md`, `guide/how-it-works.md`, `guide/known-limits.md`, `integrations/vllm/REFERENCE.md`, `docs/service.md`)
- Cai et al., arXiv 2504.04715 · https://github.com/sunblaze-ucb/llm-api-audit
- Gao, Liang, Guestrin, arXiv 2410.20247 · https://github.com/i-gao/model-equality-testing
- https://github.com/MoonshotAI/K2-Vendor-Verifier
- LLMmap, arXiv 2407.15847
- TOPLOC, arXiv 2501.16007
