//! MCP (Model Context Protocol) metrics collection
//!
//! Tracks MCP request metrics for the last 24 hours at 1-minute granularity.
//! Unlike LLM metrics, MCP metrics focus on request count, latency, and method-level breakdown
//! without token or cost tracking.

#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// MCP request metrics for recording
#[derive(Debug, Clone)]
pub struct McpRequestMetrics<'a> {
    pub client_id: &'a str,
    pub server_id: &'a str,
    pub method: &'a str,        // JSON-RPC method name
    pub latency_ms: u64,
    pub success: bool,
    pub error_code: Option<i32>, // JSON-RPC error code if failed
}

/// Per-method metrics aggregation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetrics {
    pub count: u64,
    pub successful: u64,
    pub failed: u64,
    pub total_latency_ms: u64,
}

impl MethodMetrics {
    fn new() -> Self {
        Self {
            count: 0,
            successful: 0,
            failed: 0,
            total_latency_ms: 0,
        }
    }

    fn add_request(&mut self, success: bool, latency_ms: u64) {
        self.count += 1;
        if success {
            self.successful += 1;
        } else {
            self.failed += 1;
        }
        self.total_latency_ms += latency_ms;
    }

    pub fn avg_latency_ms(&self) -> f64 {
        if self.count > 0 {
            self.total_latency_ms as f64 / self.count as f64
        } else {
            0.0
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.count > 0 {
            self.successful as f64 / self.count as f64
        } else {
            0.0
        }
    }
}

/// Time-series data point for MCP metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpMetricDataPoint {
    /// Timestamp (rounded to minute)
    pub timestamp: DateTime<Utc>,

    /// Number of requests
    pub requests: u64,

    /// Number of successful requests
    pub successful_requests: u64,

    /// Number of failed requests
    pub failed_requests: u64,

    /// Total latency (sum for averaging)
    pub total_latency_ms: u64,

    /// Latency samples (for percentile calculation)
    pub latency_samples: Vec<u64>,

    /// Method-level breakdown
    pub method_counts: HashMap<String, MethodMetrics>,
}

impl McpMetricDataPoint {
    /// Create a new empty metric data point
    fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            timestamp,
            requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            total_latency_ms: 0,
            latency_samples: Vec::new(),
            method_counts: HashMap::new(),
        }
    }

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

    /// Add a request to this data point
    fn add_request(&mut self, method: &str, success: bool, latency_ms: u64) {
        self.requests += 1;
        if success {
            self.successful_requests += 1;
        } else {
            self.failed_requests += 1;
        }
        self.total_latency_ms += latency_ms;
        self.latency_samples.push(latency_ms);

        // Update method-level metrics
        self.method_counts
            .entry(method.to_string())
            .or_insert_with(MethodMetrics::new)
            .add_request(success, latency_ms);
    }
}

/// Time-series MCP metrics storage
#[derive(Debug, Clone)]
struct McpTimeSeries {
    /// Map of timestamp (minute) to data point
    data: Arc<DashMap<i64, McpMetricDataPoint>>,
}

impl McpTimeSeries {
    /// Create a new time series
    fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
        }
    }

    /// Get the minute timestamp (rounded down)
    fn get_minute_timestamp(timestamp: DateTime<Utc>) -> i64 {
        timestamp.timestamp() / 60 * 60
    }

    /// Record a request
    fn record(&self, timestamp: DateTime<Utc>, method: &str, success: bool, latency_ms: u64) {
        let minute_ts = Self::get_minute_timestamp(timestamp);

        self.data
            .entry(minute_ts)
            .and_modify(|point| {
                point.add_request(method, success, latency_ms);
            })
            .or_insert_with(|| {
                let rounded_time =
                    DateTime::from_timestamp(minute_ts, 0).unwrap_or_else(Utc::now);
                let mut point = McpMetricDataPoint::new(rounded_time);
                point.add_request(method, success, latency_ms);
                point
            });
    }

    /// Get all data points in a time range
    fn get_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<McpMetricDataPoint> {
        let start_ts = Self::get_minute_timestamp(start);
        let end_ts = Self::get_minute_timestamp(end);

        let mut points: Vec<McpMetricDataPoint> = self
            .data
            .iter()
            .filter(|entry| {
                let ts = *entry.key();
                ts >= start_ts && ts <= end_ts
            })
            .map(|entry| entry.value().clone())
            .collect();

        points.sort_by_key(|p| p.timestamp);
        points
    }

    /// Clean up data older than retention period
    fn cleanup(&self, retention_hours: i64) {
        let cutoff = Utc::now() - Duration::hours(retention_hours);
        let cutoff_ts = Self::get_minute_timestamp(cutoff);

        self.data.retain(|ts, _| *ts >= cutoff_ts);
    }

    /// Insert a pre-computed data point (for log repopulation)
    fn insert_point(&self, point: McpMetricDataPoint) {
        let minute_ts = Self::get_minute_timestamp(point.timestamp);
        self.data.insert(minute_ts, point);
    }
}

