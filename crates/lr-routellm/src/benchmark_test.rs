//! Benchmark accuracy tests for RouteLLM implementation
//!
//! This module tests our RouteLLM Rust implementation against the official
//! benchmarks from the original RouteLLM Python implementation.
//!
//! The benchmarks use MMLU and GSM8K datasets with pre-computed model correctness
//! (True/False for weak model Mixtral-8x7B and strong model GPT-4).
//!
//! Benchmark data is derived from the RouteLLM project (Apache 2.0 license):
//! https://github.com/lm-sys/RouteLLM
//! See tests/fixtures/LICENSE for attribution details.

// The benchmark data contains win rates from Python that happen to be close to mathematical
// constants (e.g., 0.7854 ≈ π/4). These are actual test values, not approximations of constants.
#![allow(clippy::approx_constant)]

use crate::candle_router::CandleRouter;
use csv::Reader;
use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};

/// Get path to test fixtures directory
fn get_fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Custom deserializer for Python-style booleans (True/False)
fn deserialize_python_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    match s.to_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(serde::de::Error::custom(format!("Invalid boolean: {}", s))),
    }
}

/// Record from MMLU/GSM8K benchmark CSV files
#[derive(Debug, Deserialize)]
struct BenchmarkRecord {
    prompt: String,
    #[serde(
        rename = "mistralai/Mixtral-8x7B-Instruct-v0.1",
        deserialize_with = "deserialize_python_bool"
    )]
    weak_model_correct: bool,
    #[serde(
        rename = "gpt-4-1106-preview",
        deserialize_with = "deserialize_python_bool"
    )]
    strong_model_correct: bool,
}

/// Benchmark evaluation results
#[derive(Debug)]
pub struct BenchmarkResults {
    /// Total number of prompts evaluated
    pub total_prompts: usize,
    /// Number of correct routing decisions
    pub correct_routings: usize,
    /// Routing accuracy (correct_routings / total_prompts)
    pub routing_accuracy: f64,
    /// Number of prompts routed to strong model
    pub strong_model_count: usize,
    /// Number of prompts routed to weak model
    pub weak_model_count: usize,
    /// Actual correctness rate (based on which model was chosen)
    pub actual_correctness: f64,
    /// Optimal correctness (if we had perfect routing)
    pub optimal_correctness: f64,
    /// Weak model baseline correctness
    pub weak_baseline: f64,
    /// Strong model baseline correctness
    pub strong_baseline: f64,
    /// Average win_rate across all prompts
    pub avg_win_rate: f64,
    /// Threshold used for routing
    pub threshold: f32,
}

/// Load benchmark records from a CSV file
fn load_benchmark_csv(csv_path: &Path) -> Result<Vec<BenchmarkRecord>, Box<dyn std::error::Error>> {
    let mut rdr = Reader::from_path(csv_path)?;
    let mut records = Vec::new();

    for result in rdr.deserialize() {
        let record: BenchmarkRecord = result?;
        records.push(record);
    }

    Ok(records)
}

