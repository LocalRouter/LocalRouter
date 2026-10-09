//! Run the production classifier on a JSONL research corpus, without the app.
//! Usage: cargo run -p lr-routellm --example routing_research -- MODEL TOKENIZER CASES
use lr_routellm::candle_router::CandleRouter;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: routing_research MODEL_DIR TOKENIZER_DIR CASES.jsonl".into());
    }
    let start = Instant::now();
    let router = CandleRouter::new(Path::new(&args[1]), Path::new(&args[2]))?;
    eprintln!(
        "model_load_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    for _ in 0..3 {
        router.calculate_strong_win_rate("Warmup: say hello.")?;
    }
    let input = std::io::BufReader::new(std::fs::File::open(&args[3])?);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    for line in input.lines() {
        let case: Value = serde_json::from_str(&line?)?;
        let prompt = case["state"].as_str().ok_or("missing state")?;
        let start = Instant::now();
        let result = router.calculate_strong_win_rate(prompt);
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        let row = match result {
            Ok(score) => json!({"id": case["id"], "score": score, "latency_ms": latency_ms}),
            Err(error) => {
                json!({"id": case["id"], "error": error.to_string(), "latency_ms": latency_ms})
            }
        };
        writeln!(output, "{row}")?;
        output.flush()?;
    }
    Ok(())
}
