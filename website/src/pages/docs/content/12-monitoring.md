<!-- @entry access-logs -->

LocalRouter writes structured access logs for every API request to a log file in the data directory. Each log entry includes a timestamp, client ID, provider, model, request/response token counts, latency, cost, and status code.

Logs are written asynchronously to minimize performance impact. Log files rotate daily with configurable retention. The log format is newline-delimited JSON for easy parsing with standard tools like `jq`.

<!-- @entry in-memory-metrics -->

The monitoring system maintains real-time metrics in memory, collected per-request and aggregated across multiple dimensions: per-client, per-provider, per-model, and global. These metrics power the dashboard visualizations, rate limiting, and health monitoring.

<!-- @entry metrics-time-series -->

Metrics are stored as time-series data with two tiers of resolution: minute-level for the last 24 hours and hour-level for the last 30 days. This provides both fine-grained recent data and longer-term trends.

Older data is automatically evicted as new data arrives, keeping memory usage bounded.

<!-- @entry metrics-dimensions -->

Metrics are tracked across three dimensions:

- **Per-client** — Individual client usage for rate limiting and tracking.
- **Per-provider** — Provider health, latency, and error rates for routing decisions.
- **Global** — System-wide totals for the dashboard overview.

<!-- @entry metrics-percentiles -->

Latency percentiles (P50, P95, P99) are calculated per provider, per model, and globally. These metrics appear in the dashboard and help identify performance issues.

<!-- @entry historical-log-parser -->

When the dashboard requests data beyond the in-memory window (older than 24 hours at minute resolution), LocalRouter reads past access log files to reconstruct the metrics. This happens automatically and transparently — the dashboard shows a seamless timeline regardless of whether data comes from memory or log files.

<!-- @entry graph-data -->

The dashboard displays time-series graphs for requests, tokens, cost, latency percentiles, and error rates over time. Each graph can be filtered by client, provider, or model, and the time resolution adjusts automatically based on the selected time range.
