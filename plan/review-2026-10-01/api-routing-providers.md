# API, routing, and providers review — 2026-10-01

## Scope and method

Owned: `crates/lr-server`, `crates/lr-router`, `crates/lr-providers`. Read the repository's CLAUDE.md conventions. Preserved the pre-existing edit to `crates/lr-catalog/catalog/modelsdev_raw.json`. No commits. No application launch, real provider requests, credentials access, model download, model loading, inference, or GPU execution.

Coverage is deliberately distinguished: every Rust source file below received an inventory and structural risk scan (production panic/slicing sites, asynchronous synchronization, stream framing, secret handling, request surfaces, and test side effects). High-risk implementations received targeted substantive reads, and changed functions/tests received full review. This is **not a line-by-line proof of all source code**, nor a claim of complete dynamic coverage. The provider catalog/factory tables, giant route orchestration modules, and embedded engine implementations were inspected selectively; embedded serving behavior remains static-only.

## Implemented improvements

### Provider streaming correctness

1. Added shared `sse_lines::line_batches` built on the byte-preserving SSE/NDJSON framer; existing `lines` behavior is retained. Complete lines decode only after all UTF-8 bytes arrive. CRLF is normalized and an unterminated final line is emitted at EOF. Errors preserve stream ordering.
2. Migrated 13 completion adapters: Anthropic, Cerebras, DeepInfra, Gemini, Groq, Mistral, Ollama, OpenAI, generic OpenAI-compatible, OpenRouter, Perplexity, TogetherAI, xAI. Previously each independently decoded arbitrary HTTP chunks into lossy UTF-8 strings before buffering, corrupting non-ASCII text split across reads. They also dropped unterminated last lines. Removed the redundant per-adapter shared string/mutex buffer; retained provider-specific event conversion and usage handling.
3. Responses SSE now decodes through the common byte-safe line framer. Its frame-boundary routine chooses whichever LF/CRLF delimiter occurs first, preventing concatenation of independent mixed-ending events.
4. Ollama pull-progress decoding now consumes complete NDJSON lines, preserves every progress record, ignores blank lines, flushes the final record without newline, and reports malformed/provider-error records instead of fabricating empty-chunk failures. Tests feed synthetic bytes only: no model pull was performed.
5. Added exhaustive single-boundary and one-byte fragmentation fixtures containing accented text, emoji, CJK, CRLF, a data sentinel, and an unterminated tail. Responses fixtures verify wire order; Ollama fixtures verify all progress records.

### OAuth credential durability and confidentiality

`lr-providers/src/oauth/storage.rs` now serializes read-modify-save transactions, writes into a same-directory private NamedTempFile, syncs contents, and atomically replaces the destination. Unix files are 0600 before any token bytes are written, avoiding the old chmod-after-write exposure. Parallel writers cannot truncate/interleave the JSON or publish stale snapshots. A failed save/delete leaves the in-memory credentials unchanged. The blocking write task owns the cache guard until both disk and cache commit, including when the async caller is cancelled. Existing destination symlinks are replaced rather than followed. Tests use only disposable directories and synthetic credentials. Updated the module description to accurately describe the owner-private JSON store instead of claiming it is encrypted.

### Rate limiting and concurrency

- All configured metric types now participate in admission checks. Previously token/cost usage was recorded but never consulted when allowing subsequent requests, so exhausted budgets did not block requests.
- Multiple windows for one metric now charge each request once. They retain history for the longest configured window, so inspecting/checking a minute window cannot erase events still needed by an hourly window.
- Record paths prune old events as well as check paths; timestamp insertion preserves ordering despite concurrent completion order, keeping front-based expiry valid.
- DashMap shard guards are released before awaiting per-state locks when taking persistence snapshots or reading usage.
- Total token arithmetic saturates instead of overflowing; retry delays round up rather than advertising zero seconds for a remaining fractional second.
- The negative endpoint-capability cache removes expired entries conditionally, avoiding deleting a newly refreshed entry after releasing its read guard.

These fixes do not implement atomic admission reservations: simultaneous in-flight requests can still overshoot a limit because actual usage is charged after completion. The current API also reports only the first configured window for a metric in get_*_usage. Both are explicit follow-ups rather than hidden guarantees.

### HTTP server robustness, privacy, and lifecycle

