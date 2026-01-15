//! In-memory metrics collection
//!
//! Tracks usage metrics for the last 24 hours at 1-minute granularity.

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Time-series data point for metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDataPoint {
    /// Timestamp (rounded to minute)
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
    /// Create a new empty metric data point
    fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            timestamp,
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            total_latency_ms: 0,
            successful_requests: 0,
            failed_requests: 0,
            latency_samples: Vec::new(),
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

    /// Add a successful request to this data point
    fn add_success(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
    ) {
        self.requests += 1;
        self.successful_requests += 1;
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
        self.total_tokens += input_tokens + output_tokens;
        self.cost_usd += cost_usd;
        self.total_latency_ms += latency_ms;
        self.latency_samples.push(latency_ms);
    }

    /// Add a failed request to this data point
    fn add_failure(&mut self, latency_ms: u64) {
        self.requests += 1;
        self.failed_requests += 1;
        self.total_latency_ms += latency_ms;
        self.latency_samples.push(latency_ms);
    }
}

/// Time-series metrics storage
#[derive(Debug, Clone)]
struct TimeSeries {
    /// Map of timestamp (minute) to data point
    data: Arc<DashMap<i64, MetricDataPoint>>,
}

impl TimeSeries {
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

    /// Get or create a data point for a given timestamp
    #[allow(dead_code)]
    fn get_or_create_point(&self, timestamp: DateTime<Utc>) -> MetricDataPoint {
        let minute_ts = Self::get_minute_timestamp(timestamp);

        self.data
            .entry(minute_ts)
            .or_insert_with(|| {
                let rounded_time =
                    DateTime::from_timestamp(minute_ts, 0).unwrap_or_else(Utc::now);
                MetricDataPoint::new(rounded_time)
            })
            .clone()
    }

    /// Record a successful request
    fn record_success(
        &self,
        timestamp: DateTime<Utc>,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
    ) {
        let minute_ts = Self::get_minute_timestamp(timestamp);

        self.data
            .entry(minute_ts)
            .and_modify(|point| {
                point.add_success(input_tokens, output_tokens, cost_usd, latency_ms);
            })
            .or_insert_with(|| {
                let rounded_time =
                    DateTime::from_timestamp(minute_ts, 0).unwrap_or_else(Utc::now);
                let mut point = MetricDataPoint::new(rounded_time);
                point.add_success(input_tokens, output_tokens, cost_usd, latency_ms);
                point
            });
    }

    /// Record a failed request
    fn record_failure(&self, timestamp: DateTime<Utc>, latency_ms: u64) {
        let minute_ts = Self::get_minute_timestamp(timestamp);

        self.data
            .entry(minute_ts)
            .and_modify(|point| {
                point.add_failure(latency_ms);
            })
            .or_insert_with(|| {
                let rounded_time =
                    DateTime::from_timestamp(minute_ts, 0).unwrap_or_else(Utc::now);
                let mut point = MetricDataPoint::new(rounded_time);
                point.add_failure(latency_ms);
                point
            });
    }

    /// Get all data points in a time range
    fn get_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<MetricDataPoint> {
        let start_ts = Self::get_minute_timestamp(start);
        let end_ts = Self::get_minute_timestamp(end);

        let mut points: Vec<MetricDataPoint> = self
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

    /// Get total count of data points
    fn len(&self) -> usize {
        self.data.len()
    }
}

/// Metrics collector for tracking usage
pub struct MetricsCollector {
    /// Global metrics
    global: TimeSeries,

    /// Per-API-key metrics
    per_key: Arc<DashMap<String, TimeSeries>>,

    /// Per-provider metrics
    per_provider: Arc<DashMap<String, TimeSeries>>,

    /// Retention period in hours
    retention_hours: i64,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(retention_hours: i64) -> Self {
        Self {
            global: TimeSeries::new(),
            per_key: Arc::new(DashMap::new()),
            per_provider: Arc::new(DashMap::new()),
            retention_hours,
        }
    }

