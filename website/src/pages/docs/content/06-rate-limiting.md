<!-- @entry request-rate-limits -->

Request-based rate limits enforce a maximum number of API calls within a time window (Minute, Hour, or Day). Each request increments the counter by one, and the counter resets when the window expires.

<!-- @entry token-limits -->

Token-based rate limits (`TotalTokens`, `InputTokens`, `OutputTokens`) cap the number of tokens consumed within a time window.

Since actual token counts are unknown until the response arrives, token limits are enforced after each request completes. For streaming responses, tokens are estimated based on the response content length.

<!-- @entry cost-limits -->

Cost-based rate limits cap spending in USD within a time window. Cost is calculated per-request using the model's pricing and actual token counts.

Useful for enforcing budget caps like $10/day per client. Free and local models are excluded from cost limit calculations.

<!-- @entry per-key-vs-per-router -->

Rate limits operate at two levels:

**Client-level** limits are enforced per individual client — each client has independent counters for requests, tokens, and cost.

**Strategy-level** limits are shared across all clients using the same strategy, tracking aggregate usage.

Both levels are checked before a request proceeds. Rate limiter state is persisted to disk and restored on startup to survive application restarts.
