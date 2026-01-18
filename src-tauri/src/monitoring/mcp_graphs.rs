//! MCP graph data generation
//!
//! Generates time-series data for MCP metrics formatted for Chart.js.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::graphs::{Dataset, GraphData};
use super::mcp_metrics::McpMetricDataPoint;

/// MCP metric type to display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpMetricType {
    /// Requests over time
    Requests,
    /// Latency (milliseconds)
    Latency,
    /// Success rate (percentage)
    SuccessRate,
}

impl McpMetricType {
    /// Get the label for this metric type
    pub fn label(&self) -> &str {
        match self {
            McpMetricType::Requests => "Requests",
            McpMetricType::Latency => "Latency (ms)",
            McpMetricType::SuccessRate => "Success Rate (%)",
        }
    }
}

/// MCP graph data generator
pub struct McpGraphGenerator;

impl McpGraphGenerator {
    /// Generate graph data from MCP metric data points
    pub fn generate(
        data_points: &[McpMetricDataPoint],
        metric_type: McpMetricType,
        dataset_label: Option<&str>,
    ) -> GraphData {
        let labels: Vec<String> = data_points
            .iter()
            .map(|p| p.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        let values: Vec<f64> = match metric_type {
            McpMetricType::Requests => data_points.iter().map(|p| p.requests as f64).collect(),
            McpMetricType::Latency => data_points.iter().map(|p| p.avg_latency_ms()).collect(),
            McpMetricType::SuccessRate => data_points
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

    /// Generate multi-dataset graph (e.g., comparing multiple clients or servers)
    pub fn generate_multi(
        data_sets: Vec<(&str, &[McpMetricDataPoint])>,
        metric_type: McpMetricType,
    ) -> GraphData {
        if data_sets.is_empty() {
            return GraphData::new(vec![], vec![]);
        }

        // Get all unique timestamps
        let mut all_timestamps: Vec<chrono::DateTime<chrono::Utc>> = Vec::new();
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
                use chrono::{DateTime, Utc};
                let point_map: HashMap<DateTime<Utc>, &McpMetricDataPoint> =
                    points.iter().map(|p| (p.timestamp, p)).collect();

                // Generate values aligned with all_timestamps
                let values: Vec<f64> = all_timestamps
                    .iter()
                    .map(|ts| {
                        point_map.get(ts).map_or(0.0, |p| match metric_type {
                            McpMetricType::Requests => p.requests as f64,
                            McpMetricType::Latency => p.avg_latency_ms(),
                            McpMetricType::SuccessRate => p.success_rate() * 100.0,
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

    /// Generate method breakdown chart (stacked area)
    ///
    /// Shows a stacked area chart with one dataset per method.
    pub fn generate_method_breakdown(data_points: &[McpMetricDataPoint]) -> GraphData {
        if data_points.is_empty() {
            return GraphData::new(vec![], vec![]);
        }

        let labels: Vec<String> = data_points
            .iter()
            .map(|p| p.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        // Collect all unique method names across all data points
        let mut all_methods: Vec<String> = Vec::new();
        for point in data_points {
            for method in point.method_counts.keys() {
                if !all_methods.contains(method) {
                    all_methods.push(method.clone());
                }
            }
        }
        all_methods.sort();

        // Create a dataset for each method
        let datasets: Vec<Dataset> = all_methods
            .iter()
            .enumerate()
            .map(|(idx, method)| {
                let values: Vec<f64> = data_points
                    .iter()
                    .map(|p| {
                        p.method_counts
                            .get(method)
                            .map(|m| m.count as f64)
                            .unwrap_or(0.0)
                    })
                    .collect();

                let color = Self::get_color(idx);
                Dataset::new(method.as_str(), values)
                    .with_background_color(color.clone())
                    .with_border_color(color)
                    .with_fill(true)
                    .with_tension(0.4)
            })
            .collect();

        GraphData::new(labels, datasets)
    }

    /// Generate latency percentile graph for MCP requests
    pub fn generate_latency_percentiles(data_points: &[McpMetricDataPoint]) -> GraphData {
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

    /// Get a color for a dataset index
    fn get_color(index: usize) -> String {
        const COLORS: &[&str] = &[
            "#2196F3", // Blue
            "#4CAF50", // Green
            "#FF9800", // Orange
            "#F44336", // Red
            "#9C27B0", // Purple
            "#00BCD4", // Cyan
            "#FFEB3B", // Yellow
            "#E91E63", // Pink
            "#3F51B5", // Indigo
            "#8BC34A", // Light Green
        ];
        COLORS[index % COLORS.len()].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use crate::monitoring::mcp_metrics::{McpMetricDataPoint, MethodMetrics};

    #[test]
    fn test_generate_requests_graph() {
        let now = Utc::now();
        let mut point1 = McpMetricDataPoint {
            timestamp: now,
            requests: 10,
            successful_requests: 9,
            failed_requests: 1,
            total_latency_ms: 1000,
            latency_samples: vec![100, 200],
            method_counts: HashMap::new(),
        };
        point1.method_counts.insert(
            "tools/list".to_string(),
            MethodMetrics {
                count: 5,
                successful: 5,
                failed: 0,
                total_latency_ms: 500,
            },
        );

        let mut point2 = McpMetricDataPoint {
            timestamp: now + Duration::minutes(1),
            requests: 15,
            successful_requests: 15,
            failed_requests: 0,
            total_latency_ms: 1500,
            latency_samples: vec![100, 150],
            method_counts: HashMap::new(),
        };
        point2.method_counts.insert(
            "tools/list".to_string(),
            MethodMetrics {
                count: 8,
                successful: 8,
                failed: 0,
                total_latency_ms: 800,
            },
        );

        let data_points = vec![point1, point2];
        let graph = McpGraphGenerator::generate(&data_points, McpMetricType::Requests, None);

        assert_eq!(graph.labels.len(), 2);
        assert_eq!(graph.datasets.len(), 1);
        assert_eq!(graph.datasets[0].data.len(), 2);
        assert_eq!(graph.datasets[0].data[0], 10.0);
        assert_eq!(graph.datasets[0].data[1], 15.0);
    }

    #[test]
    fn test_generate_latency_graph() {
        let now = Utc::now();
        let point1 = McpMetricDataPoint {
            timestamp: now,
            requests: 10,
            successful_requests: 10,
            failed_requests: 0,
            total_latency_ms: 1000,
            latency_samples: vec![100, 200],
            method_counts: HashMap::new(),
        };

        let point2 = McpMetricDataPoint {
            timestamp: now + Duration::minutes(1),
            requests: 20,
            successful_requests: 20,
            failed_requests: 0,
            total_latency_ms: 4000,
            latency_samples: vec![100, 150],
            method_counts: HashMap::new(),
        };

        let data_points = vec![point1, point2];
        let graph = McpGraphGenerator::generate(&data_points, McpMetricType::Latency, None);

        assert_eq!(graph.datasets[0].data[0], 100.0); // 1000 / 10
        assert_eq!(graph.datasets[0].data[1], 200.0); // 4000 / 20
    }

    #[test]
    fn test_generate_success_rate_graph() {
        let now = Utc::now();
        let point1 = McpMetricDataPoint {
            timestamp: now,
            requests: 10,
            successful_requests: 9,
            failed_requests: 1,
            total_latency_ms: 1000,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };

        let point2 = McpMetricDataPoint {
            timestamp: now + Duration::minutes(1),
            requests: 20,
            successful_requests: 20,
            failed_requests: 0,
            total_latency_ms: 2000,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };

        let data_points = vec![point1, point2];
        let graph = McpGraphGenerator::generate(&data_points, McpMetricType::SuccessRate, None);

        assert_eq!(graph.datasets[0].data[0], 90.0); // 9/10 * 100
        assert_eq!(graph.datasets[0].data[1], 100.0); // 20/20 * 100
    }

    #[test]
    fn test_generate_method_breakdown() {
        let now = Utc::now();
        let mut point1 = McpMetricDataPoint {
            timestamp: now,
            requests: 10,
            successful_requests: 10,
            failed_requests: 0,
            total_latency_ms: 1000,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };
        point1.method_counts.insert(
            "tools/list".to_string(),
            MethodMetrics {
                count: 5,
                successful: 5,
                failed: 0,
                total_latency_ms: 500,
            },
        );
        point1.method_counts.insert(
            "resources/read".to_string(),
            MethodMetrics {
                count: 5,
                successful: 5,
                failed: 0,
                total_latency_ms: 500,
            },
        );

        let mut point2 = McpMetricDataPoint {
            timestamp: now + Duration::minutes(1),
            requests: 15,
            successful_requests: 15,
            failed_requests: 0,
            total_latency_ms: 1500,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };
        point2.method_counts.insert(
            "tools/list".to_string(),
            MethodMetrics {
                count: 10,
                successful: 10,
                failed: 0,
                total_latency_ms: 1000,
            },
        );
        point2.method_counts.insert(
            "prompts/get".to_string(),
            MethodMetrics {
                count: 5,
                successful: 5,
                failed: 0,
                total_latency_ms: 500,
            },
        );

        let data_points = vec![point1, point2];
        let graph = McpGraphGenerator::generate_method_breakdown(&data_points);

        // Should have 3 datasets (one per unique method)
        assert_eq!(graph.datasets.len(), 3);
        assert_eq!(graph.labels.len(), 2);

        // Find tools/list dataset
        let tools_list = graph
            .datasets
            .iter()
            .find(|d| d.label == "tools/list")
            .unwrap();
        assert_eq!(tools_list.data[0], 5.0);
        assert_eq!(tools_list.data[1], 10.0);

        // Find resources/read dataset (only in first point)
        let resources_read = graph
            .datasets
            .iter()
            .find(|d| d.label == "resources/read")
            .unwrap();
        assert_eq!(resources_read.data[0], 5.0);
        assert_eq!(resources_read.data[1], 0.0); // Not in second point

        // Find prompts/get dataset (only in second point)
        let prompts_get = graph
            .datasets
            .iter()
            .find(|d| d.label == "prompts/get")
            .unwrap();
        assert_eq!(prompts_get.data[0], 0.0); // Not in first point
        assert_eq!(prompts_get.data[1], 5.0);
    }

    #[test]
    fn test_generate_multi() {
        let now = Utc::now();
        let point1 = McpMetricDataPoint {
            timestamp: now,
            requests: 10,
            successful_requests: 10,
            failed_requests: 0,
            total_latency_ms: 1000,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };

        let point2 = McpMetricDataPoint {
            timestamp: now,
            requests: 20,
            successful_requests: 20,
            failed_requests: 0,
            total_latency_ms: 2000,
            latency_samples: vec![],
            method_counts: HashMap::new(),
        };

        let data_sets = vec![("Client 1", vec![point1].as_slice()), ("Client 2", vec![point2].as_slice())];
        let graph = McpGraphGenerator::generate_multi(data_sets, McpMetricType::Requests);

        assert_eq!(graph.datasets.len(), 2);
        assert_eq!(graph.datasets[0].label, "Client 1");
        assert_eq!(graph.datasets[1].label, "Client 2");
        assert_eq!(graph.datasets[0].data[0], 10.0);
        assert_eq!(graph.datasets[1].data[0], 20.0);
    }
}
