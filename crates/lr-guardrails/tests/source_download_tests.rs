//! Integration tests for guardrail source downloads.
//!
//! These tests require network access and are marked `#[ignore]` by default.
//! Run explicitly with:
//!   cargo test -p lr-guardrails --test source_download_tests -- --ignored --nocapture
//!
//! Uses tarball downloads from GitHub CDN (no API rate limit).
//! Results are cached in `~/.localrouter-dev/guardrails/` so subsequent runs
//! reuse the cache without network access.

use std::path::PathBuf;

use lr_guardrails::source_manager::{GuardrailSourceConfig, SourceManager};

/// Get the shared guardrails cache directory (~/.localrouter-dev/guardrails/).
fn cache_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".localrouter-dev")
        .join("guardrails")
}

fn make_source(
    id: &str,
    url: &str,
    branch: &str,
    data_paths: &[&str],
    source_type: &str,
) -> GuardrailSourceConfig {
    GuardrailSourceConfig {
        id: id.to_string(),
        label: id.to_string(),
        source_type: source_type.to_string(),
        enabled: true,
        url: url.to_string(),
        data_paths: data_paths.iter().map(|s| s.to_string()).collect(),
        branch: branch.to_string(),
        predefined: true,
        confidence_threshold: 0.7,
        model_architecture: None,
        hf_repo_id: None,
        requires_auth: false,
    }
}

/// Helper: load rules from cache or download if not cached.
/// Returns (rule_count, Vec<RawRule>).
async fn get_or_download_rules(
    source: &GuardrailSourceConfig,
) -> (usize, Vec<lr_guardrails::RawRule>) {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).expect("Failed to create cache dir");
    let manager = SourceManager::new(dir.clone());

    // Check if compiled rules already exist in cache
    let compiled_path = dir.join(&source.id).join("compiled_rules.json");
    if compiled_path.exists() {
        let data = std::fs::read_to_string(&compiled_path).unwrap();
        let rules: Vec<lr_guardrails::RawRule> =
            serde_json::from_str(&data).unwrap_or_default();
        if !rules.is_empty() {
            println!(
                "Source '{}': loaded {} rules from cache (no download needed)",
                source.id,
                rules.len()
            );
            return (rules.len(), rules);
        }
    }

    // Not cached — download via tarball
    println!("Source '{}': downloading via tarball...", source.id);
    let rule_count = manager
        .update_source(source)
        .await
        .unwrap_or_else(|e| panic!("Failed to update source '{}': {}", source.id, e));

    // Read back the compiled rules from cache
    let rules: Vec<lr_guardrails::RawRule> = if compiled_path.exists() {
        let data = std::fs::read_to_string(&compiled_path).unwrap();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        vec![]
    };

    // Verify details are accessible
    let details = manager.get_source_details(source);
    assert_eq!(
        details.compiled_rules_count, rule_count,
        "Compiled rules count mismatch for '{}'",
        source.id
    );

    println!(
        "Source '{}': {} rules downloaded from {} raw files",
        source.id,
        rule_count,
        details.raw_files.len()
    );

    if !details.path_errors.is_empty() {
        for err in &details.path_errors {
            println!("  PATH ERROR: {} -> {}", err.path, err.detail);
        }
    }

    (rule_count, rules)
}

