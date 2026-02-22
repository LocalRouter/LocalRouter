<!-- @entry access-logs -->

LocalRouter writes structured access logs for every API request to a log file in the data directory. Each log entry includes a timestamp, client ID, provider, model, request/response token counts, latency, cost, and status code. Logs are written asynchronously using a buffered writer to minimize performance impact. Log files rotate daily with configurable retention. The log format is newline-delimited JSON for easy parsing with standard tools like `jq`.

<!-- @entry in-memory-metrics -->

The monitoring system maintains real-time metrics in memory using a 4-tier aggregation architecture. Metrics are collected per-request and aggregated at multiple levels: per-client, per-provider, per-model, and global. The in-memory store uses time-series bucketing with configurable resolution, enabling efficient queries for recent activity without touching disk. Metrics are used internally for rate limiting projections, dashboard visualizations, and health monitoring.

<!-- @entry metrics-time-series -->

Time-series data is stored in fixed-size ring buffers with per-minute bucketing. Each bucket records request count, total tokens (input/output), total cost, and latency samples. Older buckets are automatically evicted as new data arrives, maintaining a rolling window of recent activity (default: 24 hours at minute resolution, 30 days at hour resolution). This two-tier resolution provides both fine-grained recent data and longer-term trends.

<!-- @entry metrics-dimensions -->

Metrics are indexed across three dimensions. **Per-client** metrics track individual client usage for rate limiting and billing. **Per-provider** metrics track provider health, latency, and error rates for routing decisions. **Global** metrics provide system-wide totals for the dashboard overview. Each dimension maintains independent time-series buckets, allowing queries like "requests per minute for client X on provider Y" without scanning raw logs.

<!-- @entry metrics-percentiles -->

Latency percentiles (P50, P95, P99) are calculated from sampled latency values stored in each time-series bucket. The monitoring system uses a reservoir sampling algorithm to maintain a representative sample within bounded memory. Percentiles are recalculated on each query from the stored samples. These metrics appear in the dashboard and can be queried via Tauri commands for custom alerting or reporting.

<!-- @entry historical-log-parser -->

The historical log parser reads past access log files to reconstruct metrics for periods not covered by the in-memory store. When the dashboard requests data beyond the in-memory window, the parser scans the relevant log files, aggregates the data, and returns it in the same format as in-memory metrics. Parsing is done asynchronously with streaming to handle large log files without excessive memory usage.

<!-- @entry graph-data -->

The monitoring system generates graph-ready data series for the dashboard visualizations. Data is returned as arrays of `(timestamp, value)` points suitable for line charts, bar charts, and area charts. Available graph series include requests over time, tokens over time, cost over time, latency percentiles over time, and error rate over time. Each series can be filtered by client, provider, or model, and the time resolution adjusts automatically based on the requested time range.
