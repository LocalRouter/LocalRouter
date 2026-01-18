//! Prompt Caching feature adapter
//!
//! This adapter enables prompt caching to reduce costs for repeated context.
//! Anthropic charges 90% less for cached tokens (5-minute TTL) and 50% less
//! for cache hits within 1 hour.
//!
//! Supported providers:
//! - Anthropic: Native support via cache_control in messages
//! - OpenRouter: Supports Anthropic's cache_control format
//!
//! Example usage:
//! ```json
//! {
//!   "model": "claude-opus-4-5",
//!   "messages": [
//!     {"role": "system", "content": "Long system prompt..."},
//!     {"role": "user", "content": "Long context..."}
//!   ],
//!   "extensions": {
//!     "prompt_caching": {
//!       "cache_control": {
//!         "type": "ephemeral"
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Cost Savings:
//! - Cache creation: Same cost as regular input tokens
//! - Cache hits (< 5 min): 90% discount (0.1x cost)
//! - Cache hits (< 1 hour): 50% discount (0.5x cost)

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::utils::errors::{AppError, AppResult};
use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::providers::{CompletionRequest, CompletionResponse};

/// Feature adapter for prompt caching
pub struct PromptCachingAdapter;

impl PromptCachingAdapter {
    /// Get cache control configuration from parameters
    fn get_cache_config(params: &FeatureParams) -> AppResult<Value> {
        let cache_control = params
            .get("cache_control")
            .cloned()
            .unwrap_or_else(|| json!({"type": "ephemeral"}));

        // Validate cache_control structure
        if !cache_control.is_object() {
            return Err(AppError::Config(
                "cache_control must be an object".to_string(),
            ));
        }

        let cache_type = cache_control
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("ephemeral");

        if cache_type != "ephemeral" {
            return Err(AppError::Config(format!(
                "Invalid cache_control type: '{}'. Only 'ephemeral' is supported",
                cache_type
            )));
        }

        Ok(cache_control)
    }

    /// Determine cache breakpoint positions
    ///
    /// Strategy:
    /// 1. Cache the system message (usually large and static)
    /// 2. Cache early user messages if they contain substantial context
    /// 3. Don't cache the final user message (varies between requests)
    fn determine_cache_breakpoints(request: &CompletionRequest) -> Vec<usize> {
        let mut breakpoints = Vec::new();
        let message_count = request.messages.len();

        if message_count == 0 {
            return breakpoints;
        }

        // Strategy 1: If there's a system message at the start, cache it
        if request.messages[0].role == "system" {
            breakpoints.push(0);
        }

        // Strategy 2: If there are multiple messages, cache the second-to-last message
        // This allows the conversation history to be cached while the final user message varies
        if message_count >= 3 {
            // Cache up to the second-to-last message
            let cache_point = message_count - 2;
            if !breakpoints.contains(&cache_point) {
                breakpoints.push(cache_point);
            }
        }

        breakpoints
    }


    /// Calculate cache savings percentage
    fn calculate_cache_savings(
        cache_creation_tokens: u32,
        cache_read_tokens: u32,
        regular_input_tokens: u32,
    ) -> f64 {
        let total_input = cache_creation_tokens + cache_read_tokens + regular_input_tokens;
        if total_input == 0 {
            return 0.0;
        }

        // Cache creation: 1x cost (no savings)
        // Cache read: 0.1x cost (90% savings)
        // Regular: 1x cost (no savings)

        let full_cost = total_input as f64;
        let cached_cost = (cache_creation_tokens as f64)
            + (cache_read_tokens as f64 * 0.1)
            + (regular_input_tokens as f64);

        let savings = (full_cost - cached_cost) / full_cost * 100.0;
        savings.clamp(0.0, 100.0)
    }

    /// Extract cache metrics from response extensions
    fn extract_cache_metrics(extensions: &HashMap<String, Value>) -> Option<CacheMetrics> {
        // Look for Anthropic-style usage breakdown
        let usage = extensions.get("usage")?;

        let cache_creation_input_tokens = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let cache_read_input_tokens = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let input_tokens = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        if cache_creation_input_tokens == 0 && cache_read_input_tokens == 0 {
            return None;
        }

        Some(CacheMetrics {
            cache_creation_input_tokens,
            cache_read_input_tokens,
            input_tokens,
        })
    }

    /// Detect if provider supports caching
    fn supports_caching(model: &str) -> bool {
        // Anthropic Claude 3+ models support caching
        model.starts_with("claude-3") ||
        model.starts_with("claude-opus") ||
        model.starts_with("claude-sonnet")
    }
}

#[derive(Debug, Clone)]
struct CacheMetrics {
    cache_creation_input_tokens: u32,
    cache_read_input_tokens: u32,
    input_tokens: u32,
}

