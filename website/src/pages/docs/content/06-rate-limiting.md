<!-- @entry request-rate-limits -->

Request-based rate limits enforce a maximum number of API calls within a time window (`Minute`, `Hour`, or `Day`).

The `RateLimiterManager` uses a sliding window algorithm backed by a `VecDeque<UsageEvent>` — each request increments the counter by 1.0, and events older than the window are pruned on each check. At the strategy level, the router uses metrics-based projection: it queries recent usage and adds 1.0 to the current count, rejecting the request if the projected total exceeds the limit.

<!-- @entry token-limits -->

Token-based rate limits (`TotalTokens`, `InputTokens`, `OutputTokens`) cap the number of tokens consumed within a sliding time window.

Since actual token counts are unknown until the response arrives, token limits are enforced *after* completion at the client level and via pre-request projection at the strategy level. For streaming responses, tokens are estimated at ~1 token per 4 characters of content.

<!-- @entry cost-limits -->

Cost-based rate limits cap spending in USD within a time window. Cost is calculated per-request as `(input_tokens / 1000) x input_cost + (output_tokens / 1000) x output_cost` using provider pricing data.

At the strategy level, cost projection uses the average cost from recent metrics; free/local models are excluded from cost limit calculations. Useful for enforcing budget caps like $10/day per client.

<!-- @entry per-key-vs-per-router -->

Rate limits operate at two levels.

**Client-level** limits are enforced per individual client using sliding window counters — each client has independent counters for requests, tokens, and cost.

**Strategy-level** limits use metrics-based pre-request projection, querying aggregate usage across all clients sharing that strategy.

Both levels are checked before a request proceeds. Rate limiter state is persisted to disk periodically and restored on startup to survive application restarts.
