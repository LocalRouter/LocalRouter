//! Metrics collection with SQLite persistence
//!
//! Tracks usage metrics with progressive aggregation:
//! - Per-minute: Last 24 hours
//! - Per-hour: Last 7 days
//! - Per-day: Last 90 days

use chrono::{DateTime, DurationRound, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::mcp_metrics::McpMetricsCollector;
use super::storage::{Granularity, MetricRow, MetricsDatabase};

/// Request metrics for recording
#[derive(Debug, Clone)]
pub struct RequestMetrics<'a> {
    pub api_key_name: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub strategy_id: &'a str,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
}

/// Time-series data point for metrics (for API compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDataPoint {
    /// Timestamp (rounded to minute/hour/day)
    pub timestamp: DateTime<Utc>,

    /// Number of requests
    pub requests: u64,

    /// Input tokens
    pub input_tokens: u64,

    /// Output tokens
    pub output_tokens: u64,

    /// Total tokens
    pub total_tokens: u64,

    /// Cost in USD
    pub cost_usd: f64,

    /// Total latency (sum for averaging)
    pub total_latency_ms: u64,

    /// Number of successful requests
    pub successful_requests: u64,

    /// Number of failed requests
    pub failed_requests: u64,

    /// Latency samples (for percentile calculation)
    pub latency_samples: Vec<u64>,
}

impl MetricDataPoint {
    /// Get average latency in milliseconds
    pub fn avg_latency_ms(&self) -> f64 {
        if self.requests > 0 {
            self.total_latency_ms as f64 / self.requests as f64
        } else {
            0.0
        }
    }

    /// Get success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.requests > 0 {
            self.successful_requests as f64 / self.requests as f64
        } else {
            0.0
        }
    }

    /// Calculate latency percentile
    pub fn latency_percentile(&self, percentile: f64) -> u64 {
        if self.latency_samples.is_empty() {
            return 0;
        }

        let mut sorted = self.latency_samples.clone();
        sorted.sort_unstable();

        let index = ((percentile / 100.0) * (sorted.len() as f64 - 1.0)) as usize;
        sorted[index]
    }
}

impl From<MetricRow> for MetricDataPoint {
    fn from(row: MetricRow) -> Self {
        let input_tokens = row.input_tokens.unwrap_or(0);
        let output_tokens = row.output_tokens.unwrap_or(0);

        Self {
            timestamp: row.timestamp,
            requests: row.requests,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            cost_usd: row.cost_usd.unwrap_or(0.0),
            total_latency_ms: (row.avg_latency_ms * row.requests as f64) as u64,
            successful_requests: row.successful_requests,
            failed_requests: row.failed_requests,
            latency_samples: vec![], // Not stored in aggregated data
        }
    }
}

/// Metrics collector for tracking usage with SQLite persistence
pub struct MetricsCollector {
    /// SQLite database for persistent storage
    db: Arc<MetricsDatabase>,

    /// MCP metrics collector (still in-memory for now)
    mcp_metrics: McpMetricsCollector,

    /// Optional callback to notify when metrics are recorded
    on_metrics_recorded: parking_lot::RwLock<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl MetricsCollector {
    /// Create a new metrics collector with database
    pub fn new(db: Arc<MetricsDatabase>) -> Self {
        Self {
            db,
            mcp_metrics: McpMetricsCollector::new(24), // 24 hour retention for MCP
            on_metrics_recorded: parking_lot::RwLock::new(None),
        }
    }

    /// Set callback to be called when metrics are recorded
    pub fn set_on_metrics_recorded<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *self.on_metrics_recorded.write() = Some(Box::new(callback));
    }

    /// Create a new metrics collector with default retention (deprecated)
    #[deprecated(note = "Use new() with database instead")]
    pub fn with_default_retention() -> Self {
        // Create an in-memory database for backward compatibility
        let db_path = std::env::temp_dir().join(format!("metrics-{}.db", uuid::Uuid::new_v4()));
        let db = Arc::new(MetricsDatabase::new(db_path).expect("Failed to create temp database"));
        Self::new(db)
    }

    /// Record a successful request
    pub fn record_success(&self, metrics: &RequestMetrics) {
        self.record_success_at(metrics, Utc::now());
    }