    /// Create a new metrics collector with default 24-hour retention
    pub fn with_default_retention() -> Self {
        Self::new(24)
    }

    /// Record a successful request
    pub fn record_success(
        &self,
        api_key_name: &str,
        provider: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        latency_ms: u64,
    ) {
        let timestamp = Utc::now();

        // Record in global metrics
        self.global
            .record_success(timestamp, input_tokens, output_tokens, cost_usd, latency_ms);

        // Record in per-key metrics
        self.per_key
            .entry(api_key_name.to_string())
            .or_insert_with(TimeSeries::new)
            .record_success(timestamp, input_tokens, output_tokens, cost_usd, latency_ms);

        // Record in per-provider metrics
        self.per_provider
            .entry(provider.to_string())
            .or_insert_with(TimeSeries::new)
            .record_success(timestamp, input_tokens, output_tokens, cost_usd, latency_ms);
    }

    /// Record a failed request
    pub fn record_failure(&self, api_key_name: &str, provider: &str, latency_ms: u64) {
        let timestamp = Utc::now();

        // Record in global metrics
        self.global.record_failure(timestamp, latency_ms);

        // Record in per-key metrics
        self.per_key
            .entry(api_key_name.to_string())
            .or_insert_with(TimeSeries::new)
            .record_failure(timestamp, latency_ms);

        // Record in per-provider metrics
        self.per_provider
            .entry(provider.to_string())
            .or_insert_with(TimeSeries::new)
            .record_failure(timestamp, latency_ms);
    }

    /// Get global metrics for a time range
    pub fn get_global_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        self.global.get_range(start, end)
    }

    /// Get metrics for a specific API key
    pub fn get_key_range(
        &self,
        api_key_name: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        self.per_key
            .get(api_key_name)
            .map(|ts| ts.get_range(start, end))
            .unwrap_or_default()
    }