/// MCP metrics collector for tracking usage
pub struct McpMetricsCollector {
    /// Global MCP metrics
    global: McpTimeSeries,

    /// Per-client metrics
    per_client: Arc<DashMap<String, McpTimeSeries>>,

    /// Per-MCP-server metrics
    per_server: Arc<DashMap<String, McpTimeSeries>>,

    /// Retention period in hours
    retention_hours: i64,
}

impl McpMetricsCollector {
    /// Create a new MCP metrics collector
    pub fn new(retention_hours: i64) -> Self {
        Self {
            global: McpTimeSeries::new(),
            per_client: Arc::new(DashMap::new()),
            per_server: Arc::new(DashMap::new()),
            retention_hours,
        }
    }

    /// Create a new MCP metrics collector with default 24-hour retention
    pub fn with_default_retention() -> Self {
        Self::new(24)
    }

    /// Record an MCP request
    pub fn record(&self, metrics: &McpRequestMetrics) {
        self.record_at(metrics, Utc::now());
    }

    /// Record an MCP request at a specific timestamp (for testing/repopulation)
    pub fn record_at(&self, metrics: &McpRequestMetrics, timestamp: DateTime<Utc>) {
        // Record in global metrics
        self.global
            .record(timestamp, metrics.method, metrics.success, metrics.latency_ms);

        // Record in per-client metrics
        self.per_client
            .entry(metrics.client_id.to_string())
            .or_insert_with(McpTimeSeries::new)
            .record(timestamp, metrics.method, metrics.success, metrics.latency_ms);

        // Record in per-server metrics
        self.per_server
            .entry(metrics.server_id.to_string())
            .or_insert_with(McpTimeSeries::new)
            .record(timestamp, metrics.method, metrics.success, metrics.latency_ms);
    }

    /// Get global MCP metrics for a time range
    pub fn get_global_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<McpMetricDataPoint> {
        self.global.get_range(start, end)
    }

