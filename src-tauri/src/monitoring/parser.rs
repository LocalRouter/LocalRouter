//! Historical log parser
//!
//! Parses access logs from disk and provides query capabilities.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, warn};

use super::logger::AccessLogEntry;
use super::metrics::MetricDataPoint;
use crate::utils::errors::{AppError, AppResult};

/// Cache entry for parsed log data
#[derive(Debug, Clone)]
struct CacheEntry {
    /// Parsed data points
    data: Vec<MetricDataPoint>,

    /// Timestamp when this entry was cached
    cached_at: DateTime<Utc>,
}

/// Historical log parser with caching
pub struct LogParser {
    /// Log directory
    log_dir: PathBuf,

    /// Cache of parsed results (key: cache_key, value: parsed data)
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,

    /// Cache TTL in seconds
    cache_ttl_seconds: i64,
}

impl LogParser {
    /// Create a new log parser
    pub fn new(log_dir: PathBuf, cache_ttl_seconds: i64) -> Self {
        Self {
            log_dir,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl_seconds,
        }
    }

    /// Create cache key for a query
    fn cache_key(
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        api_key_filter: Option<&str>,
        provider_filter: Option<&str>,
    ) -> String {
        format!(
            "{}:{}:{}:{}",
            start.timestamp(),
            end.timestamp(),
            api_key_filter.unwrap_or("*"),
            provider_filter.unwrap_or("*")
        )
    }

    /// Check if cache entry is still valid
    fn is_cache_valid(&self, entry: &CacheEntry) -> bool {
        let age = Utc::now().signed_duration_since(entry.cached_at);
        age.num_seconds() < self.cache_ttl_seconds
    }

    /// Get log file path for a specific date
    fn get_log_file_path(&self, date: &str) -> PathBuf {
        self.log_dir.join(format!("localrouter-{}.log", date))
    }

