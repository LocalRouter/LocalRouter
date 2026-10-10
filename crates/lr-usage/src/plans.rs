//! Plan names and list prices, for value estimates.
//!
//! Prices are US list prices per month (monthly billing). They are estimates:
//! users can override plan and price per account in Settings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanInfo {
    /// Display name, e.g. "Max 20x".
    pub label: String,
    pub monthly_price_usd: Option<f64>,
}

fn plan(label: &str, price: Option<f64>) -> PlanInfo {
    PlanInfo {
        label: label.to_string(),
        monthly_price_usd: price,
    }
}

/// Plan for a provider family's raw plan id (from a usage endpoint, a token
/// claim or a CLI login).
pub fn plan_info(provider: &str, raw: &str) -> Option<PlanInfo> {
    let raw = raw.trim().to_ascii_lowercase();
    match provider {
        "anthropic" => Some(if raw.contains("max_20x") || raw.contains("max20") {
            plan("Max 20x", Some(200.0))
        } else if raw.contains("max_5x") || raw.contains("max5") {
            plan("Max 5x", Some(100.0))
        } else if raw.contains("max") {
            plan("Max", None)
        } else if raw.contains("pro") {
            plan("Pro", Some(20.0))
        } else if raw.contains("team") {
            plan("Team", Some(30.0))
        } else if raw.contains("enterprise") {
            plan("Enterprise", None)
        } else {
            return None;
        }),
        "openai" => Some(match raw.as_str() {
            "free" => plan("Free", Some(0.0)),
            "go" => plan("Go", Some(8.0)),
            "plus" => plan("Plus", Some(20.0)),
            "prolite" => plan("Pro Lite", Some(100.0)),
            "pro" | "promax" => plan("Pro", Some(200.0)),
            "team" => plan("Team", Some(30.0)),
            "business" | "self_serve_business_prolite" | "self_serve_business_usage_based" => {
                plan("Business", Some(30.0))
            }
            "enterprise" => plan("Enterprise", None),
            "edu" => plan("Edu", None),
            _ => return None,
        }),
        "github-copilot" => Some(match raw.as_str() {
            "free" | "free_limited_copilot" => plan("Free", Some(0.0)),
            "individual" | "pro" => plan("Pro", Some(10.0)),
            "individual_pro" | "pro_plus" | "pro+" => plan("Pro+", Some(39.0)),
            "business" => plan("Business", Some(19.0)),
            "enterprise" => plan("Enterprise", Some(39.0)),
            _ => return None,
        }),
        _ => None,
    }
}

/// Display name of a provider family.
pub fn provider_label(provider: &str) -> String {
    match provider {
        "anthropic" => "Anthropic".to_string(),
        "openai" => "OpenAI".to_string(),
        "github-copilot" => "GitHub Copilot".to_string(),
        "openrouter" => "OpenRouter".to_string(),
        "groq" => "Groq".to_string(),
        "cerebras" => "Cerebras".to_string(),
        "mistral" => "Mistral".to_string(),
        "xai" => "xAI".to_string(),
        "togetherai" => "Together AI".to_string(),
        "deepinfra" => "DeepInfra".to_string(),
        "gemini" => "Gemini".to_string(),
        "perplexity" => "Perplexity".to_string(),
        "cohere" => "Cohere".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "moonshot" => "Moonshot".to_string(),
        other => other.to_string(),
    }
}

/// Product name of a provider's subscription.
pub fn subscription_label(provider: &str) -> String {
    match provider {
        "anthropic" => "Claude".to_string(),
        "openai" => "ChatGPT".to_string(),
        "github-copilot" => "GitHub Copilot".to_string(),
        other => provider_label(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_tiers() {
        assert_eq!(
            plan_info("anthropic", "default_claude_max_20x")
                .unwrap()
                .monthly_price_usd,
            Some(200.0)
        );
        assert_eq!(
            plan_info("anthropic", "default_claude_max_5x")
                .unwrap()
                .label,
            "Max 5x"
        );
        assert_eq!(
            plan_info("anthropic", "max").unwrap().monthly_price_usd,
            None
        );
        assert_eq!(plan_info("anthropic", "claude_pro").unwrap().label, "Pro");
        assert!(plan_info("anthropic", "unknown").is_none());
    }

    #[test]
    fn chatgpt_and_copilot_tiers() {
        assert_eq!(
            plan_info("openai", "PRO").unwrap().monthly_price_usd,
            Some(200.0)
        );
        assert_eq!(plan_info("openai", "plus").unwrap().label, "Plus");
        assert_eq!(
            plan_info("github-copilot", "individual").unwrap().label,
            "Pro"
        );
        assert!(plan_info("groq", "pro").is_none());
    }
}