    /// Record a successful request at a specific timestamp (for testing)
    pub fn record_success_at(&self, metrics: &RequestMetrics, timestamp: DateTime<Utc>) {
        let minute_timestamp = timestamp
            .duration_trunc(chrono::Duration::minutes(1))
            .unwrap();

        // Create metric row for this minute
        let row = MetricRow {
            timestamp: minute_timestamp,
            granularity: Granularity::Minute,
            requests: 1,
            successful_requests: 1,
            failed_requests: 0,
            avg_latency_ms: metrics.latency_ms as f64,
            input_tokens: Some(metrics.input_tokens),
            output_tokens: Some(metrics.output_tokens),
            cost_usd: Some(metrics.cost_usd),
            method_counts: None,
            p50_latency_ms: Some(metrics.latency_ms as f64),
            p95_latency_ms: Some(metrics.latency_ms as f64),
            p99_latency_ms: Some(metrics.latency_ms as f64),
        };

        // Write to all five tiers
        let metric_types = vec![
            "llm_global".to_string(),
            format!("llm_key:{}", metrics.api_key_name),
            format!("llm_provider:{}", metrics.provider),
            format!("llm_model:{}", metrics.model),
            format!("llm_strategy:{}", metrics.strategy_id),
        ];

        for metric_type in metric_types {
            // Try to read existing data for this minute
            if let Ok(existing) = self.db.query_metrics(
                &metric_type,
                minute_timestamp,
                minute_timestamp + chrono::Duration::minutes(1),
            ) {
                if let Some(existing_row) = existing.first() {
                    // Merge with existing data
                    let merged_row = MetricRow {
                        timestamp: minute_timestamp,
                        granularity: Granularity::Minute,
                        requests: existing_row.requests + 1,
                        successful_requests: existing_row.successful_requests + 1,
                        failed_requests: existing_row.failed_requests,
                        avg_latency_ms: (existing_row.avg_latency_ms
                            * existing_row.requests as f64
                            + metrics.latency_ms as f64)
                            / (existing_row.requests + 1) as f64,
                        input_tokens: Some(
                            existing_row.input_tokens.unwrap_or(0) + metrics.input_tokens,
                        ),
                        output_tokens: Some(
                            existing_row.output_tokens.unwrap_or(0) + metrics.output_tokens,
                        ),
                        cost_usd: Some(existing_row.cost_usd.unwrap_or(0.0) + metrics.cost_usd),
                        method_counts: None,
                        p50_latency_ms: existing_row.p50_latency_ms, // Keep existing percentiles
                        p95_latency_ms: existing_row.p95_latency_ms,
                        p99_latency_ms: existing_row.p99_latency_ms,
                    };

                    let _ = self.db.upsert_metric(&metric_type, &merged_row);
                } else {
                    // No existing data, insert new
                    let _ = self.db.upsert_metric(&metric_type, &row);
                }
            } else {
                // Error querying, just insert new
                let _ = self.db.upsert_metric(&metric_type, &row);
            }
        }

        // Notify callback that metrics were recorded
        if let Some(ref callback) = *self.on_metrics_recorded.read() {
            callback();
        }
    }

    /// Record a failed request
    pub fn record_failure(
        &self,
        api_key_name: &str,
        provider: &str,
        model: &str,
        strategy_id: &str,
        latency_ms: u64,
    ) {
        self.record_failure_at(
            api_key_name,
            provider,
            model,
            strategy_id,
            latency_ms,
            Utc::now(),
        );
    }

