# Native rerank

Enable **Client Settings → Document operations → Allow rerank**. This is a
separate default-off grant; allowing OCR or web search does not enable rerank.
Normal model permissions, credential lifetime and request quotas still apply.

`POST /v1/rerank` (also `/rerank`) accepts caller-provided text candidates:

```json
{
  "model": "cohere/rerank-v4.0-pro",
  "query": "Which document answers the question?",
  "documents": ["First original candidate", "Second original candidate"],
  "top_n": 2,
  "return_documents": true
}
```

Direct Cohere uses its native v2 rerank endpoint. LLM Gateway uses its native
Cohere-compatible rerank endpoint, for example
`gateway/qwen3-reranker-8b`. The model prefix is the configured LocalRouter
provider instance. Unknown provider protocols fail; rerank is never emulated by
an ordinary chat model or a keyword heuristic.

Limits: nonempty query up to 16 KiB, 1–128 nonempty candidates up to 32 KiB each
and 1 MiB total, and top_n from 1 to the candidate count. Omitting top_n returns
up to all candidates. Direct Cohere accepts max_tokens_per_doc from 1 to 4096;
that field has no verified LLM Gateway mapping and is rejected there. Unverified
max_chunks_per_doc and unknown fields reject before IO. Native providers may
truncate long candidates; returning an original source does not claim every
byte was scored.

Results preserve native order, scores and original zero-based candidate indices.
Invalid/duplicate/out-of-range indices, nonfinite scores and reversed score
ordering fail explicitly. Relevance scores are model-specific and are not
asserted to be probabilities calibrated across queries. When return_documents
is true, LocalRouter maps validated indices to the exact original candidates;
it never substitutes mismatched upstream document text.

The shared sequential pipeline scans actual query and candidate text before
dispatch. Model/content rewrites require a new explicit native request. Automatic
model selection, compression, classification, MCP loops, affinity, cache reuse
and automatic replay are not enabled. No document corpus or URL is fetched.
Strict local-only/no-retention profiles, free-only strategies and cost ceilings
are unsupported. Configured token ceilings are also rejected because native
billing units cannot reliably satisfy a common text-token ceiling. Use explicit
request quotas for these operations.

Native meta.billed_units is retained: Cohere may report search units, while
LLM Gateway reports input-token units. Only reported integer input tokens feed
token statistics; no Cohere tokens or currency fees are invented. The
localrouter.rerank response extension reports unknown fees, native units,
candidate count and provider-truncation semantics. Numeric aggregate cost
metrics contain known subtotals only.

Each operation dispatches once, with redirects and fallback disabled. Errors
carry x-should-retry:false for compatible SDKs; Try It Out disables SDK retries.
Dispatched requests consume quota even on failure or cancellation. Aborting
local IO does not guarantee the provider stopped processing or billing.

Try It Out → Rerank displays native scores and original source positions as plain
text. The website uses a labeled illustrative fixture, never a claim that a
model scored the supplied documents. No remote assets or executable candidate
markup are loaded.

Verified primary contracts:
- https://docs.cohere.com/v2/reference/rerank
- https://docs.llmgateway.io/features/rerank
