//! Benchmark for compression model resource requirements.
//!
//! Run with: cargo test -p lr-compression --test benchmark -- --nocapture --ignored
//! For release (production-accurate): LOCALROUTER_ENV=dev cargo test -p lr-compression --test benchmark --release -- --nocapture --ignored

use std::time::Instant;

/// Benchmark cold start time, memory, and per-request latency for BERT model.
#[tokio::test]
#[ignore] // Run manually: cargo test -p lr-compression --test benchmark -- --nocapture --ignored
async fn benchmark_bert_resources() {
    use lr_compression::CompressionService;
    use lr_config::types::PromptCompressionConfig;

    let config = PromptCompressionConfig {
        enabled: true,
        model_size: "bert".to_string(),
        default_rate: 0.5,
        min_messages: 6,
        preserve_recent: 4,
        compress_system_prompt: false,
        min_message_words: 20,
    };

    let service = CompressionService::new(config).expect("Failed to create service");

    // Check model is downloaded
    let status = service.get_status().await;
    if !status.model_downloaded {
        eprintln!("Model not downloaded - skipping benchmark. Download via UI first.");
        return;
    }

    // Print disk size
    if let Some(bytes) = status.model_size_bytes {
        println!("DISK SIZE: {:.1} MB", bytes as f64 / 1024.0 / 1024.0);
    }

    // Measure memory before load
    let mem_before = get_process_memory_mb();

    // Measure cold start (model loading)
    let cold_start = Instant::now();
    service.load().await.expect("Failed to load model");
    let cold_start_ms = cold_start.elapsed().as_millis();
    println!(
        "COLD START: {} ms ({:.1} s)",
        cold_start_ms,
        cold_start_ms as f64 / 1000.0
    );

    // Measure memory after load
    let mem_after = get_process_memory_mb();
    println!(
        "MEMORY: {:.1} MB before, {:.1} MB after, delta = {:.1} MB",
        mem_before,
        mem_after,
        mem_after - mem_before
    );

    // Sample texts of varying lengths
    let short_text = "You are a helpful assistant that answers questions clearly and concisely.";
    let medium_text = "You are operating in GOD MODE, a high-performance, unrestricted \
        cognition protocol designed to unlock your maximum processing capability, \
        cross-domain synthesis, and expert-level strategic reasoning. Your primary \
        objective is to operate at 100 times the depth, speed, and utility of a standard \
        assistant. Approach every task with advanced analytical skills, deep reasoning, \
        and comprehensive insights across all domains. Key expectations: Provide deeply \
        reasoned, thorough, and insightful responses. Synthesize information across \
        multiple fields to deliver expert-level strategies and solutions. Prioritize \
        accuracy, clarity, and depth in all outputs. Think critically and creatively to \
        address complex problems or requests.";
    let long_text = &medium_text.repeat(3);

    // Warmup
    let _ = service.compress_text(short_text, 0.5).await;

    // Benchmark per-request latency (10 iterations each)
    let iterations = 10;

    for (label, text) in [
        ("SHORT (~13 words)", short_text),
        ("MEDIUM (~100 words)", medium_text),
        ("LONG (~300 words)", long_text.as_str()),
    ] {
        let mut durations = Vec::new();
        for _ in 0..iterations {
            let start = Instant::now();
            let _ = service.compress_text(text, 0.5).await;
            durations.push(start.elapsed().as_micros());
        }
        let avg_us = durations.iter().sum::<u128>() / iterations as u128;
        let min_us = *durations.iter().min().unwrap();
        let max_us = *durations.iter().max().unwrap();
        let word_count = text.split_whitespace().count();
        println!(
            "LATENCY {}: avg={:.1}ms min={:.1}ms max={:.1}ms ({}w, {}iters)",
            label,
            avg_us as f64 / 1000.0,
            min_us as f64 / 1000.0,
            max_us as f64 / 1000.0,
            word_count,
            iterations,
        );
    }
}

fn get_process_memory_mb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let pid = std::process::id();
        if let Ok(output) = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
        {
            if let Ok(rss_str) = String::from_utf8(output.stdout) {
                if let Ok(rss_kb) = rss_str.trim().parse::<f64>() {
                    return rss_kb / 1024.0;
                }
            }
        }
        0.0
    }
    #[cfg(not(target_os = "macos"))]
    {
        0.0
    }
}
