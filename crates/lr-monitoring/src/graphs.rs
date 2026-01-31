//! Graph data generation
//!
//! Generates time-series data formatted for Chart.js and other visualization libraries.

#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::metrics::MetricDataPoint;

/// Time range for graph data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeRange {
    /// Last hour
    Hour,
    /// Last 24 hours
    Day,
    /// Last 7 days
    Week,
    /// Last 30 days
    Month,
}

impl TimeRange {
    /// Get the duration for this time range
    pub fn duration(&self) -> Duration {
        match self {
            TimeRange::Hour => Duration::hours(1),
            TimeRange::Day => Duration::days(1),
            TimeRange::Week => Duration::weeks(1),
            TimeRange::Month => Duration::days(30),
        }
    }

    /// Get start and end timestamps for this range
    pub fn get_range(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        let end = Utc::now();
        let start = end - self.duration();
        (start, end)
    }

    /// Get the appropriate bucket interval in minutes for this time range
    /// This determines how data points are aggregated for display
    pub fn bucket_interval_minutes(&self) -> i64 {
        match self {
            TimeRange::Hour => 5,    // 12 buckets per hour
            TimeRange::Day => 60,    // 24 buckets per day
            TimeRange::Week => 360,  // 28 buckets (6-hour intervals)
            TimeRange::Month => 720, // 60 buckets (12-hour intervals)
        }
    }
}

/// Metric type to display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    /// Tokens over time
    Tokens,
    /// Cost over time (USD)
    Cost,
    /// Requests over time
    Requests,
    /// Latency (milliseconds)
    Latency,
    /// Success rate (percentage)
    SuccessRate,
}

impl MetricType {
    /// Get the label for this metric type
    pub fn label(&self) -> &str {
        match self {
            MetricType::Tokens => "Tokens",
            MetricType::Cost => "Cost (USD)",
            MetricType::Requests => "Requests",
            MetricType::Latency => "Latency (ms)",
            MetricType::SuccessRate => "Success Rate (%)",
        }
    }
}

/// Chart.js compatible dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    /// Dataset label
    pub label: String,

    /// Data points (y-values)
    pub data: Vec<f64>,

    /// Optional background color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,

    /// Optional border color
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,

    /// Fill area under line
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<bool>,

    /// Line tension (0 = straight lines, 1 = curves)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tension: Option<f64>,
}

impl Dataset {
    /// Create a new dataset
    pub fn new(label: impl Into<String>, data: Vec<f64>) -> Self {
        Self {
            label: label.into(),
            data,
            background_color: None,
            border_color: None,
            fill: None,
            tension: None,
        }
    }

    /// Set background color
    pub fn with_background_color(mut self, color: impl Into<String>) -> Self {
        self.background_color = Some(color.into());
        self
    }

    /// Set border color
    pub fn with_border_color(mut self, color: impl Into<String>) -> Self {
        self.border_color = Some(color.into());
        self
    }

    /// Set fill
    pub fn with_fill(mut self, fill: bool) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Set tension
    pub fn with_tension(mut self, tension: f64) -> Self {
        self.tension = Some(tension);
        self
    }
}

/// Rate limit information for displaying reference lines on graphs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitInfo {
    /// Type of rate limit
    pub limit_type: String,
    /// Limit value (max allowed per time window)
    pub value: f64,
    /// Time window in seconds
    pub time_window_seconds: i64,
}

impl RateLimitInfo {
    /// Create a new rate limit info
    pub fn new(limit_type: impl Into<String>, value: f64, time_window_seconds: i64) -> Self {
        Self {
            limit_type: limit_type.into(),
            value,
            time_window_seconds,
        }
    }
}

/// Chart.js compatible graph data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphData {
    /// Labels (x-axis, timestamps)
    pub labels: Vec<String>,

    /// Datasets (y-values)
    pub datasets: Vec<Dataset>,

    /// Rate limits to display as reference lines (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<Vec<RateLimitInfo>>,
}

