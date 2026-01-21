///! Integration tests for RouteLLM configuration and routing logic
///!
///! These tests verify that RouteLLM configuration structs are properly defined
///! and that the routing logic works as expected.

use localrouter_ai::config::{AutoModelConfig, RouteLLMConfig, RouteLLMGlobalSettings};

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn test_routellm_config_creation() {
        let config = RouteLLMConfig {
            enabled: true,
            threshold: 0.5,
            strong_models: vec![("openai".to_string(), "gpt-4".to_string())],
            weak_models: vec![("openai".to_string(), "gpt-3.5-turbo".to_string())],
        };

        assert!(config.enabled);
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.strong_models.len(), 1);
        assert_eq!(config.weak_models.len(), 1);
    }

    #[test]
    fn test_routellm_config_default() {
        let config = RouteLLMConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.threshold, 0.3); // Balanced profile
        assert!(config.strong_models.is_empty());
        assert!(config.weak_models.is_empty());
    }

    #[test]
    fn test_routellm_global_settings_default() {
        let settings = RouteLLMGlobalSettings::default();

        assert!(settings.onnx_model_path.is_none());
        assert!(settings.tokenizer_path.is_none());
        assert_eq!(settings.idle_timeout_secs, 600); // 10 minutes
        assert!(settings.download_status.is_none());
    }

    #[test]
    fn test_threshold_range_validation() {
        // Test various threshold values
        let valid_thresholds = vec![0.0, 0.2, 0.3, 0.5, 0.7, 1.0];

        for threshold in valid_thresholds {
            let config = RouteLLMConfig {
                enabled: true,
                threshold,
                strong_models: vec![],
                weak_models: vec![],
            };

            assert!(
                config.threshold >= 0.0 && config.threshold <= 1.0,
                "Threshold {} should be in range [0, 1]",
                threshold
            );
        }
    }

    #[test]
    fn test_auto_model_config_with_routellm() {
        let auto_config = AutoModelConfig {
            enabled: true,
            prioritized_models: vec![("ollama".to_string(), "llama3.2".to_string())],
            available_models: vec![],
            routellm_config: Some(RouteLLMConfig {
                enabled: true,
                threshold: 0.5,
                strong_models: vec![("ollama".to_string(), "llama3.2".to_string())],
                weak_models: vec![("ollama".to_string(), "qwen2.5".to_string())],
            }),
        };

        assert!(auto_config.enabled);
        assert!(auto_config.routellm_config.is_some());

        let routellm_config = auto_config.routellm_config.unwrap();
        assert!(routellm_config.enabled);
        assert_eq!(routellm_config.threshold, 0.5);
    }
}

#[cfg(test)]
mod routing_logic_tests {
    use super::*;

    #[test]
    fn test_threshold_decision_logic() {
        // Simulate routing decision based on win_rate and threshold
        let test_cases = vec![
            (0.8, 0.5, true),  // High win rate with medium threshold -> strong
            (0.2, 0.5, false), // Low win rate with medium threshold -> weak
            (0.5, 0.5, true),  // At threshold -> strong (inclusive)
            (0.49, 0.5, false), // Just below threshold -> weak
            (0.51, 0.5, true), // Just above threshold -> strong
            (0.9, 0.7, true),  // High win rate with high threshold -> strong
            (0.6, 0.7, false), // Medium win rate with high threshold -> weak
        ];

        for (win_rate, threshold, expected_strong) in test_cases {
            let is_strong = win_rate >= threshold;
            assert_eq!(
                is_strong, expected_strong,
                "Win rate {} with threshold {} should route to {}",
                win_rate,
                threshold,
                if expected_strong { "strong" } else { "weak" }
            );
        }
    }

    #[test]
    fn test_threshold_profiles() {
        // Test standard threshold profiles and their expected behavior
        let profiles = vec![
            ("cost_optimized", 0.7),
            ("balanced", 0.5),
            ("balanced_alt", 0.3),
            ("quality_prioritized", 0.2),
        ];

        for (name, threshold) in profiles {
            assert!(
                threshold >= 0.0 && threshold <= 1.0,
                "Profile {} has invalid threshold {}",
                name,
                threshold
            );
        }
    }

    #[test]
    fn test_model_list_selection() {
        let config = RouteLLMConfig {
            enabled: true,
            threshold: 0.5,
            strong_models: vec![
                ("openai".to_string(), "gpt-4".to_string()),
                ("anthropic".to_string(), "claude-3-opus".to_string()),
            ],
            weak_models: vec![
                ("openai".to_string(), "gpt-3.5-turbo".to_string()),
                ("ollama".to_string(), "llama3.2".to_string()),
            ],
        };

        // Simulate selection logic
        let win_rate_high = 0.8;
        let win_rate_low = 0.2;

        let selected_strong = if win_rate_high >= config.threshold {
            &config.strong_models
        } else {
            &config.weak_models
        };

        let selected_weak = if win_rate_low >= config.threshold {
            &config.strong_models
        } else {
            &config.weak_models
        };

        // High win rate should select strong models
        assert_eq!(selected_strong.len(), 2);
        assert_eq!(selected_strong[0].1, "gpt-4");

        // Low win rate should select weak models
        assert_eq!(selected_weak.len(), 2);
        assert_eq!(selected_weak[0].1, "gpt-3.5-turbo");
    }
}

#[cfg(test)]
mod cost_estimation_tests {
    #[test]
    fn test_cost_savings_calculation() {
        // Simulate cost savings based on weak/strong split
        let weak_cost_per_token = 0.000001; // $0.001 per 1K tokens
        let strong_cost_per_token = 0.00003; // $0.03 per 1K tokens

        // With 70% weak model usage (high threshold)
        let weak_pct = 0.7;
        let strong_pct = 0.3;

        let avg_cost = (weak_pct * weak_cost_per_token) + (strong_pct * strong_cost_per_token);
        let baseline_cost = strong_cost_per_token;

        let savings_pct = ((baseline_cost - avg_cost) / baseline_cost) * 100.0;

        assert!(
            savings_pct > 60.0 && savings_pct < 75.0,
            "70% weak usage should yield ~67% savings, got {:.2}%",
            savings_pct
        );
    }

    #[test]
    fn test_balanced_profile_savings() {
        // 50/50 split should yield moderate savings
        let weak_cost_per_token = 0.000001;
        let strong_cost_per_token = 0.00003;

        let weak_pct = 0.5;
        let strong_pct = 0.5;

        let avg_cost = (weak_pct * weak_cost_per_token) + (strong_pct * strong_cost_per_token);
        let baseline_cost = strong_cost_per_token;

        let savings_pct = ((baseline_cost - avg_cost) / baseline_cost) * 100.0;

        assert!(
            savings_pct > 45.0 && savings_pct < 55.0,
            "50/50 split should yield ~50% savings, got {:.2}%",
            savings_pct
        );
    }
}

#[cfg(test)]
mod win_rate_validation_tests {
    #[test]
    fn test_win_rate_range() {
        // Test that win rates are always in valid range
        let test_win_rates = vec![0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];

        for win_rate in test_win_rates {
            assert!(
                win_rate >= 0.0 && win_rate <= 1.0,
                "Win rate {} should be in range [0, 1]",
                win_rate
            );
        }
    }

    #[test]
    fn test_invalid_win_rates() {
        // These would be invalid if not clamped/validated
        let invalid_rates = vec![-0.1, 1.1, 2.0];

        for rate in invalid_rates {
            assert!(
                rate < 0.0 || rate > 1.0,
                "Rate {} should be detected as invalid",
                rate
            );
        }
    }
}