    /// Get metrics for a specific provider
    pub fn get_provider_range(
        &self,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<MetricDataPoint> {
        self.per_provider
            .get(provider)
            .map(|ts| ts.get_range(start, end))
            .unwrap_or_default()
    }

    /// Get all API key names
    pub fn get_api_key_names(&self) -> Vec<String> {
        self.per_key
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all provider names
    pub fn get_provider_names(&self) -> Vec<String> {
        self.per_provider
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clean up old metrics data
    pub fn cleanup(&self) {
        self.global.cleanup(self.retention_hours);

        for entry in self.per_key.iter() {
            entry.value().cleanup(self.retention_hours);
        }

        for entry in self.per_provider.iter() {
            entry.value().cleanup(self.retention_hours);
        }
    }

    /// Get total number of data points in global metrics
    pub fn global_data_point_count(&self) -> usize {
        self.global.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_data_point_creation() {
        let now = Utc::now();
        let point = MetricDataPoint::new(now);

        assert_eq!(point.requests, 0);
        assert_eq!(point.input_tokens, 0);
        assert_eq!(point.output_tokens, 0);
        assert_eq!(point.total_tokens, 0);
        assert_eq!(point.cost_usd, 0.0);
    }

    #[test]
    fn test_metric_data_point_add_success() {
        let now = Utc::now();
        let mut point = MetricDataPoint::new(now);

        point.add_success(100, 200, 0.05, 1000);

        assert_eq!(point.requests, 1);
        assert_eq!(point.successful_requests, 1);
        assert_eq!(point.input_tokens, 100);
        assert_eq!(point.output_tokens, 200);
        assert_eq!(point.total_tokens, 300);
        assert_eq!(point.cost_usd, 0.05);
        assert_eq!(point.total_latency_ms, 1000);
        assert_eq!(point.latency_samples.len(), 1);
    }

    #[test]
    fn test_metric_data_point_add_failure() {
        let now = Utc::now();
        let mut point = MetricDataPoint::new(now);

        point.add_failure(500);

        assert_eq!(point.requests, 1);
        assert_eq!(point.failed_requests, 1);
        assert_eq!(point.successful_requests, 0);
        assert_eq!(point.total_latency_ms, 500);
    }

    #[test]
    fn test_metric_data_point_avg_latency() {
        let now = Utc::now();
        let mut point = MetricDataPoint::new(now);

        point.add_success(100, 200, 0.05, 1000);
        point.add_success(100, 200, 0.05, 2000);

        assert_eq!(point.avg_latency_ms(), 1500.0);
    }

    #[test]
    fn test_metric_data_point_success_rate() {
        let now = Utc::now();
        let mut point = MetricDataPoint::new(now);

        point.add_success(100, 200, 0.05, 1000);
        point.add_success(100, 200, 0.05, 1000);
        point.add_failure(500);

        assert_eq!(point.success_rate(), 2.0 / 3.0);
    }

    #[test]
    fn test_metric_data_point_percentile() {
        let now = Utc::now();
        let mut point = MetricDataPoint::new(now);

        point.add_success(100, 200, 0.05, 100);
        point.add_success(100, 200, 0.05, 200);
        point.add_success(100, 200, 0.05, 300);
        point.add_success(100, 200, 0.05, 400);
        point.add_success(100, 200, 0.05, 500);

        assert_eq!(point.latency_percentile(50.0), 300);
        assert_eq!(point.latency_percentile(95.0), 400); // 95% of [100,200,300,400,500] -> index 3
    }

    #[test]
    fn test_time_series_record_success() {
        let ts = TimeSeries::new();
        let now = Utc::now();

        ts.record_success(now, 100, 200, 0.05, 1000);

        assert_eq!(ts.len(), 1);
    }

    #[test]
    fn test_time_series_get_range() {
        let ts = TimeSeries::new();
        let now = Utc::now();

        ts.record_success(now, 100, 200, 0.05, 1000);
        ts.record_success(now - Duration::minutes(5), 100, 200, 0.05, 1000);
        ts.record_success(now - Duration::minutes(10), 100, 200, 0.05, 1000);

        let start = now - Duration::minutes(15);
        let end = now;

        let points = ts.get_range(start, end);
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn test_time_series_cleanup() {
        let ts = TimeSeries::new();
        let now = Utc::now();

        ts.record_success(now, 100, 200, 0.05, 1000);
        ts.record_success(now - Duration::hours(25), 100, 200, 0.05, 1000);

        assert_eq!(ts.len(), 2);

        ts.cleanup(24);

        assert_eq!(ts.len(), 1);
    }

    #[test]
    fn test_metrics_collector_record_success() {
        let collector = MetricsCollector::with_default_retention();

        collector.record_success("key1", "openai", 100, 200, 0.05, 1000);

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
    }

    #[test]
    fn test_metrics_collector_record_failure() {
        let collector = MetricsCollector::with_default_retention();

        collector.record_failure("key1", "openai", 500);

        let now = Utc::now();
        let start = now - Duration::hours(1);
        let end = now + Duration::hours(1);

        let global_metrics = collector.get_global_range(start, end);
        assert_eq!(global_metrics.len(), 1);
        assert_eq!(global_metrics[0].failed_requests, 1);
    }

    #[test]
    fn test_metrics_collector_get_names() {
        let collector = MetricsCollector::with_default_retention();

        collector.record_success("key1", "openai", 100, 200, 0.05, 1000);
        collector.record_success("key2", "ollama", 100, 200, 0.0, 1000);

        let key_names = collector.get_api_key_names();
        assert_eq!(key_names.len(), 2);
        assert!(key_names.contains(&"key1".to_string()));
        assert!(key_names.contains(&"key2".to_string()));

        let provider_names = collector.get_provider_names();
        assert_eq!(provider_names.len(), 2);
        assert!(provider_names.contains(&"openai".to_string()));
        assert!(provider_names.contains(&"ollama".to_string()));
    }
}
