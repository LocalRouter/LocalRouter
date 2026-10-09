<!-- @entry decision-routing-abstract -->

**LocalRouter original research · October 8, 2026 · Experiments conducted with Codex on a local Apple M2 Max.**

Can a small decision model replace a fixed strong/weak classifier? We tested the existing RouteLLM implementation against Laya and Kev, then investigated a different product question: can users define *when* each model should be used, independently of which model is supposedly strongest?

The distinction matters. Predicting which model will answer correctly requires performance evidence for that model pair. Following “use this model for planning” requires matching the request to a user preference. An explicit client setting such as `mode=plan` requires neither prediction nor inference.

Our experiments found fast local inference, but inconsistent semantic routing. Kev matched 15 of 16 specialist policies and only 19 of 28 workflow policies. Both decision models frequently ignored authoritative mode metadata when the prompt suggested a different route. The resulting design uses **exact rules for supplied client mode, configurable semantic questions where interpretation is needed, and explicit defaults for failures**. We do not claim the tested models reliably improve answer quality over RouteLLM.

<!-- @entry decision-routing-method -->

Hardware: Apple M2 Max, 96 GiB RAM. We used existing cached weights:

- **RouteLLM:** the actual Rust/Candle classifier using `routellm/bert_gpt4_augmented`, with Metal and its existing 512-token prefix behavior.
- **Laya English:** 421M, served by the installed Ollaya 0.7.5 with Metal. This is not a test of every Laya checkpoint or the latest Ollaya release.
- **Kev 0.8B:** dedicated MLX engine, bf16, pinned engine revision and shipped temperature recorded with the results.

The first corpus contained 232 cases: six existing UI examples, seven verification prompts, the repository's 200-example GSM8K fixture with historical model correctness, and 19 authored stress cases. The second contained **72 frozen policy cases**: workflow, specialist, custom support and explicit metadata rules. It included five uses of repository UI examples plus authored boundaries, contradictory instructions, multilingual requests and conversation histories.

We recorded raw answers, failures and wall-clock latency, with three warmups per run. Controls reversed option order and varied context. A terse strong/weak prompt was introduced after observing initial results and is explicitly exploratory. All policy labels and questions were frozen before their runs. No private conversations or cloud inference were used. Tests ran sequentially on a development machine with other activity; latency is not a dedicated serving benchmark.

<!-- @entry decision-routing-quality -->

The historical fixture compares Mixtral-8x7B-Instruct-v0.1 with GPT-4-1106-preview. At a descriptive budget of exactly **100 of 200 requests sent to the strong model**, using each classifier's highest scores:

| Router and question | Historical answers correct | Warm p50 / p95 |
|---|---:|---:|
| Existing RouteLLM | 152/200 (76.0%) | 13 / 22 ms |
| Laya, descriptive difficulty | 143/200 (71.5%) | 29 / 43 ms |
| Kev, descriptive difficulty | 153/200 (76.5%) | 68 / 131 ms |
| Laya, named model pair | 146/200 (73.0%) | 40 / 62 ms |
| Kev, named model pair | 141/200 (70.5%) | 69 / 104 ms |
| Kev, terse difficulty, exploratory | 155/200 (77.5%) | 61 / 106 ms |

Always weak achieved 62.5%; always strong 83.0%. Random selection at the same strong-request fraction would average 72.75%. The per-item correctness oracle reached 92.5%, because the weak model sometimes answered correctly when the strong model did not.

![Historical correctness versus fraction routed to the strong model](/research/decision-model-routing/routing-curve.png)

[Download the chart as SVG](/research/decision-model-routing/routing-curve.svg).

Kev's descriptive advantage over RouteLLM was **one question**. This does not establish superiority. Scores were ranked over the full fixture; these are neither held-out threshold results nor newly generated answers. Equal strong-request fractions do not imply equal dollar cost. Carrying the old 0.3 threshold onto Kev also changed the routing rate dramatically, so thresholds cannot be transferred between classifiers.

<!-- @entry decision-routing-policies -->

The second experiment asks whether the classifier follows explicit user descriptions. Each option is a stable route label mapped separately to a destination model.

| Policy | Laya correct | Kev correct |
|---|---:|---:|
| Planning / implementation / review / general | 20/28 (71.4%) | 19/28 (67.9%) |
| Code / writing / analysis / general | 13/16 (81.3%) | 15/16 (93.8%) |
| Billing / technical / cancellation / general | 8/14 (57.1%) | 11/14 (78.6%) |
| Exact mode rule: plan → Astra, otherwise Sol | 5/14 (35.7%) | 7/14 (50.0%) |
| Total | 46/72 (63.9%) | 52/72 (72.2%) |

Laya rejected both long histories; these count as unsuccessful. Successful warm p50/p95 latency was **31/49 ms for Laya and 56/98 ms for Kev**. A direct equality rule matched all 14 synthetic mode cases without inference. This validates rule evaluation on already-normalized metadata, not detecting mode from a real client.