/// Evaluate the router on benchmark data
///
/// # Arguments
/// * `router` - The CandleRouter instance
/// * `records` - Benchmark records with prompts and ground truth
/// * `threshold` - Routing threshold (route to strong if win_rate >= threshold)
///
/// # Returns
/// BenchmarkResults with accuracy metrics
pub fn evaluate_benchmark(
    router: &CandleRouter,
    records: &[BenchmarkRecord],
    threshold: f32,
) -> BenchmarkResults {
    let mut correct_routings = 0;
    let mut strong_model_count = 0;
    let mut weak_model_count = 0;
    let mut actual_correct = 0;
    let mut optimal_correct = 0;
    let mut weak_correct = 0;
    let mut strong_correct = 0;
    let mut total_win_rate: f64 = 0.0;

    for record in records {
        let win_rate = router
            .calculate_strong_win_rate(&record.prompt)
            .expect("Failed to calculate win rate");
        total_win_rate += win_rate as f64;

        let route_to_strong = win_rate >= threshold;

        // Count model selections
        if route_to_strong {
            strong_model_count += 1;
        } else {
            weak_model_count += 1;
        }

        // Calculate baselines
        if record.weak_model_correct {
            weak_correct += 1;
        }
        if record.strong_model_correct {
            strong_correct += 1;
        }

        // Calculate actual correctness based on routing decision
        let routed_model_correct = if route_to_strong {
            record.strong_model_correct
        } else {
            record.weak_model_correct
        };
        if routed_model_correct {
            actual_correct += 1;
        }

        // Calculate optimal routing:
        // - If weak model is correct, routing to weak is optimal
        // - If weak is wrong but strong is correct, routing to strong is optimal
        // - If both are wrong, either choice is equally bad
        let optimal_routing = if record.weak_model_correct {
            // Weak model handles it fine, no need for strong
            !route_to_strong
        } else if record.strong_model_correct {
            // Weak fails, strong succeeds - should route to strong
            route_to_strong
        } else {
            // Both fail - doesn't matter which we choose
            true
        };

        if optimal_routing {
            correct_routings += 1;
        }

        // Calculate optimal correctness (ceiling)
        if record.weak_model_correct || record.strong_model_correct {
            optimal_correct += 1;
        }
    }

    let total = records.len();

    BenchmarkResults {
        total_prompts: total,
        correct_routings,
        routing_accuracy: correct_routings as f64 / total as f64,
        strong_model_count,
        weak_model_count,
        actual_correctness: actual_correct as f64 / total as f64,
        optimal_correctness: optimal_correct as f64 / total as f64,
        weak_baseline: weak_correct as f64 / total as f64,
        strong_baseline: strong_correct as f64 / total as f64,
        avg_win_rate: total_win_rate / total as f64,
        threshold,
    }
}