- Host-header protection parses HTTP authority correctly: IPv6 loopback works; non-ASCII/malformed hosts, invalid ports, userinfo and external domains are rejected; DNS localhost matching is case-insensitive. Existing support for a missing Host header is retained.
- Request/trace logs include only the URI path, preventing MCP `?token=` credentials and other query data from entering persistent logs.
- Tower authentication now calls the service instance that was polled for readiness, preserving concurrency-limit/readiness permits instead of calling a fresh clone.
- Port search stops at 65535 instead of overflowing; binding port zero returns/reports the OS-assigned port.
- Periodic server session/token cleanup tasks observe server cancellation rather than retaining old server state after every restart.
- Transcription/translation monitor previews truncate on a character boundary, preventing panics when byte 200 lies inside a Unicode transcript character.
- `/responses` history retrieval is scoped to the authenticated client through the new `get_active_for_client` storage API supplied by the MCP/context review agent. Foreign response IDs cannot import another client's messages/tools; foreign/missing/expired references retain the existing start-fresh behavior. Owner-scope regression coverage is in lr-responses-sessions.

### Numeric validation

- Anthropic thinking budgets reject integers that cannot fit the supported type instead of wrapping into an apparently valid small budget.
- Logprob counts reject negative, fractional, textual and oversized values before narrowing; omitted counts retain the existing zero default.
- System One probability normalization scales by the maximum finite weight before summing. Large finite weights such as `[f64::MAX, f64::MAX]` now normalize to `[0.5, 0.5]` instead of all zeros from an infinite sum.

## Validation

Compiled all three library test targets without executing any tests:

```sh
env RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 CARGO_TARGET_DIR=/private/tmp/localrouter-review-target rustup run stable cargo test --offline -p lr-providers -p lr-router -p lr-server --lib --no-run
```

**Compilation passed in 15m09s** on stable Rust 1.99. Build log: `/private/tmp/localrouter-review-api-build.log`. The only reported build caveat was an upstream `block 0.1.6` future-incompatibility warning. Compilation of model dependencies does not load a model or execute GPU work.

To release the Cargo lock for the other reviewers, executed inspected filters directly against the resulting test binaries under `/private/tmp/localrouter-review-target/debug/deps`:

| Binary / filters | Result | Side effects |
|---|---:|---|
| `lr_providers-05b81bf276645437`: `sse_lines::tests`, `openai_responses::stream::tests`, `oauth::storage::tests`, `features::anthropic_thinking::tests`, `features::logprobs::tests`, `systemone::types::tests`, `ollama::tests::pull_progress_` | **55 passed, 0 failed** | Synthetic byte/string fixtures; temporary OAuth files only |
| `lr_router-a524b103bc205375`: `rate_limit::tests`, `endpoint_cache::tests` | **13 passed, 0 failed** | In-memory state, temporary persistence and short expiry waits |
| `lr_server-a932ff08da3f7e2c`: `host_validation_tests`, `trace_middleware_tests`, `transcript_previews_preserve_multibyte_characters`, `kill_switch_tests` | **12 passed, 0 failed** | In-memory Axum requests/streams; no app/server startup |
| Provider binary: the 17 `stream_*` usage fixtures enumerated below | **17 passed, 0 failed** | Temporary localhost HTTP fixture servers and synthetic keys |

**Total: 97 passed, 0 failed.** Tests ran with `--test-threads=4`. The localhost-fixture run used approved sandbox escalation because loopback binds are restricted; no test contacted a real provider or used actual credentials. Exact fixture filters: `anthropic::tests::stream_`, `cerebras::tests::stream_reports_upstream_usage`, `deepinfra::tests::stream_reports_upstream_usage`, `gemini::tests::stream_`, `groq::tests::stream_reports_upstream_usage`, `mistral::tests::stream_reports_upstream_usage`, `ollama::tests::stream_reports_final_eval_counts`, `openai::tests::stream_reports_upstream_usage`, `openai_compatible::tests::stream_`, `openrouter::tests::stream_reports_upstream_usage`, `perplexity::tests::stream_reports_upstream_usage`, `togetherai::tests::stream_reports_upstream_usage`, `xai::tests::stream_reports_upstream_usage`.

Changed Rust sources were formatted and `git diff --check` passed for owned files. Full workspace lint/build results are recorded by the root reviewer; the above tests intentionally exclude embedded model execution, GPU/device selection, real provider integration and external OAuth flows.

## Important follow-ups and review limitations