impl FeatureAdapter for PromptCachingAdapter {
    fn feature_name(&self) -> &str {
        "prompt_caching"
    }

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        Self::get_cache_config(params)?;
        Ok(())
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        if !Self::supports_caching(&request.model) {
            return Err(AppError::Config(format!(
                "Prompt caching not supported for model: {}",
                request.model
            )));
        }

        let cache_control = Self::get_cache_config(params)?;

        // Determine where to place cache breakpoints
        let breakpoints = Self::determine_cache_breakpoints(request);

        if breakpoints.is_empty() {
            return Err(AppError::Config(
                "No suitable cache breakpoints found. Need at least 1 message.".to_string(),
            ));
        }

        // Store cache configuration in extensions for the provider to use
        let mut extensions = request.extensions.clone().unwrap_or_default();

        extensions.insert(
            "_prompt_caching_breakpoints".to_string(),
            json!(breakpoints),
        );

        extensions.insert(
            "_prompt_caching_control".to_string(),
            cache_control,
        );

        request.extensions = Some(extensions);

        Ok(())
    }

    fn adapt_response(
        &self,
        response: &mut CompletionResponse,
    ) -> AppResult<Option<FeatureData>> {
        // Check if caching was used
        let extensions = match &response.extensions {
            Some(ext) => ext,
            None => return Ok(None),
        };

        let metrics = match Self::extract_cache_metrics(extensions) {
            Some(m) => m,
            None => return Ok(None),
        };

        // Update TokenUsage with cache details
        let cache_savings = Self::calculate_cache_savings(
            metrics.cache_creation_input_tokens,
            metrics.cache_read_input_tokens,
            metrics.input_tokens,
        );

        // Add cache details to TokenUsage
        if response.usage.prompt_tokens_details.is_none() {
            response.usage.prompt_tokens_details = Some(crate::providers::PromptTokensDetails {
                cached_tokens: None,
                cache_creation_tokens: Some(metrics.cache_creation_input_tokens),
                cache_read_tokens: Some(metrics.cache_read_input_tokens),
            });
        }

        // Return feature data with savings information
        Ok(Some(FeatureData::new(
            "prompt_caching",
            json!({
                "cache_creation_input_tokens": metrics.cache_creation_input_tokens,
                "cache_read_input_tokens": metrics.cache_read_input_tokens,
                "input_tokens": metrics.input_tokens,
                "cache_savings_percent": format!("{:.1}%", cache_savings),
                "cache_hit": metrics.cache_read_input_tokens > 0,
            }),
        )))
    }

    fn cost_multiplier(&self) -> f64 {
        // Caching reduces costs, but this is dynamic based on cache hits
        // Return 1.0 as the base multiplier - actual savings are calculated per-response
        1.0
    }

    fn help_text(&self) -> &str {
        "Enable prompt caching to reduce costs for repeated context. \
         Anthropic charges 90% less for cached tokens (5-minute TTL). \
         Automatically determines optimal cache breakpoints. \
         Supported by Anthropic Claude 3+ models."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_name() {
        let adapter = PromptCachingAdapter;
        assert_eq!(adapter.feature_name(), "prompt_caching");
    }

    #[test]
    fn test_validate_params_default() {
        let adapter = PromptCachingAdapter;
        let params = HashMap::new();

        // Should use default ephemeral cache
        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_explicit() {
        let adapter = PromptCachingAdapter;
        let mut params = HashMap::new();
        params.insert(
            "cache_control".to_string(),
            json!({"type": "ephemeral"}),
        );

        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_invalid_type() {
        let adapter = PromptCachingAdapter;
        let mut params = HashMap::new();
        params.insert(
            "cache_control".to_string(),
            json!({"type": "permanent"}),
        );

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Only 'ephemeral' is supported"));
    }

    #[test]
    fn test_determine_cache_breakpoints_system_message() {
        let request = CompletionRequest {
            model: "claude-3-opus".to_string(),
            messages: vec![
                crate::providers::ChatMessage {
                    role: "system".to_string(),
                    content: "You are a helpful assistant.".to_string(),
                },
                crate::providers::ChatMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
        };

        let breakpoints = PromptCachingAdapter::determine_cache_breakpoints(&request);
        assert_eq!(breakpoints, vec![0]);
    }

    #[test]
    fn test_determine_cache_breakpoints_conversation() {
        let request = CompletionRequest {
            model: "claude-3-opus".to_string(),
            messages: vec![
                crate::providers::ChatMessage {
                    role: "system".to_string(),
                    content: "System prompt".to_string(),
                },
                crate::providers::ChatMessage {
                    role: "user".to_string(),
                    content: "Message 1".to_string(),
                },
                crate::providers::ChatMessage {
                    role: "assistant".to_string(),
                    content: "Response 1".to_string(),
                },
                crate::providers::ChatMessage {
                    role: "user".to_string(),
                    content: "Message 2".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
        };

        let breakpoints = PromptCachingAdapter::determine_cache_breakpoints(&request);
        // Should cache system message (0) and second-to-last message (2)
        assert!(breakpoints.contains(&0));
        assert!(breakpoints.contains(&2));
    }

    #[test]
    fn test_calculate_cache_savings_full_cache_hit() {
        // All tokens from cache
        let savings = PromptCachingAdapter::calculate_cache_savings(0, 1000, 0);
        assert!((savings - 90.0).abs() < 0.1); // ~90% savings
    }

    #[test]
    fn test_calculate_cache_savings_cache_creation() {
        // All tokens are cache creation (first time)
        let savings = PromptCachingAdapter::calculate_cache_savings(1000, 0, 0);
        assert!((savings - 0.0).abs() < 0.1); // 0% savings on creation
    }

    #[test]
    fn test_calculate_cache_savings_mixed() {
        // 500 cached, 500 regular
        let savings = PromptCachingAdapter::calculate_cache_savings(0, 500, 500);
        assert!((savings - 45.0).abs() < 1.0); // ~45% savings
    }

    #[test]
    fn test_supports_caching() {
        assert!(PromptCachingAdapter::supports_caching("claude-3-opus"));
        assert!(PromptCachingAdapter::supports_caching("claude-3-sonnet"));
        assert!(PromptCachingAdapter::supports_caching("claude-opus-4-5"));
        assert!(PromptCachingAdapter::supports_caching("claude-sonnet-4-5"));
        assert!(!PromptCachingAdapter::supports_caching("gpt-4"));
        assert!(!PromptCachingAdapter::supports_caching("gemini-pro"));
    }

    #[test]
    fn test_adapt_request_claude() {
        let adapter = PromptCachingAdapter;
        let mut request = CompletionRequest {
            model: "claude-3-opus".to_string(),
            messages: vec![
                crate::providers::ChatMessage {
                    role: "system".to_string(),
                    content: "System prompt".to_string(),
                },
                crate::providers::ChatMessage {
                    role: "user".to_string(),
                    content: "User message".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
        };

        let params = HashMap::new();
        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check extensions were added
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert!(extensions.contains_key("_prompt_caching_breakpoints"));
        assert!(extensions.contains_key("_prompt_caching_control"));
    }

    #[test]
    fn test_adapt_request_unsupported_model() {
        let adapter = PromptCachingAdapter;
        let mut request = CompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                crate::providers::ChatMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
        };

        let params = HashMap::new();
        let result = adapter.adapt_request(&mut request, &params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[test]
    fn test_extract_cache_metrics() {
        let mut extensions = HashMap::new();
        extensions.insert(
            "usage".to_string(),
            json!({
                "cache_creation_input_tokens": 100,
                "cache_read_input_tokens": 500,
                "input_tokens": 50
            }),
        );

        let metrics = PromptCachingAdapter::extract_cache_metrics(&extensions).unwrap();
        assert_eq!(metrics.cache_creation_input_tokens, 100);
        assert_eq!(metrics.cache_read_input_tokens, 500);
        assert_eq!(metrics.input_tokens, 50);
    }

    #[test]
    fn test_adapt_response_with_cache_hit() {
        let adapter = PromptCachingAdapter;

        let mut extensions = HashMap::new();
        extensions.insert(
            "usage".to_string(),
            json!({
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 1000,
                "input_tokens": 100
            }),
        );

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "claude-3-opus".to_string(),
            provider: "anthropic".to_string(),
            choices: vec![],
            usage: crate::providers::TokenUsage {
                prompt_tokens: 1100,
                completion_tokens: 50,
                total_tokens: 1150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: Some(extensions),
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());

        let feature_data = result.unwrap();
        assert!(feature_data.is_some());

        let data = feature_data.unwrap();
        assert_eq!(data.feature, "prompt_caching");
        assert_eq!(data.data["cache_hit"], true);
        assert_eq!(data.data["cache_read_input_tokens"], 1000);

        // Check TokenUsage was updated
        assert!(response.usage.prompt_tokens_details.is_some());
        let details = response.usage.prompt_tokens_details.unwrap();
        assert_eq!(details.cache_read_tokens, Some(1000));
    }

    #[test]
    fn test_adapt_response_no_cache() {
        let adapter = PromptCachingAdapter;

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "claude-3-opus".to_string(),
            provider: "anthropic".to_string(),
            choices: vec![],
            usage: crate::providers::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