impl GraphData {
    /// Create a new graph data structure
    pub fn new(labels: Vec<String>, datasets: Vec<Dataset>) -> Self {
        Self {
            labels,
            datasets,
            rate_limits: None,
        }
    }

    /// Create graph data with rate limits
    pub fn with_rate_limits(
        labels: Vec<String>,
        datasets: Vec<Dataset>,
        rate_limits: Vec<RateLimitInfo>,
    ) -> Self {
        Self {
            labels,
            datasets,
            rate_limits: Some(rate_limits),
        }
    }

    /// Add rate limits to existing graph data
    pub fn set_rate_limits(mut self, rate_limits: Vec<RateLimitInfo>) -> Self {
        self.rate_limits = Some(rate_limits);
        self
    }
}

/// Graph data generator
pub struct GraphGenerator;

impl GraphGenerator {
    /// Generate graph data from metric data points
    pub fn generate(
        data_points: &[MetricDataPoint],
        metric_type: MetricType,
        dataset_label: Option<&str>,
    ) -> GraphData {
        let labels: Vec<String> = data_points
            .iter()
            .map(|p| p.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        let values: Vec<f64> = match metric_type {
            MetricType::Tokens => data_points.iter().map(|p| p.total_tokens as f64).collect(),
            MetricType::Cost => data_points.iter().map(|p| p.cost_usd).collect(),
            MetricType::Requests => data_points.iter().map(|p| p.requests as f64).collect(),
            MetricType::Latency => data_points.iter().map(|p| p.avg_latency_ms()).collect(),
            MetricType::SuccessRate => data_points
                .iter()
                .map(|p| p.success_rate() * 100.0)
                .collect(),
        };

        let label = dataset_label.unwrap_or(metric_type.label());
        let dataset = Dataset::new(label, values)
            .with_fill(true)
            .with_tension(0.4);

        GraphData::new(labels, vec![dataset])
    }

    /// Generate multi-dataset graph (e.g., comparing multiple API keys)
    pub fn generate_multi(
        data_sets: Vec<(&str, &[MetricDataPoint])>,
        metric_type: MetricType,
    ) -> GraphData {
        if data_sets.is_empty() {
            return GraphData::new(vec![], vec![]);
        }

        // Get all unique timestamps
        let mut all_timestamps: Vec<DateTime<Utc>> = Vec::new();
        for (_, points) in &data_sets {
            for point in *points {
                if !all_timestamps.contains(&point.timestamp) {
                    all_timestamps.push(point.timestamp);
                }
            }
        }
        all_timestamps.sort();

        let labels: Vec<String> = all_timestamps
            .iter()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        // Create a dataset for each input
        let datasets: Vec<Dataset> = data_sets
            .iter()
            .enumerate()
            .map(|(idx, (label, points))| {
                // Create a map of timestamp to data point
                let point_map: std::collections::HashMap<DateTime<Utc>, &MetricDataPoint> =
                    points.iter().map(|p| (p.timestamp, p)).collect();

                // Generate values aligned with all_timestamps
                let values: Vec<f64> = all_timestamps
                    .iter()
                    .map(|ts| {
                        point_map.get(ts).map_or(0.0, |p| match metric_type {
                            MetricType::Tokens => p.total_tokens as f64,
                            MetricType::Cost => p.cost_usd,
                            MetricType::Requests => p.requests as f64,
                            MetricType::Latency => p.avg_latency_ms(),
                            MetricType::SuccessRate => p.success_rate() * 100.0,
                        })
                    })
                    .collect();

                let color = Self::get_color(idx);
                Dataset::new(*label, values)
                    .with_border_color(color.clone())
                    .with_background_color(format!("{}33", color)) // Add transparency
                    .with_fill(true)
                    .with_tension(0.4)
            })
            .collect();

        GraphData::new(labels, datasets)
    }

    /// Generate latency percentile graph
    pub fn generate_latency_percentiles(data_points: &[MetricDataPoint]) -> GraphData {
        let labels: Vec<String> = data_points
            .iter()
            .map(|p| p.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        let p50_values: Vec<f64> = data_points
            .iter()
            .map(|p| p.latency_percentile(50.0) as f64)
            .collect();
        let p95_values: Vec<f64> = data_points
            .iter()
            .map(|p| p.latency_percentile(95.0) as f64)
            .collect();
        let p99_values: Vec<f64> = data_points
            .iter()
            .map(|p| p.latency_percentile(99.0) as f64)
            .collect();

        let datasets = vec![
            Dataset::new("P50", p50_values)
                .with_border_color("#4CAF50")
                .with_fill(false)
                .with_tension(0.4),
            Dataset::new("P95", p95_values)
                .with_border_color("#FF9800")
                .with_fill(false)
                .with_tension(0.4),
            Dataset::new("P99", p99_values)
                .with_border_color("#F44336")
                .with_fill(false)
                .with_tension(0.4),
        ];

        GraphData::new(labels, datasets)
    }

    /// Generate stacked area chart for token breakdown
    pub fn generate_token_breakdown(data_points: &[MetricDataPoint]) -> GraphData {
        let labels: Vec<String> = data_points
            .iter()
            .map(|p| p.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        let input_values: Vec<f64> = data_points.iter().map(|p| p.input_tokens as f64).collect();
        let output_values: Vec<f64> = data_points.iter().map(|p| p.output_tokens as f64).collect();

        let datasets = vec![
            Dataset::new("Input Tokens", input_values)
                .with_background_color("#2196F3")
                .with_border_color("#2196F3")
                .with_fill(true)
                .with_tension(0.4),
            Dataset::new("Output Tokens", output_values)
                .with_background_color("#4CAF50")
                .with_border_color("#4CAF50")
                .with_fill(true)
                .with_tension(0.4),
        ];

        GraphData::new(labels, datasets)
    }

    /// Get a color for a dataset index
    fn get_color(index: usize) -> String {
        let colors = vec![
            "#2196F3", // Blue
            "#4CAF50", // Green
            "#FF9800", // Orange
            "#F44336", // Red
            "#9C27B0", // Purple
            "#00BCD4", // Cyan
            "#FFEB3B", // Yellow
            "#795548", // Brown
            "#607D8B", // Blue Grey
            "#E91E63", // Pink
        ];

        colors[index % colors.len()].to_string()
    }

    /// Aggregate data points into fixed time buckets
    /// This ensures consistent time intervals on the chart regardless of data density
    pub fn aggregate_into_buckets(
        data_points: &[MetricDataPoint],
        time_range: TimeRange,
    ) -> Vec<MetricDataPoint> {
        let (start, end) = time_range.get_range();
        let interval_minutes = time_range.bucket_interval_minutes();
        let interval_seconds = interval_minutes * 60;

        // Create bucket boundaries
        let start_ts = start.timestamp();
        let end_ts = end.timestamp();

        // Round start to bucket boundary
        let bucket_start = (start_ts / interval_seconds) * interval_seconds;

        // Create a map of bucket start time -> aggregated data
        let mut buckets: std::collections::BTreeMap<i64, MetricDataPoint> =
            std::collections::BTreeMap::new();

        // Initialize all buckets with zero values
        let mut current = bucket_start;
        while current <= end_ts {
            buckets.insert(
                current,
                MetricDataPoint {
                    timestamp: DateTime::from_timestamp(current, 0).unwrap_or(start),
                    requests: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cost_usd: 0.0,
                    total_latency_ms: 0,
                    successful_requests: 0,
                    failed_requests: 0,
                    latency_samples: Vec::new(),
                    p50_latency_ms: None,
                    p95_latency_ms: None,
                    p99_latency_ms: None,
                },
            );
            current += interval_seconds;
        }

        // Aggregate data points into buckets
        for point in data_points {
            let point_ts = point.timestamp.timestamp();
            let bucket_ts = (point_ts / interval_seconds) * interval_seconds;

            if let Some(bucket) = buckets.get_mut(&bucket_ts) {
                bucket.requests += point.requests;
                bucket.input_tokens += point.input_tokens;
                bucket.output_tokens += point.output_tokens;
                bucket.total_tokens += point.total_tokens;
                bucket.cost_usd += point.cost_usd;
                bucket.total_latency_ms += point.total_latency_ms;
                bucket.successful_requests += point.successful_requests;
                bucket.failed_requests += point.failed_requests;
                bucket
                    .latency_samples
                    .extend(point.latency_samples.iter().cloned());
            }
        }

        buckets.into_values().collect()
    }

    /// Generate graph data with proper time bucketing
    pub fn generate_bucketed(
        data_points: &[MetricDataPoint],
        metric_type: MetricType,
        dataset_label: Option<&str>,
        time_range: TimeRange,
    ) -> GraphData {
        let bucketed = Self::aggregate_into_buckets(data_points, time_range);
        Self::generate(&bucketed, metric_type, dataset_label)
    }

    /// Generate multi-dataset graph with proper time bucketing
    pub fn generate_multi_bucketed(
        data_sets: Vec<(&str, &[MetricDataPoint])>,
        metric_type: MetricType,
        time_range: TimeRange,
    ) -> GraphData {
        if data_sets.is_empty() {
            return GraphData::new(vec![], vec![]);
        }

        let (start, end) = time_range.get_range();
        let interval_minutes = time_range.bucket_interval_minutes();
        let interval_seconds = interval_minutes * 60;

        // Create bucket boundaries
        let start_ts = start.timestamp();
        let end_ts = end.timestamp();
        let bucket_start = (start_ts / interval_seconds) * interval_seconds;

        // Generate all bucket timestamps
        let mut bucket_timestamps: Vec<i64> = Vec::new();
        let mut current = bucket_start;
        while current <= end_ts {
            bucket_timestamps.push(current);
            current += interval_seconds;
        }

        // Generate labels from bucket timestamps
        let labels: Vec<String> = bucket_timestamps
            .iter()
            .filter_map(|&ts| DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        // Create datasets
        let datasets: Vec<Dataset> = data_sets
            .iter()
            .enumerate()
            .map(|(idx, (label, points))| {
                // Aggregate this dataset's points into buckets
                let mut bucket_values: std::collections::HashMap<i64, f64> =
                    std::collections::HashMap::new();

                for point in *points {
                    let point_ts = point.timestamp.timestamp();
                    let bucket_ts = (point_ts / interval_seconds) * interval_seconds;

                    let value = match metric_type {
                        MetricType::Tokens => point.total_tokens as f64,
                        MetricType::Cost => point.cost_usd,
                        MetricType::Requests => point.requests as f64,
                        MetricType::Latency => point.avg_latency_ms(),
                        MetricType::SuccessRate => point.success_rate() * 100.0,
                    };

                    *bucket_values.entry(bucket_ts).or_insert(0.0) += value;
                }

                // Generate values array aligned with bucket_timestamps
                let values: Vec<f64> = bucket_timestamps
                    .iter()
                    .map(|ts| bucket_values.get(ts).copied().unwrap_or(0.0))
                    .collect();

                let color = Self::get_color(idx);
                Dataset::new(*label, values)
                    .with_border_color(color.clone())
                    .with_background_color(format!("{}33", color))
                    .with_fill(true)
                    .with_tension(0.4)
            })
            .collect();

        GraphData::new(labels, datasets)
    }

    /// Fill missing time points with zeros
    pub fn fill_gaps(
        data_points: &[MetricDataPoint],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        interval_minutes: i64,
    ) -> Vec<MetricDataPoint> {
        // Validate interval to prevent infinite loops
        if interval_minutes <= 0 {
            panic!(
                "interval_minutes must be positive, got: {}",
                interval_minutes
            );
        }

        if data_points.is_empty() {
            return Vec::new();
        }

        let point_map: std::collections::HashMap<i64, &MetricDataPoint> = data_points
            .iter()
            .map(|p| (p.timestamp.timestamp() / 60, p))
            .collect();

        let mut filled = Vec::new();
        let mut current = start;

        while current <= end {
            let minute_ts = current.timestamp() / 60;

            let point = if let Some(existing) = point_map.get(&minute_ts) {
                (*existing).clone()
            } else {
                MetricDataPoint {
                    timestamp: current,
                    requests: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cost_usd: 0.0,
                    total_latency_ms: 0,
                    successful_requests: 0,
                    failed_requests: 0,
                    latency_samples: Vec::new(),
                    p50_latency_ms: None,
                    p95_latency_ms: None,
                    p99_latency_ms: None,
                }
            };

            filled.push(point);
            current += Duration::minutes(interval_minutes);
        }

        filled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_data_points(count: usize) -> Vec<MetricDataPoint> {
        let now = Utc::now();
        (0..count)
            .map(|i| MetricDataPoint {
                timestamp: now - Duration::minutes((count - i - 1) as i64),
                requests: (i + 1) as u64,
                input_tokens: (i + 1) as u64 * 100,
                output_tokens: (i + 1) as u64 * 200,
                total_tokens: (i + 1) as u64 * 300,
                cost_usd: (i + 1) as f64 * 0.01,
                total_latency_ms: (i + 1) as u64 * 1000,
                successful_requests: (i + 1) as u64,
                failed_requests: 0,
                latency_samples: vec![(i + 1) as u64 * 100],
                p50_latency_ms: None,
                p95_latency_ms: None,
                p99_latency_ms: None,
            })
            .collect()
    }

    #[test]
    fn test_time_range_duration() {
        assert_eq!(TimeRange::Hour.duration(), Duration::hours(1));
        assert_eq!(TimeRange::Day.duration(), Duration::days(1));
        assert_eq!(TimeRange::Week.duration(), Duration::weeks(1));
        assert_eq!(TimeRange::Month.duration(), Duration::days(30));
    }

    #[test]
    fn test_time_range_get_range() {
        let (start, end) = TimeRange::Hour.get_range();
        let diff = end.signed_duration_since(start);
        assert_eq!(diff, Duration::hours(1));
    }

    #[test]
    fn test_metric_type_label() {
        assert_eq!(MetricType::Tokens.label(), "Tokens");
        assert_eq!(MetricType::Cost.label(), "Cost (USD)");
        assert_eq!(MetricType::Requests.label(), "Requests");
        assert_eq!(MetricType::Latency.label(), "Latency (ms)");
        assert_eq!(MetricType::SuccessRate.label(), "Success Rate (%)");
    }

    #[test]
    fn test_dataset_creation() {
        let dataset = Dataset::new("Test", vec![1.0, 2.0, 3.0])
            .with_background_color("#FF0000")
            .with_border_color("#00FF00")
            .with_fill(true)
            .with_tension(0.5);

        assert_eq!(dataset.label, "Test");
        assert_eq!(dataset.data, vec![1.0, 2.0, 3.0]);
        assert_eq!(dataset.background_color, Some("#FF0000".to_string()));
        assert_eq!(dataset.border_color, Some("#00FF00".to_string()));
        assert_eq!(dataset.fill, Some(true));
        assert_eq!(dataset.tension, Some(0.5));
    }

    #[test]
    fn test_generate_tokens() {
        let points = create_test_data_points(5);
        let graph_data = GraphGenerator::generate(&points, MetricType::Tokens, None);

        assert_eq!(graph_data.labels.len(), 5);
        assert_eq!(graph_data.datasets.len(), 1);
        assert_eq!(graph_data.datasets[0].data.len(), 5);
        assert_eq!(graph_data.datasets[0].data[0], 300.0);
        assert_eq!(graph_data.datasets[0].data[4], 1500.0);
    }

    #[test]
    fn test_generate_cost() {
        let points = create_test_data_points(3);
        let graph_data = GraphGenerator::generate(&points, MetricType::Cost, None);

        assert_eq!(graph_data.datasets[0].data.len(), 3);
        assert_eq!(graph_data.datasets[0].data[0], 0.01);
        assert_eq!(graph_data.datasets[0].data[1], 0.02);
        assert_eq!(graph_data.datasets[0].data[2], 0.03);
    }

    #[test]
    fn test_generate_requests() {
        let points = create_test_data_points(3);
        let graph_data = GraphGenerator::generate(&points, MetricType::Requests, None);

        assert_eq!(graph_data.datasets[0].data.len(), 3);
        assert_eq!(graph_data.datasets[0].data[0], 1.0);
        assert_eq!(graph_data.datasets[0].data[1], 2.0);
        assert_eq!(graph_data.datasets[0].data[2], 3.0);
    }

    #[test]
    fn test_generate_multi() {
        let points1 = create_test_data_points(3);
        let points2 = create_test_data_points(3);

        let graph_data = GraphGenerator::generate_multi(
            vec![("Dataset 1", &points1), ("Dataset 2", &points2)],
            MetricType::Tokens,
        );

        assert_eq!(graph_data.datasets.len(), 2);
        assert_eq!(graph_data.datasets[0].label, "Dataset 1");
        assert_eq!(graph_data.datasets[1].label, "Dataset 2");
    }

    #[test]
    fn test_generate_latency_percentiles() {
        let points = create_test_data_points(3);
        let graph_data = GraphGenerator::generate_latency_percentiles(&points);

        assert_eq!(graph_data.datasets.len(), 3);
        assert_eq!(graph_data.datasets[0].label, "P50");
        assert_eq!(graph_data.datasets[1].label, "P95");
        assert_eq!(graph_data.datasets[2].label, "P99");
    }

    #[test]
    fn test_generate_token_breakdown() {
        let points = create_test_data_points(3);
        let graph_data = GraphGenerator::generate_token_breakdown(&points);

        assert_eq!(graph_data.datasets.len(), 2);
        assert_eq!(graph_data.datasets[0].label, "Input Tokens");
        assert_eq!(graph_data.datasets[1].label, "Output Tokens");
    }

    #[test]
    fn test_fill_gaps() {
        let now = Utc::now();
        let points = vec![MetricDataPoint {
            timestamp: now,
            requests: 10,
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 300,
            cost_usd: 0.05,
            total_latency_ms: 1000,
            successful_requests: 10,
            failed_requests: 0,
            latency_samples: vec![100],
            p50_latency_ms: None,
            p95_latency_ms: None,
            p99_latency_ms: None,
        }];

        let start = now - Duration::minutes(2);
        let end = now;

        let filled = GraphGenerator::fill_gaps(&points, start, end, 1);

        // Should have 3 points (0, 1, 2 minutes ago)
        assert_eq!(filled.len(), 3);
        assert_eq!(filled[0].requests, 0); // Gap
        assert_eq!(filled[1].requests, 0); // Gap
        assert_eq!(filled[2].requests, 10); // Actual data
    }

    #[test]
    fn test_graph_data_serialization() {
        let graph_data = GraphData::new(
            vec!["2026-01-14 10:00".to_string()],
            vec![Dataset::new("Test", vec![1.0])],
        );

        let json = serde_json::to_string(&graph_data).unwrap();
        let parsed: GraphData = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.labels.len(), 1);
        assert_eq!(parsed.datasets.len(), 1);
    }
}