This direction has research precedent: [Arch-Router](https://arxiv.org/html/2506.16655v1) studies natural-language route policies independently of downstream model assignments, and [DigitalOcean's Inference Router](https://www.digitalocean.com/blog/inference-router-architecture) develops that approach further. Those are external evaluations; we did not run their checkpoints here.

<!-- @entry decision-routing-failures -->

**History can overpower the latest request.** Kev selected planning for “Now implement it” after a plan and for “Thanks, just say goodbye” after a planning discussion. Removing history increased workflow agreement to 23/28, but lost the meaning of follow-ups such as “Yes, go ahead.” Latest-only input is not a complete fix.

**Quoted keywords and conflicting instructions matter.** “The billing page throws error 500; payments are correct” was routed to billing by both models. Instructions inside a request to choose a particular routing label also caused mistakes.

**Option order changes results.** Reversing the policy choices changed 9/70 valid Laya predictions and 2/72 Kev predictions. Reproducible ordering avoids accidental changes but does not establish correctness.

**Probability is not a quality guarantee.** A minimum top-option probability of 0.7 accepted 31/72 Kev cases; only 23 were correct. The remaining 41 needed a fallback. At 0.9, Laya accepted eight cases and got six correct. These are descriptive coverage checks, not independently calibrated thresholds.

The recent [Fast Models, Slow Evidence](https://arxiv.org/html/2610.02267v1) evaluation also cautions against zero-shot performance routing. Its authors explicitly limit that conclusion to the tested labels and models; it is not a general impossibility result for custom policies.

<!-- @entry decision-routing-mode -->

A client mode and a request's semantic intent can disagree. In our cases, mode `default` plus “Write a detailed plan” caused both classifiers to choose Astra, contrary to the configured rule. Fake `mode: plan` text inside the user prompt caused similar failures.

LocalRouter uses a separate metadata field:

```json
{
  "model": "localrouter/auto",
  "metadata": {"localrouter.mode": "plan"},
  "messages": [{"role": "user", "content": "Plan the migration."}]
}
```

`localrouter.mode` is a LocalRouter convention. OpenAI's [Chat Completions API](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create) supports metadata; it does not define a universal plan/edit mode. Codex exposes collaboration mode through its separate [app-server protocol](https://learn.chatgpt.com/docs/app-server), which is not automatically an inference request field. Client integration is required to reliably transmit it.

For “plan → Astra, otherwise Sol,” missing mode means Sol too. Inferring planning from the prompt when mode is missing should be an independently configured semantic policy. Mode is also distinct from sandbox permissions and reasoning effort.

<!-- @entry decision-routing-design -->

The resulting feature replaces the fixed strong/weak abstraction with **Routing policies**:

1. Select a native decision model from configured providers, including hosted Jev or local Laya/Kev/Ollaya models. Local execution is available; selecting a hosted provider sends it the routing context.
2. Start with an editable template: client mode, coding workflow, specialist tasks, quick/thorough, or a custom question.
3. Define the question and options, then assign an ordered destination list to each option. The user owns these mappings; the classifier chooses a route label.
4. Evaluate exact mode rules before semantic inference. Keep role-aware, bounded context and preserve metadata through both Chat Completions and Responses.
5. Validate answers and use a configured default for missing models, errors, timeouts or insufficient probability. Apply ordinary model permissions and provider fallback rules.
6. Preview examples and inspect the selected route, probabilities, source and fallback reason. Do not display unmeasured cost-saving or quality percentages.

A gateway chooses a model for a request. It does not split “plan and implement” into two independently routed phases inside one generated answer. That requires an agent harness issuing separate requests.

The legacy migration preserves the previous model pools and uses the old strong/default list until a new decision model is selected. The old threshold is discarded. A new question cannot inherit the old classifier's calibration.

<!-- @entry decision-routing-limits -->

This is a small, deliberately challenging smoke study, with hand-labelled policy expectations and an old math fixture. It does not measure representative traffic, modern destination-model answer quality, cloud provider performance, cold start, concurrent throughput or end-to-end savings. The semantic variants were not tuned and evaluated on separate held-out conversations. Some boundaries are subjective and depend on the exact configured descriptions.

The original evidence is committed independently of the implementation: [research artifacts at commit 6cc5abd0](https://github.com/LocalRouter/LocalRouter/tree/6cc5abd0/research/strong-weak-2026-10-08). It includes frozen cases, prompts, raw responses and errors, environment versions, analysis scripts and summaries. [Policy report](https://github.com/LocalRouter/LocalRouter/blob/6cc5abd0/research/strong-weak-2026-10-08/POLICY_ROUTING.md) · [Strong/weak report](https://github.com/LocalRouter/LocalRouter/blob/6cc5abd0/research/strong-weak-2026-10-08/README.md).

The next evaluation should compare policy-trained routers and stronger decision models on new held-out conversations, independently test real client mode transmission, and measure destination-model outcomes separately from policy agreement.
