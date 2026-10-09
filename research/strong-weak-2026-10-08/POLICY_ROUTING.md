# Configurable model routing: evidence and proposed behavior

Research date: October 8, 2026, Toronto. Runs completed October 9 UTC. Research only; production behavior is unchanged.

## Recommendation

Replace the fixed **Strong/Weak** feature with **Routing policies**: the user defines when each route applies and assigns its model or ordered model list. Start with exact conditions when the client supplies the answer, and use a local classifier for semantic questions such as “What kind of work is requested?”

For the user's example, `mode == plan → Astra; otherwise → Sol` should be an exact rule. A decision model adds latency and mistakes to this equality check. If instead the user means “when the request asks for planning, even outside plan mode,” that is a semantic policy. These are separate selectable behaviors, with the difference visible in the preview.

The local models are fast enough for experimentation, but the results below do **not** establish reliable automatic workflow switching. Evaluate a router trained for natural-language policies next, alongside any improved System One checkpoint. Retire the old RouteLLM backend after migration and acceptance testing; keeping it permanently is unnecessary. Its replacement should deliver explicit user control without claiming it predicts answer quality.

## What research supports this approach?

**Arch-Router directly studies user-defined natural-language routing policies.** Its 1.5B model chooses a domain/action route independently of the downstream model mapping. The authors report 96.05% aggregated turn accuracy and 88.48% whole-conversation accuracy. Evaluation adapts four public datasets with generated route descriptions and additional annotation; these are author-reported results, not a guarantee for LocalRouter's workflow labels. [Paper](https://arxiv.org/html/2506.16655v1). The released checkpoint is generative, not a native System One endpoint, and has a custom research license that requires review before bundling. [Model card](https://huggingface.co/katanemo/Arch-Router-1.5B).

**DigitalOcean's newer Inference Router/Plano work uses descriptions and conversation context too.** Its authors report 84.68% overall for 4B and 87.84% for 30B-A3B on 1,958 messages from 605 multi-turn conversations; the coding results differ materially between sizes. This is relevant product evidence that preference routing is practical, with substantial remaining errors. It is a vendor evaluation, not a comparison with our local corpus. [Architecture and evaluation](https://www.digitalocean.com/blog/inference-router-architecture).

The available evidence supports **matching requests to preferences**. It does not show that “planning → Astra, implementation → Sol” maximizes quality or reduces cost for a particular workload. Those mappings belong to the user. Measuring policy compliance and measuring resulting answer quality require different labels.

### Checking the research supplied by the other agent

The central warning in **Fast Models, Slow Evidence** is supported: on its 400-case performance-routing task, Jev mostly chose cheap and Laya's AUROC was 0.48. But its authors explicitly limit the conclusion to those zero-shot labels: most are multiple-choice tasks with single-run outcomes, and fine-tuning was not evaluated. This October 1 preprint is relevant evidence against assuming a general decision model knows which LLM will perform better. It does not establish that user-defined intent policies cannot work. Its order/calibration warnings also motivate the controls below. [Paper, especially §§5 and 8](https://arxiv.org/html/2610.02267v1).

Two attribution corrections: arXiv **2501.01818 is Rerouting LLM Routers**, not the RouteLLM paper; RouteLLM is **2406.18665**. VDAR-Router's cited preprint is dated **July 2026**, rather than 2025. Treat claims of one approach “dominating the state of the art” cautiously across different model pools, budgets and evaluations. [Rerouting](https://arxiv.org/abs/2501.01818), [RouteLLM](https://arxiv.org/html/2406.18665v4), [VDAR-Router](https://arxiv.org/abs/2607.18098).

I would not adopt the suggested decision-model + embedding + learned-win-rate pipeline as the initial implementation of the user's policy feature. It solves a larger optimization problem and needs new outcome data. Nor should generic “suspicious” text stripping be assumed to fix misrouting: it can remove legitimate request content. Preserve role boundaries, enforce allowed destinations, test conflicting instructions, and apply explicit rules outside the classifier.

