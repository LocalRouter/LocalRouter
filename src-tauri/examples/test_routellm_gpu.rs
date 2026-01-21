//! Standalone test for RouteLLM GPU performance
//! Run with: cargo run --example test_routellm_gpu --release

use std::path::PathBuf;
use std::time::Instant;

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("================================================");
    println!("RouteLLM GPU Performance Test");
    println!("================================================\n");

    // Set up paths
    let home = std::env::var("HOME").expect("HOME not set");
    let routellm_dir = PathBuf::from(home).join(".localrouter-dev/routellm");
    let model_path = routellm_dir.join("model");
    let tokenizer_path = routellm_dir.join("tokenizer");

    // Check if models exist
    if !model_path.join("model.safetensors").exists() {
        eprintln!("❌ Model not found at {:?}", model_path);
        eprintln!("\nPlease download the model first:");
        eprintln!("cargo test --lib routellm::tests::downloader_tests::test_download_and_verify_models -- --ignored --nocapture");
        std::process::exit(1);
    }

    println!("✓ Loading RouteLLM model...\n");

    // Load the model
    use localrouter_ai::routellm::candle_router::CandleRouter;
    let router = match CandleRouter::new(&model_path, &tokenizer_path) {
        Ok(r) => {
            println!("✓ Model loaded successfully!\n");
            r
        }
        Err(e) => {
            eprintln!("❌ Failed to load model: {:?}", e);
            std::process::exit(1);
        }
    };

    // Test cases
    let test_cases = vec![
        ("Short text", "What is 2+2?"),
        ("Medium text", &"a".repeat(500)),
        ("Long text (user's case)", "2026-01-21T04:22:37.434492Z  INFO localrouter_ai: Initializing rate limiter...\n\
                     2026-01-21T04:22:37.434546Z  INFO localrouter_ai: Initializing metrics collector...\n\
                     2026-01-21T04:22:37.438532Z  INFO localrouter_ai: Initializing RouteLLM service...\n\
                     2026-01-21T04:22:37.438781Z  INFO localrouter_ai: RouteLLM service initialized with idle timeout: 600s\n\
                     2026-01-21T04:22:37.438795Z  INFO localrouter_ai: Initializing router...\n\
                     2026-01-21T04:22:37.438872Z  INFO localrouter_ai: Initializing web server...\n\
                     2026-01-21T04:22:37.439066Z  INFO localrouter_ai::server: Starting web server on 127.0.0.1:33625\n\
                     2026-01-21T04:22:37.439108Z  INFO localrouter_ai::server::state: Generated transient internal test bearer token for UI model testing\n\
                     2026-01-21T04:22:37.439409Z  INFO localrouter_ai::monitoring::logger: Access logger initialized with directory: \"/Users/matus/Library/Logs/LocalRouter\"\n\
                     2026-01-21T04:22:37.439446Z  INFO localrouter_ai::monitoring::mcp_logger: MCP access logger initialized with directory: \"/Users/matus/Library/Logs/LocalRouter\"\n\
                     2026-01-21T04:22:37.439479Z  WARN localrouter_ai::api_keys::keychain_trait: Using file-based keychain storage (DEVELOPMENT MODE)\n\
                     2026-01-21T04:22:37.444840Z  INFO localrouter_ai::server: Web server listening on http://127.0.0.1:33625\n\
                     2026-01-21T04:22:37.444852Z  INFO localrouter_ai::server: OpenAI-compatible endpoints available at:\n\
                     2026-01-21T04:22:37.445062Z  INFO localrouter_ai::server::manager: Server started successfully on port 33625\n\
                     2026-01-21T04:22:37.698310Z  INFO localrouter_ai: Tauri app initialized\n"),
        ("Very long text", &"a".repeat(10_000)),
    ];

    println!("Running performance tests...\n");

    let mut timings = Vec::new();

    for (name, text) in &test_cases {
        let start = Instant::now();

        match router.calculate_strong_win_rate(text) {
            Ok(win_rate) => {
                let duration = start.elapsed();
                timings.push(duration.as_secs_f64());

                println!("{} ({} chars):", name, text.len());
                println!("  Time: {:.3}s", duration.as_secs_f64());
                println!("  Win rate: {:.3}", win_rate);
                println!();
            }
            Err(e) => {
                eprintln!("❌ Failed on {}: {:?}", name, e);
            }
        }
    }

    // Performance summary
    println!("================================================");
    println!("Performance Summary");
    println!("================================================\n");

    if timings.len() >= 2 {
        let short_time = timings[0];
        let long_time = timings[2]; // User's 1500-char case
        let ratio = long_time / short_time;

        println!("Short text time: {:.3}s", short_time);
        println!("Long text time: {:.3}s", long_time);
        println!("Performance ratio: {:.2}x", ratio);
        println!();

        if ratio < 2.0 {
            println!("✅ EXCELLENT! Performance is consistent regardless of text length.");
            println!("   The truncation fix is working!");
        } else if ratio < 5.0 {
            println!("✅ GOOD! Performance scaling is reasonable.");
        } else {
            println!("⚠️  Performance degrades with longer text.");
            println!("   Truncation may not be working correctly.");
        }
    }

    println!("\n================================================");
    println!("Test Complete!");
    println!("================================================");
}