// ─────────────────────────────────────────────────────────────────────────────
// Source: Microsoft Presidio
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_presidio_downloads_and_has_pii_patterns() {
    let source = make_source(
        "presidio",
        "https://github.com/microsoft/presidio",
        "main",
        &["presidio-analyzer/presidio_analyzer/predefined_recognizers"],
        "regex",
    );

    let (rule_count, rules) = get_or_download_rules(&source).await;

    // Presidio should produce a meaningful number of PII detection patterns
    assert!(
        rule_count >= 10,
        "Presidio should have at least 10 rules, got {}",
        rule_count
    );

    // Verify all rules compile as valid regex
    for rule in &rules {
        assert!(
            regex::Regex::new(&rule.pattern).is_ok(),
            "Rule '{}' has invalid regex: {}",
            rule.id,
            rule.pattern
        );
    }

    // Verify we find known PII pattern types
    let has_credit_card = rules
        .iter()
        .any(|r| r.name.to_lowercase().contains("credit") || r.pattern.contains("\\d{4}"));
    assert!(
        has_credit_card,
        "Presidio should contain credit card patterns"
    );

    let has_email = rules
        .iter()
        .any(|r| r.name.to_lowercase().contains("email") || r.pattern.contains("@"));
    assert!(has_email, "Presidio should contain email patterns");

    println!("Presidio: {} rules verified", rule_count);
    for rule in rules.iter().take(5) {
        println!(
            "  [{}] {} = {}",
            rule.category,
            rule.name,
            &rule.pattern[..rule.pattern.len().min(80)]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Source: LLM Guard (ProtectAI)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_llm_guard_downloads_and_has_secret_patterns() {
    let source = make_source(
        "llm_guard",
        "https://github.com/protectai/llm-guard",
        "main",
        &["llm_guard/input_scanners", "llm_guard/output_scanners"],
        "regex",
    );

    let (rule_count, rules) = get_or_download_rules(&source).await;

    // LLM Guard has 96+ secret detection plugins with re.compile() patterns
    assert!(
        rule_count >= 20,
        "LLM Guard should have at least 20 rules from secrets plugins, got {}",
        rule_count
    );

    // Verify all rules compile as valid regex
    let mut invalid_count = 0;
    for rule in &rules {
        if regex::Regex::new(&rule.pattern).is_err() {
            println!("  INVALID REGEX in rule '{}': {}", rule.id, rule.pattern);
            invalid_count += 1;
        }
    }
    assert_eq!(
        invalid_count, 0,
        "{} rules have invalid regex patterns",
        invalid_count
    );

    // Should have GitHub token detection (ghp_, gho_, etc.)
    let has_github = rules.iter().any(|r| {
        r.pattern.contains("ghp") || r.pattern.contains("gho") || r.pattern.contains("ghu")
    });
    assert!(
        has_github,
        "LLM Guard should contain GitHub token patterns"
    );

    println!("LLM Guard: {} rules verified", rule_count);
    for rule in rules.iter().take(5) {
        println!(
            "  [{}] {} = {}",
            rule.category,
            rule.name,
            &rule.pattern[..rule.pattern.len().min(80)]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-cutting: all enabled default sources produce valid compilable rules
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_all_enabled_sources_produce_valid_compilable_rules() {
    let sources = vec![
        make_source(
            "presidio",
            "https://github.com/microsoft/presidio",
            "main",
            &["presidio-analyzer/presidio_analyzer/predefined_recognizers"],
            "regex",
        ),
        make_source(
            "llm_guard",
            "https://github.com/protectai/llm-guard",
            "main",
            &["llm_guard/input_scanners", "llm_guard/output_scanners"],
            "regex",
        ),
    ];

    let dir = cache_dir();
    std::fs::create_dir_all(&dir).expect("Failed to create cache dir");
    let manager = SourceManager::new(dir.clone());

    let mut total_rules = 0;
    let mut total_invalid = 0;

    for source in &sources {
        // Check cache first
        let compiled_path = dir.join(&source.id).join("compiled_rules.json");
        let rule_count = if compiled_path.exists() {
            let data = std::fs::read_to_string(&compiled_path).unwrap();
            let rules: Vec<lr_guardrails::RawRule> =
                serde_json::from_str(&data).unwrap_or_default();
            if !rules.is_empty() {
                println!("Source '{}': {} rules from cache", source.id, rules.len());
                for rule in &rules {
                    if regex::Regex::new(&rule.pattern).is_err() {
                        println!(
                            "  INVALID: [{}] {} = {}",
                            source.id, rule.name, rule.pattern
                        );
                        total_invalid += 1;
                    }
                }
                total_rules += rules.len();
                continue;
            }
            0
        } else {
            0
        };

        if rule_count == 0 {
            let count = manager
                .update_source(source)
                .await
                .unwrap_or_else(|e| panic!("Failed to update '{}': {}", source.id, e));

            assert!(
                count > 0,
                "Enabled source '{}' should produce rules",
                source.id
            );

            if compiled_path.exists() {
                let data = std::fs::read_to_string(&compiled_path).unwrap();
                let rules: Vec<lr_guardrails::RawRule> =
                    serde_json::from_str(&data).unwrap_or_default();

                for rule in &rules {
                    if regex::Regex::new(&rule.pattern).is_err() {
                        println!(
                            "  INVALID: [{}] {} = {}",
                            source.id, rule.name, rule.pattern
                        );
                        total_invalid += 1;
                    }
                }
            }

            total_rules += count;
            println!("Source '{}': {} rules OK", source.id, count);
        }
    }

    println!("\nTotal: {} rules, {} invalid", total_rules, total_invalid);
    assert_eq!(
        total_invalid, 0,
        "{} rules across enabled sources have invalid regex patterns",
        total_invalid
    );
}