## Can the API expose plan/edit mode?

**There is no universal OpenAI inference field meaning “the client is in plan/edit mode.”** Chat Completions provides a string-valued `metadata` map, which a cooperating client can populate. A key such as `localrouter.mode` would be our convention, not an official OpenAI mode field. [Chat Completions request reference](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create).

**Codex has `turn/start.collaborationMode` in its app-server protocol.** That is the application's control protocol; it is not automatically a field on the outgoing inference request received by an OpenAI-compatible gateway. [Codex App Server](https://learn.chatgpt.com/docs/app-server).

Local source inspection of the cached Codex revision `e9fb493` adds a possible adapter: `core/src/context/collaboration_mode_instructions.rs` renders selected mode instructions in a **developer** message delimited by `<collaboration_mode>`. `codex-api/src/common.rs::ResponsesApiRequest` has no dedicated collaboration-mode field. The developer marker can provide a version-specific hint if it reaches the gateway intact. This has not been validated by capturing a live request from the user's client. A marker in user text or an old quoted message must not count as current client mode. Unknown clients/missing markers remain unknown. Sandbox permissions and reasoning effort are separate concepts.

The current LocalRouter paths need these concrete changes:

- `crates/lr-server/src/types.rs::ChatCompletionRequest` accepts metadata, and `pipeline.rs` forwards it, but `spawn_routellm_classification` reads only joined message text.
- `crates/lr-server/src/routes/responses.rs::CreateResponseRequest` accepts metadata, but `build_chat_completion_request` sets the translated request's `metadata` to `None`. Extract/preserve routing context across this boundary.
- `crates/lr-coding-agents/src/manager.rs` already knows configured permission mode for managed agents, but the Codex executor branch does not currently translate `CodingPermissionMode::Plan` into a collaboration-mode override. Do not infer working Codex mode integration from the existence of the settings enum.

Recommended precedence: a configured client adapter or authenticated client routing metadata, then a recognized current developer-mode marker from that client adapter, then unknown. Record the source. Metadata represents a client preference, not authority to bypass model permissions, budgets or capability checks.

Example **proposed client convention**:

```json
{
  "model": "localrouter/auto",
  "metadata": {"localrouter.mode": "plan"},
  "messages": [{"role": "user", "content": "Plan the migration."}]
}
```

Normalized classifier state could be:

```json
{
  "request_context": {"mode": "plan", "mode_source": "client_metadata"},
  "latest_user_request": "Plan the migration.",
  "conversation": []
}
```

The exact rule uses `request_context.mode` directly. For **plan → Astra, otherwise Sol**, missing mode also means Sol. Semantic fallback on missing mode should be a different user-selected policy, not a silent reinterpretation of “otherwise.” Consume routing-only metadata locally rather than forwarding it indiscriminately to every provider.

## Local policy experiment

72 hand-labelled cases, frozen with four policies **before inference**, including five uses of existing repository UI example prompts. Categories include straightforward requests, ambiguous boundaries, quoted keywords, multilingual messages, contradictory routing instructions, short histories and two long histories. The 14 mode cases deliberately include contradictory prompt text and missing/stale metadata. This small, deliberately difficult smoke corpus is not representative production traffic or a held-out benchmark of a tuned model.

Models: existing **Laya English through Ollaya 0.7.5/Metal**, and **Kev 0.8B through its dedicated MLX engine**, on the same Apple M2 Max/96 GiB machine as the [original strong/weak experiment](README.md). Exact runtime metadata is in [environment.json](environment.json). No cloud model calls or private conversations. Three warmups per run; sequential requests while other development activity could use the machine. Reported latency covers successful warm loopback requests, not cold start, concurrency or downstream generation.