    /// Get metrics for a specific client
    pub fn get_client_range(
        &self,
        client_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<McpMetricDataPoint> {
        self.per_client
            .get(client_id)
            .map(|ts| ts.get_range(start, end))
            .unwrap_or_default()
    }

    /// Get metrics for a specific MCP server
    pub fn get_server_range(
        &self,
        server_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<McpMetricDataPoint> {
        self.per_server
            .get(server_id)
            .map(|ts| ts.get_range(start, end))
            .unwrap_or_default()
    }

    /// Get all client IDs
    pub fn get_client_ids(&self) -> Vec<String> {
        self.per_client
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all MCP server IDs
    pub fn get_server_ids(&self) -> Vec<String> {
        self.per_server
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clean up old data
    pub fn cleanup(&self) {
        self.global.cleanup(self.retention_hours);

        self.per_client.iter().for_each(|entry| {
            entry.value().cleanup(self.retention_hours);
        });

        self.per_server.iter().for_each(|entry| {
            entry.value().cleanup(self.retention_hours);
        });
    }

    /// Repopulate in-memory metrics from parsed log data
    pub fn repopulate_from_logs(
        &self,
        log_data: HashMap<String, Vec<McpMetricDataPoint>>,
    ) -> anyhow::Result<()> {
        for (key, data_points) in log_data {
            if key == "global" {
                for point in data_points {
                    self.global.insert_point(point);
                }
            } else if let Some(client_id) = key.strip_prefix("client:") {
                for point in data_points {
                    self.per_client
                        .entry(client_id.to_string())
                        .or_insert_with(McpTimeSeries::new)
                        .insert_point(point);
                }
            } else if let Some(server_id) = key.strip_prefix("server:") {
                for point in data_points {
                    self.per_server
                        .entry(server_id.to_string())
                        .or_insert_with(McpTimeSeries::new)
                        .insert_point(point);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_metric_data_point_new() {
        let timestamp = Utc::now();
        let point = McpMetricDataPoint::new(timestamp);

        assert_eq!(point.requests, 0);
        assert_eq!(point.successful_requests, 0);
        assert_eq!(point.failed_requests, 0);
        assert_eq!(point.total_latency_ms, 0);
        assert!(point.latency_samples.is_empty());
        assert!(point.method_counts.is_empty());
    }

    #[test]
    fn test_add_request() {
        let timestamp = Utc::now();
        let mut point = McpMetricDataPoint::new(timestamp);

        point.add_request("tools/list", true, 100);
        assert_eq!(point.requests, 1);
        assert_eq!(point.successful_requests, 1);
        assert_eq!(point.failed_requests, 0);
        assert_eq!(point.total_latency_ms, 100);
        assert_eq!(point.latency_samples.len(), 1);
        assert_eq!(point.method_counts.len(), 1);
        assert_eq!(point.method_counts.get("tools/list").unwrap().count, 1);

        point.add_request("resources/read", false, 200);
        assert_eq!(point.requests, 2);
        assert_eq!(point.successful_requests, 1);
        assert_eq!(point.failed_requests, 1);
        assert_eq!(point.total_latency_ms, 300);
        assert_eq!(point.method_counts.len(), 2);
    }

    #[test]
    fn test_method_metrics() {
        let mut metrics = MethodMetrics::new();
        metrics.add_request(true, 100);
        metrics.add_request(true, 150);
        metrics.add_request(false, 200);

        assert_eq!(metrics.count, 3);
        assert_eq!(metrics.successful, 2);
        assert_eq!(metrics.failed, 1);
        assert_eq!(metrics.total_latency_ms, 450);
        assert_eq!(metrics.avg_latency_ms(), 150.0);
        assert_eq!(metrics.success_rate(), 2.0 / 3.0);
    }

    #[test]
    fn test_mcp_time_series_record() {
        let ts = McpTimeSeries::new();
        let timestamp = Utc::now();

        ts.record(timestamp, "tools/list", true, 100);
        ts.record(timestamp, "resources/read", true, 150);

        let points = ts.get_range(
            timestamp - Duration::minutes(5),
            timestamp + Duration::minutes(5),
        );

        assert_eq!(points.len(), 1);
        assert_eq!(points[0].requests, 2);
        assert_eq!(points[0].successful_requests, 2);
        assert_eq!(points[0].method_counts.len(), 2);
    }

    #[test]
    fn test_mcp_metrics_collector() {
        let collector = McpMetricsCollector::with_default_retention();
        let timestamp = Utc::now();

        let metrics = McpRequestMetrics {
            client_id: "client-1",
            server_id: "server-1",
            method: "tools/list",
            latency_ms: 100,
            success: true,
            error_code: None,
        };

        collector.record_at(&metrics, timestamp);

        // Check global metrics
        let global_points = collector.get_global_range(
            timestamp - Duration::minutes(5),
            timestamp + Duration::minutes(5),
        );
        assert_eq!(global_points.len(), 1);
        assert_eq!(global_points[0].requests, 1);

        // Check client metrics
        let client_points = collector.get_client_range(
            "client-1",
            timestamp - Duration::minutes(5),
            timestamp + Duration::minutes(5),
        );
        assert_eq!(client_points.len(), 1);
        assert_eq!(client_points[0].requests, 1);

        // Check server metrics
        let server_points = collector.get_server_range(
            "server-1",
            timestamp - Duration::minutes(5),
            timestamp + Duration::minutes(5),
        );
        assert_eq!(server_points.len(), 1);
        assert_eq!(server_points[0].requests, 1);
    }

    #[test]
    fn test_latency_percentile() {
        let timestamp = Utc::now();
        let mut point = McpMetricDataPoint::new(timestamp);

        point.add_request("tools/list", true, 100);
        point.add_request("tools/list", true, 200);
        point.add_request("tools/list", true, 300);
        point.add_request("tools/list", true, 400);
        point.add_request("tools/list", true, 500);

        assert_eq!(point.latency_percentile(0.0), 100);
        assert_eq!(point.latency_percentile(50.0), 300);
        assert_eq!(point.latency_percentile(100.0), 500);
    }

    #[test]
    fn test_cleanup() {
        let collector = McpMetricsCollector::new(1); // 1-hour retention
        let now = Utc::now();
        let old_timestamp = now - Duration::hours(2);

        // Record old data
        let old_metrics = McpRequestMetrics {
            client_id: "client-1",
            server_id: "server-1",
            method: "tools/list",
            latency_ms: 100,
            success: true,
            error_code: None,
        };
        collector.record_at(&old_metrics, old_timestamp);

        // Record new data
        let new_metrics = McpRequestMetrics {
            client_id: "client-1",
            server_id: "server-1",
            method: "tools/list",
            latency_ms: 100,
            success: true,
            error_code: None,
        };
        collector.record_at(&new_metrics, now);

        // Cleanup should remove old data
        collector.cleanup();

        let global_points = collector.get_global_range(
            old_timestamp - Duration::minutes(5),
            now + Duration::minutes(5),
        );

        // Should only have the recent data point
        assert_eq!(global_points.len(), 1);
        assert!(global_points[0].timestamp >= now - Duration::hours(1));
    }
}