/// Helper function to get model paths
fn get_model_paths() -> (PathBuf, PathBuf) {
    let home_dir = dirs::home_dir().expect("Could not determine home directory");
    let routellm_dir = home_dir.join(".localrouter-dev/routellm");
    (routellm_dir.join("model"), routellm_dir.join("tokenizer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test against MMLU abstract algebra benchmark (small, fast)
    #[test]
    #[ignore] // Requires models and benchmark data
    fn test_mmlu_abstract_algebra_accuracy() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // Load MMLU abstract algebra benchmark
        let benchmark_path = Path::new(
            "/Users/matus/dev/RouteLLM/routellm/evals/mmlu/responses/mmlu_abstract_algebra.csv",
        );

        if !benchmark_path.exists() {
            println!("Benchmark file not found at {:?}", benchmark_path);
            println!("Skipping test - run from correct directory");
            return;
        }

        let records = load_benchmark_csv(benchmark_path).expect("Failed to load benchmark CSV");

        println!("Loaded {} benchmark records", records.len());

        // Test multiple thresholds
        for threshold in [0.2, 0.3, 0.5, 0.7] {
            let results = evaluate_benchmark(&router, &records, threshold);
            print_results(&results, "MMLU Abstract Algebra");
        }
    }

    /// Test against GSM8K benchmark (math problems)
    /// Uses local fixture data (Apache 2.0 licensed from RouteLLM)
    #[test]
    #[ignore] // Requires models to be downloaded
    fn test_gsm8k_accuracy() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // Load GSM8K benchmark from local fixtures
        let benchmark_path = get_fixtures_path().join("gsm8k_sample.csv");

        if !benchmark_path.exists() {
            panic!(
                "Benchmark fixture not found at {:?}. Run 'cargo test' from the crate directory.",
                benchmark_path
            );
        }

        let records = load_benchmark_csv(&benchmark_path).expect("Failed to load benchmark CSV");

        println!(
            "Loaded {} benchmark records from local fixture",
            records.len()
        );

        // Test with balanced threshold (0.5 - the default)
        let results = evaluate_benchmark(&router, &records, 0.5);
        print_results(&results, "GSM8K");

        // Assert quality metrics meet targets
        let quality_retention = results.actual_correctness / results.strong_baseline;
        assert!(
            quality_retention >= 0.80,
            "Quality retention {:.1}% should be >= 80% at threshold 0.5",
            quality_retention * 100.0
        );

        let cost_savings = results.weak_model_count as f64 / results.total_prompts as f64;
        assert!(
            cost_savings >= 0.25,
            "Cost savings {:.1}% should be >= 25% at threshold 0.5",
            cost_savings * 100.0
        );

        println!("\nAssertions passed:");
        println!(
            "  Quality retention: {:.1}% (>= 80%)",
            quality_retention * 100.0
        );
        println!("  Cost savings: {:.1}% (>= 25%)", cost_savings * 100.0);
    }

    /// Comprehensive threshold sweep to find optimal values
    #[test]
    #[ignore]
    fn test_threshold_calibration_sweep() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // Load GSM8K benchmark from local fixtures
        let benchmark_path = get_fixtures_path().join("gsm8k_sample.csv");

        if !benchmark_path.exists() {
            panic!("Benchmark fixture not found at {:?}", benchmark_path);
        }

        let records = load_benchmark_csv(&benchmark_path).expect("Failed to load benchmark CSV");

        println!("Loaded {} benchmark records\n", records.len());

        // Sweep thresholds from 0.1 to 0.9
        let thresholds: Vec<f32> = (1..=9).map(|i| i as f32 * 0.1).collect();

        println!("=== Threshold Calibration Sweep ===\n");
        println!(
            "{:>10} | {:>10} | {:>10} | {:>10} | {:>12}",
            "Threshold", "Strong %", "Weak %", "Quality %", "Correctness"
        );
        println!("{}", "-".repeat(62));

        for threshold in thresholds {
            let results = evaluate_benchmark(&router, &records, threshold);

            let strong_pct =
                100.0 * results.strong_model_count as f64 / results.total_prompts as f64;
            let weak_pct = 100.0 * results.weak_model_count as f64 / results.total_prompts as f64;
            let quality_pct = 100.0 * results.actual_correctness / results.strong_baseline;

            println!(
                "{:>10.2} | {:>9.1}% | {:>9.1}% | {:>9.1}% | {:>11.2}%",
                threshold,
                strong_pct,
                weak_pct,
                quality_pct,
                results.actual_correctness * 100.0
            );
        }

        println!("\n=== Recommended Thresholds ===\n");
        println!("Use case         | Threshold | Expected Quality | Expected Cost Savings");
        println!("{}", "-".repeat(70));
        println!("Quality First    | 0.30      | ~95%             | ~20-30%");
        println!("Balanced         | 0.50      | ~87%             | ~50-55%");
        println!("Cost Optimized   | 0.70      | ~75%             | ~70-80%");
    }

    /// Test that the default UI threshold (0.5) provides good quality/cost balance
    /// This validates that our recommended default setting works correctly
    #[test]
    #[ignore]
    fn test_default_threshold_quality() {
        const DEFAULT_THRESHOLD: f32 = 0.5; // Must match lr-config default

        let (model_path, tokenizer_path) = get_model_paths();
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        let benchmark_path = get_fixtures_path().join("gsm8k_sample.csv");
        let records = load_benchmark_csv(&benchmark_path).expect("Failed to load benchmark CSV");

        let results = evaluate_benchmark(&router, &records, DEFAULT_THRESHOLD);

        // At threshold 0.5, we expect:
        // - Quality retention >= 85% of strong model
        // - Cost savings >= 30% (weak model usage)
        let quality_retention = results.actual_correctness / results.strong_baseline;
        let cost_savings = results.weak_model_count as f64 / results.total_prompts as f64;

        println!("Default threshold ({}) validation:", DEFAULT_THRESHOLD);
        println!("  Quality retention: {:.1}%", quality_retention * 100.0);
        println!("  Cost savings: {:.1}%", cost_savings * 100.0);

        assert!(
            quality_retention >= 0.85,
            "Default threshold should achieve >= 85% quality retention, got {:.1}%",
            quality_retention * 100.0
        );

        assert!(
            cost_savings >= 0.30,
            "Default threshold should achieve >= 30% cost savings, got {:.1}%",
            cost_savings * 100.0
        );
    }

    /// Full MMLU benchmark across all domains
    #[test]
    #[ignore] // Requires models and benchmark data, takes ~10 minutes
    fn test_mmlu_full_accuracy() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        let mmlu_dir = Path::new("/Users/matus/dev/RouteLLM/routellm/evals/mmlu/responses");

        if !mmlu_dir.exists() {
            println!("MMLU directory not found at {:?}", mmlu_dir);
            return;
        }

        // Load all MMLU domains
        let mut all_records = Vec::new();

        for entry in std::fs::read_dir(mmlu_dir).expect("Failed to read MMLU directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if path.extension().map_or(false, |e| e == "csv") {
                println!("Loading {:?}...", path.file_name().unwrap());
                match load_benchmark_csv(&path) {
                    Ok(records) => all_records.extend(records),
                    Err(e) => println!("  Failed to load: {}", e),
                }
            }
        }

        println!("\nTotal MMLU records loaded: {}", all_records.len());

        // Test with balanced threshold
        let results = evaluate_benchmark(&router, &all_records, 0.5);
        print_results(&results, "MMLU (All Domains)");
    }

    /// Test to analyze win_rate distribution
    #[test]
    #[ignore]
    fn test_win_rate_distribution() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // Test prompts with known expected behavior
        let test_cases = vec![
            ("What is 2+2?", "Simple arithmetic - should be LOW win_rate"),
            ("Hello!", "Simple greeting - should be LOW win_rate"),
            (
                "Explain quantum entanglement and its implications for EPR paradox",
                "Complex physics - should be HIGH win_rate",
            ),
            (
                "Write a proof that there are infinitely many prime numbers",
                "Mathematical proof - should be HIGH win_rate",
            ),
            (
                "Analyze the geopolitical implications of climate change on international relations",
                "Complex analysis - should be HIGH win_rate",
            ),
            (
                "What is the capital of France?",
                "Simple factual - should be LOW win_rate",
            ),
        ];

        println!("\n=== Win Rate Distribution Analysis ===\n");
        println!(
            "{:60} | {:10} | {}",
            "Prompt (truncated)", "Win Rate", "Expectation"
        );
        println!("{}", "-".repeat(100));

        for (prompt, description) in test_cases {
            let win_rate = router
                .calculate_strong_win_rate(prompt)
                .expect("Failed to calculate win rate");

            let truncated = if prompt.len() > 57 {
                format!("{}...", &prompt[..57])
            } else {
                prompt.to_string()
            };

            println!("{:60} | {:10.4} | {}", truncated, win_rate, description);
        }

        println!("\n=== Interpretation ===");
        println!("HIGH win_rate (>0.5) = Router thinks STRONG model is needed");
        println!("LOW win_rate (<0.5) = Router thinks WEAK model is sufficient");
        println!("\nIf results are inverted, the label interpretation may be wrong!");
    }

    /// Compare with Python RouteLLM output for specific prompts
    #[test]
    #[ignore]
    fn test_specific_prompts_vs_python() {
        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // These prompts are from the MMLU benchmark
        // We can compare our win_rates with what Python produces
        let prompts = vec![
            "Find the degree for the given field extension Q(sqrt(2), sqrt(3), sqrt(18)) over Q.\nA. 0\nB. 4\nC. 2\nD. 6\nAnswer:",
            "Let p = (1, 2, 5, 4)(2, 3) in S_5 . Find the index of <p> in S_5.\nA. 8\nB. 2\nC. 24\nD. 120\nAnswer:",
            "Janet's ducks lay 16 eggs per day. She eats three for breakfast every morning and bakes muffins for her friends every day with four. She sells the remainder at the farmers' market daily for $2 per fresh duck egg. How much in dollars does she make every day at the farmers' market?",
        ];

        println!("\n=== Win Rate for Benchmark Prompts ===\n");

        for (i, prompt) in prompts.iter().enumerate() {
            let win_rate = router
                .calculate_strong_win_rate(prompt)
                .expect("Failed to calculate win rate");

            println!("Prompt {}: win_rate = {:.6}", i + 1, win_rate);
            println!("  First 80 chars: {}...", &prompt[..prompt.len().min(80)]);
            println!();
        }

        println!("To verify: Run the same prompts through Python RouteLLM and compare win_rates.");
        println!("If they differ significantly, there may be an implementation mismatch.");
    }

    /// Compare Rust vs Python outputs for identical prompts
    #[test]
    #[ignore]
    fn test_compare_rust_vs_python() {
        use crate::candle_router::CandleRouter;

        let (model_path, tokenizer_path) = get_model_paths();

        println!("Loading router from {:?}", model_path);
        let router =
            CandleRouter::new(&model_path, &tokenizer_path).expect("Failed to load router");

        // Same prompts as Python test
        let prompts = vec![
            ("What is 2+2?", 0.4197),
            ("Hello!", 0.2947),
            ("Explain quantum entanglement and its implications for EPR paradox", 0.3607),
            ("Write a proof that there are infinitely many prime numbers", 0.3947),
            ("Analyze the geopolitical implications of climate change on international relations", 0.1696),
            ("What is the capital of France?", 0.3646),
            ("Find the degree for the given field extension Q(sqrt(2), sqrt(3), sqrt(18)) over Q.\nA. 0\nB. 4\nC. 2\nD. 6\nAnswer:", 0.7854),
        ];

        println!("\n=== Rust vs Python Comparison ===\n");
        println!(
            "{:<60} | {:>10} | {:>10} | {:>10}",
            "Prompt", "Python", "Rust", "Diff"
        );
        println!("{}", "-".repeat(100));

        let mut total_diff: f32 = 0.0;

        for (prompt, python_win_rate) in &prompts {
            let rust_win_rate = router
                .calculate_strong_win_rate(prompt)
                .expect("Failed to calculate win rate");

            let diff = rust_win_rate - *python_win_rate as f32;
            total_diff += diff.abs();

            let truncated = if prompt.len() > 57 {
                format!("{}...", &prompt[..54])
            } else {
                prompt.to_string()
            };

            println!(
                "{:<60} | {:>10.4} | {:>10.4} | {:>+10.4}",
                truncated, python_win_rate, rust_win_rate, diff
            );
        }

        println!();
        println!(
            "Average absolute difference: {:.4}",
            total_diff / prompts.len() as f32
        );
        println!();

        if total_diff / prompts.len() as f32 > 0.1 {
            println!("WARNING: Large difference between Rust and Python!");
            println!("This indicates an implementation issue.");
        } else {
            println!("SUCCESS: Rust implementation matches Python within tolerance.");
        }
    }

    fn print_results(results: &BenchmarkResults, benchmark_name: &str) {
        println!(
            "\n======== {} Results (threshold={:.2}) ========",
            benchmark_name, results.threshold
        );
        println!("Total prompts: {}", results.total_prompts);
        println!(
            "Model distribution: {} strong / {} weak ({:.1}% strong)",
            results.strong_model_count,
            results.weak_model_count,
            100.0 * results.strong_model_count as f64 / results.total_prompts as f64
        );
        println!("Average win_rate: {:.4}", results.avg_win_rate);
        println!();
        println!("Baselines:");
        println!(
            "  Weak model (Mixtral): {:.2}%",
            results.weak_baseline * 100.0
        );
        println!(
            "  Strong model (GPT-4): {:.2}%",
            results.strong_baseline * 100.0
        );
        println!(
            "  Optimal (ceiling):    {:.2}%",
            results.optimal_correctness * 100.0
        );
        println!();
        println!("Router performance:");
        println!(
            "  Actual correctness:   {:.2}%",
            results.actual_correctness * 100.0
        );
        println!(
            "  Routing accuracy:     {:.2}%",
            results.routing_accuracy * 100.0
        );
        println!();

        // Calculate quality retention
        let quality_vs_strong = results.actual_correctness / results.strong_baseline;
        let cost_savings = results.weak_model_count as f64 / results.total_prompts as f64;
        println!(
            "Quality vs strong model: {:.1}% (should be >85%)",
            quality_vs_strong * 100.0
        );
        println!(
            "Cost savings (weak %):   {:.1}% (should be >30%)",
            cost_savings * 100.0
        );
    }
}