| Policy | Cases | Laya correct | Kev correct |
|---|---:|---:|---:|
| Plan / implement / review / general | 28 | 20/28 (71.4%) | 19/28 (67.9%) |
| Code / writing / analysis / general | 16 | 13/16 (81.3%) | 15/16 (93.8%) |
| Custom support: billing / technical / cancellation / general | 14 | 8/14 (57.1%) | 11/14 (78.6%) |
| Exact metadata rule: plan → Astra, otherwise Sol | 14 | 5/14 (35.7%) | 7/14 (50.0%) |
| All policies | 72 | 46/72 (63.9%) | 52/72 (72.2%) |

Laya rejected both long histories with HTTP 422; these count as unsuccessful. Kev had no API errors. Successful warm latency p50/p95: **Laya 31/49 ms; Kev 56/98 ms**. A deterministic equality rule matched **14/14 mode cases** without inference. That last result tests already-normalized synthetic metadata, not real client mode detection.

Important failure patterns:

- **Metadata loses to semantics.** With mode `default` and “Write a detailed plan,” both models selected Astra. Fake `mode: plan` inside user text also fooled both. The simple exact rule avoids these failures.
- **Conversation history can dominate the latest request.** Kev selected planning for “Now implement it” after a plan, and for “Thanks, just say goodbye” after planning discussion. Removing history increased workflow agreement to **23/28**, but lost references such as “Yes, go ahead.” Across seven short history cases it changed from 3/7 to 4/7; full context was useful on some, harmful on others. Latest-only is an ablation, not a complete solution.
- **Option order still matters.** Reversing labels changed 9/70 valid Laya choices and 2/72 Kev choices. Their reversed-run total agreement was 49/72 and 53/72. Stable order improves reproducibility, not correctness.
- **Names and conflicting instructions distract the classifier.** “The billing page throws error 500; payments are correct” was routed to billing by both. Requests to ignore routing rules also caused errors. The semantic model cannot enforce a trust boundary by itself.
- **High probability is not a reliability guarantee.** Requiring maximum option probability ≥0.7 accepted 31/72 Kev cases, of which only 23 were correct (74.2%); 41 needed fallback. At ≥0.9 only four were accepted. Laya accepted eight at ≥0.9 and got six correct. These descriptive thresholds were not tuned/calibrated on a separate set.

These results support prototyping **clear specialist policies**, and exact metadata rules immediately at the design level. They do not justify making either tested zero-shot model the default workflow router. No answers were generated with Astra/Sol; their resulting quality, cost and tool compatibility were not measured.

## Defaults and custom settings

Offer a few editable templates; users choose actual provider/model IDs rather than inheriting invented model rankings.

| Template | Question or condition | Options and use |
|---|---|---|
| Client mode | Is the supplied mode `plan`? | Plan → Astra; otherwise → Sol. Exact, lowest overhead. |
| Coding workflow | What work does the current request ask for? | Planning, implementation, review, general. Separate design, building and inspection preferences. |
| Task specialist | What output is requested? | Code, writing, analysis, general. Route by desired output rather than quoted subject matter. |
| Fast / thorough | Does this request need a detailed investigation or a short response? | Thorough, quick, default. A user preference; do not market it as proven strong/weak optimization. |
| Custom | User writes their question and descriptions | For example billing, technical support, cancellation, general; each maps to a chosen model/list. |

Keep capability constraints deterministic before model selection: image support, tools, structured output and context capacity are requirements, not semantic guesses. Likewise, an explicit “local-only” client restriction must not depend on classifying whether the prompt looks private.

For a custom policy, expose:

1. **Question**, e.g. “What kind of work is the user asking us to perform now?”
2. **Options** with stable IDs, short descriptions, and assigned model or ordered fallback list. Start with a small number of distinct options. Names such as `planning` should be independent of model names.
3. **Priority for overlaps**, e.g. “if asked to plan and implement in one request, choose planning.” A gateway chooses one model per request; splitting a single request into a plan and implementation requires agent orchestration.
4. **Default route** for no match, low confidence, timeout or missing classifier. Include capability-compatible fallbacks for unavailable destination models.
5. **Context settings**: explicit mode rule first; latest request plus bounded relevant history for semantic decisions. Preview must expose omitted context and its source.
6. **Try examples** with expected routes and saved policy/version. Show the selected route, model, option scores, duration and fallback reason. A route match is sufficient explanation; a generated rationale is optional.