    /// Parse a single log file
    fn parse_log_file(&self, path: &PathBuf) -> AppResult<Vec<AccessLogEntry>> {
        let file = File::open(path)
            .map_err(|e| AppError::Internal(format!("Failed to open log file: {}", e)))?;

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            match line_result {
                Ok(line) => {
                    if line.trim().is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<AccessLogEntry>(&line) {
                        Ok(entry) => entries.push(entry),
                        Err(e) => {
                            warn!(
                                "Failed to parse log entry at {:?}:{}: {}",
                                path,
                                line_num + 1,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to read line from {:?}: {}", path, e);
                }
            }
        }

        Ok(entries)
    }

    /// Get all log file paths in a date range
    fn get_log_files_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut current_date = start.date_naive();
        let end_date = end.date_naive();

        while current_date <= end_date {
            let date_str = current_date.format("%Y-%m-%d").to_string();
            let log_file = self.get_log_file_path(&date_str);

            if log_file.exists() {
                files.push(log_file);
            }

            current_date = current_date
                .succ_opt()
                .ok_or_else(|| AppError::Internal("Date overflow".to_string()))?;
        }

        Ok(files)
    }

    /// Filter log entries by criteria
    fn filter_entries(
        &self,
        entries: Vec<AccessLogEntry>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        api_key_filter: Option<&str>,
        provider_filter: Option<&str>,
    ) -> Vec<AccessLogEntry> {
        entries
            .into_iter()
            .filter(|entry| {
                // Filter by time range
                if entry.timestamp < start || entry.timestamp > end {
                    return false;
                }

                // Filter by API key
                if let Some(key_filter) = api_key_filter {
                    if entry.api_key_name != key_filter {
                        return false;
                    }
                }

                // Filter by provider
                if let Some(prov_filter) = provider_filter {
                    if entry.provider != prov_filter {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Aggregate log entries into time-series data points
    fn aggregate_by_minute(&self, entries: Vec<AccessLogEntry>) -> Vec<MetricDataPoint> {
        let mut points_map: HashMap<i64, MetricDataPoint> = HashMap::new();

        for entry in entries {
            // Round timestamp to minute
            let minute_ts = entry.timestamp.timestamp() / 60 * 60;

            let point = points_map.entry(minute_ts).or_insert_with(|| {
                let rounded_time =
                    DateTime::from_timestamp(minute_ts, 0).unwrap_or_else(Utc::now);
                MetricDataPoint {
                    timestamp: rounded_time,
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
            });

            // Update the data point
            point.requests += 1;

            if entry.status == "success" {
                point.successful_requests += 1;
                point.input_tokens += entry.input_tokens;
                point.output_tokens += entry.output_tokens;
                point.total_tokens += entry.total_tokens;
                point.cost_usd += entry.cost_usd;
            } else {
                point.failed_requests += 1;
            }

            point.total_latency_ms += entry.latency_ms;
            point.latency_samples.push(entry.latency_ms);
        }

        // Convert to sorted vector
        let mut points: Vec<MetricDataPoint> = points_map.into_values().collect();
        points.sort_by_key(|p| p.timestamp);

        points
    }

    /// Query historical logs with optional caching
    pub fn query(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        api_key_filter: Option<&str>,
        provider_filter: Option<&str>,
    ) -> AppResult<Vec<MetricDataPoint>> {
        // Check cache first
        let cache_key = Self::cache_key(start, end, api_key_filter, provider_filter);

        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(&cache_key) {
                if self.is_cache_valid(entry) {
                    return Ok(entry.data.clone());
                }
            }
        }

        // Cache miss or expired - parse logs
        let log_files = self.get_log_files_in_range(start, end)?;
        let mut all_entries = Vec::new();

        for log_file in log_files {
            match self.parse_log_file(&log_file) {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    warn!("Failed to parse log file {:?}: {}", log_file, e);
                }
            }
        }

        // Filter entries
        let filtered =
            self.filter_entries(all_entries, start, end, api_key_filter, provider_filter);

        // Aggregate by minute
        let aggregated = self.aggregate_by_minute(filtered);

        // Update cache
        {
            let mut cache = self.cache.write();
            cache.insert(
                cache_key,
                CacheEntry {
                    data: aggregated.clone(),
                    cached_at: Utc::now(),
                },
            );
        }

        Ok(aggregated)
    }

    /// Query with only time range (no filters)
    pub fn query_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<MetricDataPoint>> {
        self.query(start, end, None, None)
    }

    /// Query for a specific API key
    pub fn query_by_api_key(
        &self,
        api_key_name: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<MetricDataPoint>> {
        self.query(start, end, Some(api_key_name), None)
    }

    /// Query for a specific provider
    pub fn query_by_provider(
        &self,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<MetricDataPoint>> {
        self.query(start, end, None, Some(provider))
    }

    /// Clear cache
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get cache size
    pub fn cache_size(&self) -> usize {
        let cache = self.cache.read();
        cache.len()
    }

    /// Clean up expired cache entries
    pub fn cleanup_cache(&self) {
        let mut cache = self.cache.write();
        cache.retain(|_, entry| self.is_cache_valid(entry));
    }
}

/// Summary statistics for a time period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSummary {
    /// Total requests
    pub total_requests: u64,

    /// Successful requests
    pub successful_requests: u64,

    /// Failed requests
    pub failed_requests: u64,

    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,

    /// Total input tokens
    pub total_input_tokens: u64,

    /// Total output tokens
    pub total_output_tokens: u64,

    /// Total tokens
    pub total_tokens: u64,

    /// Total cost in USD
    pub total_cost_usd: f64,

    /// Average latency in milliseconds
    pub avg_latency_ms: f64,

    /// Median latency in milliseconds
    pub median_latency_ms: u64,

    /// 95th percentile latency in milliseconds
    pub p95_latency_ms: u64,

    /// 99th percentile latency in milliseconds
    pub p99_latency_ms: u64,
}

impl LogSummary {
    /// Calculate summary from metric data points
    pub fn from_data_points(points: &[MetricDataPoint]) -> Self {
        let total_requests: u64 = points.iter().map(|p| p.requests).sum();
        let successful_requests: u64 = points.iter().map(|p| p.successful_requests).sum();
        let failed_requests: u64 = points.iter().map(|p| p.failed_requests).sum();

        let success_rate = if total_requests > 0 {
            successful_requests as f64 / total_requests as f64
        } else {
            0.0
        };

        let total_input_tokens: u64 = points.iter().map(|p| p.input_tokens).sum();
        let total_output_tokens: u64 = points.iter().map(|p| p.output_tokens).sum();
        let total_tokens: u64 = points.iter().map(|p| p.total_tokens).sum();
        let total_cost_usd: f64 = points.iter().map(|p| p.cost_usd).sum();

        let total_latency_ms: u64 = points.iter().map(|p| p.total_latency_ms).sum();
        let avg_latency_ms = if total_requests > 0 {
            total_latency_ms as f64 / total_requests as f64
        } else {
            0.0
        };

        // Collect all latency samples
        let mut all_latencies: Vec<u64> = points
            .iter()
            .flat_map(|p| p.latency_samples.clone())
            .collect();
        all_latencies.sort_unstable();

        let median_latency_ms = if !all_latencies.is_empty() {
            let mid = all_latencies.len() / 2;
            all_latencies[mid]
        } else {
            0
        };

        let p95_latency_ms = Self::percentile(&all_latencies, 95.0);
        let p99_latency_ms = Self::percentile(&all_latencies, 99.0);

        Self {
            total_requests,
            successful_requests,
            failed_requests,
            success_rate,
            total_input_tokens,
            total_output_tokens,
            total_tokens,
            total_cost_usd,
            avg_latency_ms,
            median_latency_ms,
            p95_latency_ms,
            p99_latency_ms,
        }
    }

    /// Calculate percentile from sorted samples
    fn percentile(sorted_samples: &[u64], percentile: f64) -> u64 {
        if sorted_samples.is_empty() {
            return 0;
        }

        let index = ((percentile / 100.0) * (sorted_samples.len() as f64 - 1.0)) as usize;
        sorted_samples[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_log_file(
        dir: &PathBuf,
        date: &str,
        entries: Vec<AccessLogEntry>,
    ) -> AppResult<()> {
        let log_path = dir.join(format!("localrouter-{}.log", date));
        let mut file = File::create(log_path)?;

        for entry in entries {
            let json = serde_json::to_string(&entry)?;
            writeln!(file, "{}", json)?;
        }

        Ok(())
    }

    #[test]
    fn test_parse_log_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_dir = temp_dir.path().to_path_buf();

        let entries = vec![
            AccessLogEntry::success("key1", "openai", "gpt-4", 100, 200, 0.05, 1000, "req1"),
            AccessLogEntry::success("key2", "ollama", "llama3.3", 150, 250, 0.0, 500, "req2"),
        ];

        create_test_log_file(&log_dir, "2026-01-14", entries.clone()).unwrap();

        let parser = LogParser::new(log_dir.clone(), 300);
        let log_file = log_dir.join("localrouter-2026-01-14.log");

        let parsed = parser.parse_log_file(&log_file).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].api_key_name, "key1");
        assert_eq!(parsed[1].api_key_name, "key2");
    }

    #[test]
    fn test_filter_entries() {
        let parser = LogParser::new(PathBuf::from("/tmp"), 300);

        let now = Utc::now();
        let entries = vec![
            AccessLogEntry::success("key1", "openai", "gpt-4", 100, 200, 0.05, 1000, "req1"),
            AccessLogEntry::success("key2", "ollama", "llama3.3", 150, 250, 0.0, 500, "req2"),
        ];

        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        // Filter by API key
        let filtered = parser.filter_entries(entries.clone(), start, end, Some("key1"), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].api_key_name, "key1");

        // Filter by provider
        let filtered = parser.filter_entries(entries.clone(), start, end, None, Some("ollama"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].provider, "ollama");
    }

    #[test]
    fn test_aggregate_by_minute() {
        let parser = LogParser::new(PathBuf::from("/tmp"), 300);

        let entries = vec![
            AccessLogEntry::success("key1", "openai", "gpt-4", 100, 200, 0.05, 1000, "req1"),
            AccessLogEntry::success("key1", "openai", "gpt-4", 150, 250, 0.06, 1100, "req2"),
        ];

        let aggregated = parser.aggregate_by_minute(entries);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].requests, 2);
        assert_eq!(aggregated[0].successful_requests, 2);
        assert_eq!(aggregated[0].input_tokens, 250);
        assert_eq!(aggregated[0].output_tokens, 450);
    }

    #[test]
    fn test_log_summary() {
        let points = vec![MetricDataPoint {
            timestamp: Utc::now(),
            requests: 10,
            input_tokens: 1000,
            output_tokens: 2000,
            total_tokens: 3000,
            cost_usd: 0.5,
            total_latency_ms: 10000,
            successful_requests: 9,
            failed_requests: 1,
            latency_samples: vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000],
        }];

        let summary = LogSummary::from_data_points(&points);

        assert_eq!(summary.total_requests, 10);
        assert_eq!(summary.successful_requests, 9);
        assert_eq!(summary.failed_requests, 1);
        assert_eq!(summary.success_rate, 0.9);
        assert_eq!(summary.total_input_tokens, 1000);
        assert_eq!(summary.total_output_tokens, 2000);
        assert_eq!(summary.total_cost_usd, 0.5);
        assert_eq!(summary.avg_latency_ms, 1000.0);
    }

    #[test]
    fn test_cache() {
        let temp_dir = TempDir::new().unwrap();
        let log_dir = temp_dir.path().to_path_buf();

        let parser = LogParser::new(log_dir.clone(), 300);

        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        // First query - should cache
        let _ = parser.query_range(start, end);
        assert_eq!(parser.cache_size(), 1);

        // Second query - should use cache
        let _ = parser.query_range(start, end);
        assert_eq!(parser.cache_size(), 1);

        // Clear cache
        parser.clear_cache();
        assert_eq!(parser.cache_size(), 0);
    }
}