    /// Record a failed request at a specific timestamp (for testing)
    pub fn record_failure_at(
        &self,
        api_key_name: &str,
        provider: &str,
        model: &str,
        strategy_id: &str,
        latency_ms: u64,
        timestamp: DateTime<Utc>,
    ) {
        let minute_timestamp = timestamp
            .duration_trunc(chrono::Duration::minutes(1))
            .unwrap();

        // Create metric row for this minute
        let row = MetricRow {
            timestamp: minute_timestamp,
            granularity: Granularity::Minute,
            requests: 1,
            successful_requests: 0,
            failed_requests: 1,
            avg_latency_ms: latency_ms as f64,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            method_counts: None,
            p50_latency_ms: Some(latency_ms as f64),
            p95_latency_ms: Some(latency_ms as f64),
            p99_latency_ms: Some(latency_ms as f64),
        };

        // Write to all five tiers
        let metric_types = vec![
            "llm_global".to_string(),
            format!("llm_key:{}", api_key_name),
            format!("llm_provider:{}", provider),
            format!("llm_model:{}", model),
            format!("llm_strategy:{}", strategy_id),
        ];

        for metric_type in metric_types {
            // Try to read existing data for this minute
            if let Ok(existing) = self.db.query_metrics(
                &metric_type,
                minute_timestamp,
                minute_timestamp + chrono::Duration::minutes(1),
            ) {
                if let Some(existing_row) = existing.first() {
                    // Merge with existing data
                    let merged_row = MetricRow {
                        timestamp: minute_timestamp,
                        granularity: Granularity::Minute,
                        requests: existing_row.requests + 1,
                        successful_requests: existing_row.successful_requests,
                        failed_requests: existing_row.failed_requests + 1,
                        avg_latency_ms: (existing_row.avg_latency_ms
                            * existing_row.requests as f64
                            + latency_ms as f64)
                            / (existing_row.requests + 1) as f64,
                        input_tokens: existing_row.input_tokens,
                        output_tokens: existing_row.output_tokens,
                        cost_usd: existing_row.cost_usd,
                        method_counts: None,
                        p50_latency_ms: existing_row.p50_latency_ms,
                        p95_latency_ms: existing_row.p95_latency_ms,
                        p99_latency_ms: existing_row.p99_latency_ms,
                    };

                    let _ = self.db.upsert_metric(&metric_type, &merged_row);
                } else {
                    // No existing data, insert new
                    let _ = self.db.upsert_metric(&metric_type, &row);
                }
            } else {
                // Error querying, just insert new
                let _ = self.db.upsert_metric(&metric_type, &row);
            }
        }
    }