- Cross-hop trust issue was fixed jointly: arbitrary inbound trace headers previously skipped scans/approvals/accounting and the proxy firewall. The MCP agent changed central `RequestTrace::outbound_for` to preserve correlation IDs but reset the enforcement hop to 1; server tests now assert a forged header cannot set duplicate state or suppress rate-limit charges. All production inbound parse paths were scanned: the server and proxy both use the central helper. Intentional consequence: multi-hop transformations/accounting can repeat until an authenticated, request-bound handoff protocol is implemented. A simple ID registry or an unbound signature would still allow replay.
- Stream line/frame accumulation remains unbounded for a provider that never supplies a delimiter. A protocol-aware maximum and typed error need coordinated design (large tool arguments are legitimate).
- Server lifecycle `start`/`stop` and reverse-proxy startup have separate read/write phases and deserve a serialized lifecycle transaction under concurrent UI calls.
- Token endpoint documentation describes form-encoded OAuth requests but the handler still extracts JSON. Its per-client-ID attempt map also has no global cardinality cap. These are pre-existing compatibility/resource issues outside the implemented focused changes.
- Full production payload logging remains intentionally present in monitor/error paths. This review fixed definite query credential leakage, not every product-level logging retention/privacy policy.
- External provider protocol compatibility, real OAuth exchanges, embedded inference, model engines, OS dialogs/keychain access, and platform-specific runtime behavior were not exercised.
- No defects were established in the sampled existing byte-safe Cohere parser, shared typed OpenAI error classification, health cache aggregate calculation, feature registry wiring, provider model catalog fallback logic, and System One request cardinality validation. This is scoped evidence, not a blanket guarantee.

## Exact file coverage inventory

**D**: changed or targeted substantive inspection of relevant behavior; **S**: structural scan/inventory plus selected declarations/call paths. D does not imply every line in a large file was deeply reviewed.