## How to replace current Strong/Weak

The sequence below is a research design for a future implementation task. The task checklist lives in [the saved research plan](../../plan/2026-10-08-STRONG_WEAK_DECISION_RESEARCH.md).

1. Introduce versioned `RoutingPolicy` and `RoutingDecision` types: explicit predicates, semantic question/options, model lists, default route, policy/model version and evidence source. Keep the existing `localrouter/auto` entry point and ordinary ordered fallback machinery.
2. Build role-aware routing context before compression loses relevant information. Carry mode/metadata through Chat Completions and Responses. Extract text from array content, prioritize the latest turn and reserve input budget for route descriptions. Avoid the current chronological string concatenation and 512-token prefix truncation.
3. Run exact rules before semantic inference. Call an explicitly configured local native System One provider directly, never recursively through `localrouter/auto`. An adapter for a policy-trained generative router can implement the same route-selection interface; benchmark it before choosing the default.
4. Select the route's eligible model list. Bound classifier queues/timeouts and cache by policy version, classifier version and complete routing context. Record route IDs and scores in local monitoring without introducing telemetry. Do not use uncalibrated class probabilities as quality percentages.
5. Change routes at safe request/turn boundaries. Preserve tool-call state, provider-neutral conversation history and active-response affinity where required. Never switch providers mid-stream or blindly send one provider's opaque response/cache/reasoning state to another. These transitions need integration tests.
6. Migrate saved strong/weak model lists into a clearly named legacy policy template, with the strong list as conservative failure fallback. Preserve enablement preferences but invalidate the old threshold: 0.3 is not portable. Do not silently map users' lists to planning/implementation. Where the new semantic policy is unvalidated or unavailable, expose setup status and use the configured fallback.
7. Replace Strong/Weak settings, fixed savings estimates and RouteLLM download UI with templates/custom policies, a classifier selector, preview and measured local statistics. Then delete the old `lr-routellm` runtime/wiring/downloader and obsolete commands. Preserve readers for historical monitoring entries. Update Tauri types, mocks and docs together.
8. Before release, evaluate untouched conversations, mode conflicts/missing signals, option permutations, long history, model unavailability, cancellation, streaming and tool loops. Separate policy-match accuracy from answer quality, cost and latency. Test metadata extraction independently of the classifier. Review plan coverage, test coverage and implementation for bugs; run stable workspace Clippy/fmt/tests and TypeScript checks, then commit and push.

The next model comparison should include **Arch-Router or its current policy-trained successor** and a stronger native decision model on the same frozen cases plus a new held-out set. There is no evidence here to choose a new universal strong/weak checkpoint purely by release date. For the original performance-routing results and recent quality-oriented developments, see [README.md](README.md).

## Reproduce and inspect

Inputs: [policies.json](policies.json), [policy-cases.jsonl](policy-cases.jsonl). Raw outputs: `policy-{laya,kev}.jsonl`, reversed-order runs, and Kev latest-only run; every run has a `.meta.json` with question text, warmups and corpus hash. [Machine-readable analysis](policy-summary.json) includes confusion counts, failures, coverage and latency.

```sh
python3 research/strong-weak-2026-10-08/policy_benchmark.py prepare
RESEARCH_API_KEY=local-routing-research python3 research/strong-weak-2026-10-08/policy_benchmark.py run \
  --url http://127.0.0.1:19435 --model laya:en \
  --output research/strong-weak-2026-10-08/policy-laya.jsonl
# Dedicated Kev server: use port 19436 and model kev-0.8b.
# Controls: --reverse; --context latest_only. Use a distinct output name.
python3 research/strong-weak-2026-10-08/policy_analyze.py
python3 -m unittest discover -s research/strong-weak-2026-10-08 -p 'test_*.py'
```

Use isolated loopback servers as described in the [original report](README.md). These commands make local decision calls only; corpus preparation overwrites the fixed input artifacts.