    /// Get global metrics for a time range
    pub fn get_global_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        self.db
            .query_metrics("llm_global", start, end)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.into())
            .collect()
    }

    /// Get metrics for a specific API key
    pub fn get_key_range(
        &self,
        api_key_name: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        let metric_type = format!("llm_key:{}", api_key_name);
        self.db
            .query_metrics(&metric_type, start, end)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.into())
            .collect()
    }

    /// Get metrics for a specific provider
    pub fn get_provider_range(
        &self,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        let metric_type = format!("llm_provider:{}", provider);
        self.db
            .query_metrics(&metric_type, start, end)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.into())
            .collect()
    }

    /// Get metrics for a specific model
    pub fn get_model_range(
        &self,
        model: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        let metric_type = format!("llm_model:{}", model);
        self.db
            .query_metrics(&metric_type, start, end)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.into())
            .collect()
    }

    /// Get metrics for a specific strategy
    pub fn get_strategy_range(
        &self,
        strategy_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        let metric_type = format!("llm_strategy:{}", strategy_id);
        self.db
            .query_metrics(&metric_type, start, end)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.into())
            .collect()
    }

    /// Get all API key names
    pub fn get_api_key_names(&self) -> Vec<String> {
        self.db
            .get_metric_types()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|metric_type| {
                metric_type
                    .strip_prefix("llm_key:")
                    .map(|name| name.to_string())
            })
            .collect()
    }

    /// Get all provider names
    pub fn get_provider_names(&self) -> Vec<String> {
        self.db
            .get_metric_types()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|metric_type| {
                metric_type
                    .strip_prefix("llm_provider:")
                    .map(|name| name.to_string())
            })
            .collect()
    }

    /// Get all model names
    pub fn get_model_names(&self) -> Vec<String> {
        self.db
            .get_metric_types()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|metric_type| {
                metric_type
                    .strip_prefix("llm_model:")
                    .map(|name| name.to_string())
            })
            .collect()
    }

    /// Get all strategy IDs
    pub fn get_strategy_ids(&self) -> Vec<String> {
        self.db
            .get_metric_types()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|metric_type| {
                metric_type
                    .strip_prefix("llm_strategy:")
                    .map(|id| id.to_string())
            })
            .collect()
    }

    /// Get recent usage for a strategy within a time window (for rate limiting)
    /// Returns (total_requests, total_tokens, total_cost)
    pub fn get_recent_usage_for_strategy(
        &self,
        strategy_id: &str,
        window_secs: i64,
    ) -> (u64, u64, f64) {
        let now = Utc::now();
        let start = now - chrono::Duration::seconds(window_secs);

        let metric_type = format!("llm_strategy:{}", strategy_id);
        let data_points = self
            .db
            .query_metrics(&metric_type, start, now)
            .unwrap_or_default();

        let total_requests: u64 = data_points.iter().map(|p| p.requests).sum();
        let total_tokens: u64 = data_points
            .iter()
            .map(|p| p.input_tokens.unwrap_or(0) + p.output_tokens.unwrap_or(0))
            .sum();
        let total_cost: f64 = data_points.iter().map(|p| p.cost_usd.unwrap_or(0.0)).sum();

        (total_requests, total_tokens, total_cost)
    }

    /// Calculate pre-estimate for tokens/cost based on rolling average
    /// Returns (avg_tokens_per_request, avg_cost_per_request)
    /// Uses lookback_minutes of recent history to calculate average
    pub fn get_pre_estimate_for_strategy(
        &self,
        strategy_id: &str,
        lookback_minutes: i64,
    ) -> (f64, f64) {
        let now = Utc::now();
        let start = now - chrono::Duration::minutes(lookback_minutes);

        let metric_type = format!("llm_strategy:{}", strategy_id);
        let data_points = self
            .db
            .query_metrics(&metric_type, start, now)
            .unwrap_or_default();

        let total_requests: u64 = data_points.iter().map(|p| p.requests).sum();
        if total_requests == 0 {
            // No recent data, use conservative estimates
            // 1k tokens (typical small request), $0.00 cost (assume free until proven otherwise)
            // Note: We use 0.0 for cost because:
            // 1. Many providers are free (Ollama, LMStudio, local models)
            // 2. Router checks "if avg_cost == 0.0" to skip cost limits for free models
            // 3. After first request, actual cost will be recorded and used
            return (1000.0, 0.0);
        }

        let total_tokens: u64 = data_points
            .iter()
            .map(|p| p.input_tokens.unwrap_or(0) + p.output_tokens.unwrap_or(0))
            .sum();
        let total_cost: f64 = data_points.iter().map(|p| p.cost_usd.unwrap_or(0.0)).sum();

        let avg_tokens = total_tokens as f64 / total_requests as f64;
        let avg_cost = total_cost / total_requests as f64;

        (avg_tokens, avg_cost)
    }

    /// Get MCP metrics collector
    pub fn mcp(&self) -> &McpMetricsCollector {
        &self.mcp_metrics
    }

    /// Get database reference for aggregation task
    pub fn db(&self) -> Arc<MetricsDatabase> {
        self.db.clone()
    }

    /// Clean up old metrics data (called by aggregation task)
    pub fn cleanup(&self) {
        if let Err(e) = self.db.cleanup_old_data() {
            tracing::error!("Failed to cleanup metrics: {}", e);
        }

        // Clean up MCP metrics
        self.mcp_metrics.cleanup();
    }

    /// Get total number of data points in global metrics (for testing)
    pub fn global_data_point_count(&self) -> usize {
        let now = Utc::now();
        let start = now - chrono::Duration::days(90);
        self.get_global_range(start, now).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    fn create_test_collector() -> (MetricsCollector, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Arc::new(MetricsDatabase::new(db_path).unwrap());
        (MetricsCollector::new(db), dir)
    }

    #[test]
    fn test_metrics_collector_record_success() {
        let (collector, _dir) = create_test_collector();

        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.05,
            latency_ms: 1000,
            strategy_id: "default",
        });

        let now = Utc::now();
        let start = now - Duration::hours(1);
        let end = now + Duration::hours(1);

        let global_metrics = collector.get_global_range(start, end);
        assert_eq!(global_metrics.len(), 1);
        assert_eq!(global_metrics[0].requests, 1);

        let key_metrics = collector.get_key_range("key1", start, end);
        assert_eq!(key_metrics.len(), 1);
        assert_eq!(key_metrics[0].requests, 1);

        let provider_metrics = collector.get_provider_range("openai", start, end);
        assert_eq!(provider_metrics.len(), 1);
        assert_eq!(provider_metrics[0].requests, 1);

        let model_metrics = collector.get_model_range("gpt-4", start, end);
        assert_eq!(model_metrics.len(), 1);
        assert_eq!(model_metrics[0].requests, 1);
    }

    #[test]
    fn test_metrics_collector_record_failure() {
        let (collector, _dir) = create_test_collector();

        collector.record_failure("key1", "openai", "gpt-4", "default", 500);

        let now = Utc::now();
        let start = now - Duration::hours(1);
        let end = now + Duration::hours(1);

        let global_metrics = collector.get_global_range(start, end);
        assert_eq!(global_metrics.len(), 1);
        assert_eq!(global_metrics[0].failed_requests, 1);
    }

    #[test]
    fn test_metrics_collector_get_names() {
        let (collector, _dir) = create_test_collector();

        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.05,
            latency_ms: 1000,
            strategy_id: "default",
        });
        collector.record_success(&RequestMetrics {
            api_key_name: "key2",
            provider: "ollama",
            model: "llama3.3",
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.0,
            latency_ms: 1000,
            strategy_id: "default",
        });

        let key_names = collector.get_api_key_names();
        assert_eq!(key_names.len(), 2);
        assert!(key_names.contains(&"key1".to_string()));
        assert!(key_names.contains(&"key2".to_string()));

        let provider_names = collector.get_provider_names();
        assert_eq!(provider_names.len(), 2);
        assert!(provider_names.contains(&"openai".to_string()));
        assert!(provider_names.contains(&"ollama".to_string()));

        let model_names = collector.get_model_names();
        assert_eq!(model_names.len(), 2);
        assert!(model_names.contains(&"gpt-4".to_string()));
        assert!(model_names.contains(&"llama3.3".to_string()));
    }

    #[test]
    fn test_four_tier_isolation() {
        let (collector, _dir) = create_test_collector();
        let now = Utc::now();

        // Record metrics for different combinations
        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.05,
            latency_ms: 100,
            strategy_id: "default",
        });
        collector.record_success(&RequestMetrics {
            api_key_name: "key1",
            provider: "anthropic",
            model: "claude-3.5-sonnet",
            input_tokens: 800,
            output_tokens: 400,
            cost_usd: 0.03,
            latency_ms: 80,
            strategy_id: "default",
        });
        collector.record_success(&RequestMetrics {
            api_key_name: "key2",
            provider: "openai",
            model: "gpt-4",
            input_tokens: 500,
            output_tokens: 250,
            cost_usd: 0.025,
            latency_ms: 90,
            strategy_id: "default",
        });

        // Verify global (all requests)
        let global =
            collector.get_global_range(now - Duration::minutes(5), now + Duration::minutes(5));
        assert_eq!(global[0].requests, 3);

        // Verify per-key isolation
        let key1 = collector.get_key_range(
            "key1",
            now - Duration::minutes(5),
            now + Duration::minutes(5),
        );
        assert_eq!(key1[0].requests, 2);

        let key2 = collector.get_key_range(
            "key2",
            now - Duration::minutes(5),
            now + Duration::minutes(5),
        );
        assert_eq!(key2[0].requests, 1);

        // Verify per-provider isolation
        let openai = collector.get_provider_range(
            "openai",
            now - Duration::minutes(5),
            now + Duration::minutes(5),
        );
        assert_eq!(openai[0].requests, 2);

        let anthropic = collector.get_provider_range(
            "anthropic",
            now - Duration::minutes(5),
            now + Duration::minutes(5),
        );
        assert_eq!(anthropic[0].requests, 1);

        // Verify per-model isolation
        let gpt4 = collector.get_model_range(
            "gpt-4",
            now - Duration::minutes(5),
            now + Duration::minutes(5),
        );
        assert_eq!(gpt4[0].requests, 2); // Both key1 and key2 used gpt-4
    }
}