| File | Lines at report generation | Coverage |
|---|---:|---|
| `crates/lr-server/src/lib.rs` | 911 | D |
| `crates/lr-server/src/manager.rs` | 156 | D |
| `crates/lr-server/src/middleware/auth_layer.rs` | 302 | D |
| `crates/lr-server/src/middleware/client_auth.rs` | 270 | D |
| `crates/lr-server/src/middleware/error.rs` | 218 | S |
| `crates/lr-server/src/middleware/mod.rs` | 5 | S |
| `crates/lr-server/src/openapi/extensions.rs` | 325 | S |
| `crates/lr-server/src/openapi/mod.rs` | 333 | S |
| `crates/lr-server/src/routes/audio.rs` | 1621 | D |
| `crates/lr-server/src/routes/chat.rs` | 2843 | S |
| `crates/lr-server/src/routes/completions.rs` | 1691 | S |
| `crates/lr-server/src/routes/embeddings.rs` | 492 | S |
| `crates/lr-server/src/routes/finalize.rs` | 473 | S |
| `crates/lr-server/src/routes/generation.rs` | 48 | S |
| `crates/lr-server/src/routes/helpers.rs` | 357 | D |
| `crates/lr-server/src/routes/images.rs` | 663 | S |
| `crates/lr-server/src/routes/mcp.rs` | 1797 | S |
| `crates/lr-server/src/routes/mcp_ws.rs` | 319 | S |
| `crates/lr-server/src/routes/mod.rs` | 37 | S |
| `crates/lr-server/src/routes/models.rs` | 398 | S |
| `crates/lr-server/src/routes/moderations.rs` | 397 | S |
| `crates/lr-server/src/routes/monitor_helpers.rs` | 927 | S |
| `crates/lr-server/src/routes/oauth.rs` | 443 | D |
| `crates/lr-server/src/routes/pipeline.rs` | 2802 | D |
| `crates/lr-server/src/routes/responses.rs` | 1523 | D |
| `crates/lr-server/src/routes/stream_usage.rs` | 466 | D |
| `crates/lr-server/src/routes/systemone.rs` | 671 | S |
| `crates/lr-server/src/state.rs` | 1816 | S |
| `crates/lr-server/src/types.rs` | 1155 | S |
| `crates/lr-router/src/endpoint_cache.rs` | 120 | D |
| `crates/lr-router/src/free_tier.rs` | 2644 | D |
| `crates/lr-router/src/lib.rs` | 3516 | S |
| `crates/lr-router/src/rate_limit.rs` | 1048 | D |
| `crates/lr-router/src/systemone.rs` | 1097 | S |
| `crates/lr-providers/src/anthropic.rs` | 2050 | D |
| `crates/lr-providers/src/cerebras.rs` | 442 | D |
| `crates/lr-providers/src/cohere.rs` | 1508 | D |
| `crates/lr-providers/src/deepinfra.rs` | 731 | D |
| `crates/lr-providers/src/embedded/decider.rs` | 565 | S |
| `crates/lr-providers/src/embedded/kev.rs` | 606 | S |
| `crates/lr-providers/src/embedded/laya.rs` | 623 | S |
| `crates/lr-providers/src/embedded/llamacpp.rs` | 744 | S |
| `crates/lr-providers/src/embedded/mod.rs` | 645 | S |
| `crates/lr-providers/src/embedded/ollaya.rs` | 1363 | S |
| `crates/lr-providers/src/embedded/sdcpp.rs` | 883 | S |
| `crates/lr-providers/src/embedded/von.rs` | 552 | S |
| `crates/lr-providers/src/factory.rs` | 4366 | S |
| `crates/lr-providers/src/features/anthropic_thinking.rs` | 302 | D |
| `crates/lr-providers/src/features/gemini_thinking.rs` | 240 | S |
| `crates/lr-providers/src/features/json_mode.rs` | 544 | S |
| `crates/lr-providers/src/features/logprobs.rs` | 553 | D |
| `crates/lr-providers/src/features/mod.rs` | 196 | S |
| `crates/lr-providers/src/features/openai_reasoning.rs` | 215 | S |
| `crates/lr-providers/src/features/prompt_caching.rs` | 691 | S |
| `crates/lr-providers/src/features/structured_outputs.rs` | 682 | S |
| `crates/lr-providers/src/gemini.rs` | 1721 | D |
| `crates/lr-providers/src/gpt4all.rs` | 554 | S |
| `crates/lr-providers/src/groq.rs` | 745 | D |
| `crates/lr-providers/src/health.rs` | 405 | D |
| `crates/lr-providers/src/health_cache.rs` | 595 | D |
| `crates/lr-providers/src/http_client.rs` | 369 | D |
| `crates/lr-providers/src/jan.rs` | 554 | S |
| `crates/lr-providers/src/key_storage.rs` | 314 | D |
| `crates/lr-providers/src/lib.rs` | 3149 | S |
| `crates/lr-providers/src/llamacpp.rs` | 584 | S |
| `crates/lr-providers/src/lmstudio.rs` | 745 | S |
| `crates/lr-providers/src/localai.rs` | 717 | S |
| `crates/lr-providers/src/mistral.rs` | 659 | D |
| `crates/lr-providers/src/oauth/anthropic_claude.rs` | 237 | S |
| `crates/lr-providers/src/oauth/github_copilot.rs` | 556 | S |
| `crates/lr-providers/src/oauth/mod.rs` | 295 | D |
| `crates/lr-providers/src/oauth/openai_codex.rs` | 357 | S |
| `crates/lr-providers/src/oauth/storage.rs` | 297 | D |
| `crates/lr-providers/src/oauth/token_source.rs` | 530 | D |
| `crates/lr-providers/src/ollama.rs` | 1474 | D |
| `crates/lr-providers/src/openai.rs` | 2006 | D |
| `crates/lr-providers/src/openai_compatible/stream_usage.rs` | 591 | D |
| `crates/lr-providers/src/openai_compatible.rs` | 1040 | D |
| `crates/lr-providers/src/openai_responses/emit.rs` | 479 | S |
| `crates/lr-providers/src/openai_responses/http.rs` | 131 | S |
| `crates/lr-providers/src/openai_responses/mod.rs` | 30 | S |
| `crates/lr-providers/src/openai_responses/request.rs` | 492 | S |
| `crates/lr-providers/src/openai_responses/response.rs` | 208 | S |
| `crates/lr-providers/src/openai_responses/stream.rs` | 776 | D |
| `crates/lr-providers/src/openai_responses/types.rs` | 368 | S |
| `crates/lr-providers/src/openrouter.rs` | 872 | D |
| `crates/lr-providers/src/perplexity.rs` | 456 | D |
| `crates/lr-providers/src/registry.rs` | 1757 | D |
| `crates/lr-providers/src/sse_lines.rs` | 249 | D |
| `crates/lr-providers/src/systemone/emulation.rs` | 581 | S |
| `crates/lr-providers/src/systemone/gateway.rs` | 865 | S |
| `crates/lr-providers/src/systemone/mod.rs` | 20 | S |
| `crates/lr-providers/src/systemone/provider.rs` | 619 | S |
| `crates/lr-providers/src/systemone/types.rs` | 639 | D |
| `crates/lr-providers/src/togetherai.rs` | 916 | D |
| `crates/lr-providers/src/xai.rs` | 472 | D |
