//! Anthropic Extended Thinking feature adapter
//!
//! Supports Claude's extended thinking capability which allows models to
//! process complex reasoning tasks with dedicated thinking tokens.

use async_trait::async_trait;

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::{CompletionRequest, CompletionResponse};
use lr_types::{AppError, AppResult};

/// Adapter for Anthropic's extended thinking feature
///
/// This feature is available on:
/// - Claude Opus 4.5
/// - Claude Sonnet 4.5
/// - Claude Haiku 4.5
///
/// Extended thinking allows the model to use additional tokens for internal
/// reasoning before generating the final response.
///
/// # Parameters
/// - `thinking_budget`: Number of tokens to allocate for thinking (min: 1024)
///
/// # Example
/// ```json
/// {
///   "extensions": {
///     "anthropic_thinking": {
///       "thinking_budget": 10000
///     }
///   }
/// }
/// ```
pub struct AnthropicThinkingAdapter;

impl AnthropicThinkingAdapter {
    /// Minimum thinking budget in tokens
    const MIN_THINKING_BUDGET: u32 = 1024;

    /// Extract thinking budget from parameters
    fn get_thinking_budget(params: &FeatureParams) -> AppResult<u32> {
        let budget_value = params
            .get("thinking_budget")
            .ok_or_else(|| AppError::Config("Missing 'thinking_budget' parameter".to_string()))?;

        // Try to parse as number
        let budget = if let Some(num) = budget_value.as_u64() {
            num as u32
        } else if let Some(num) = budget_value.as_i64() {
            if num < 0 {
                return Err(AppError::Config(
                    "thinking_budget must be non-negative".to_string(),
                ));
            }
            num as u32
        } else {
            return Err(AppError::Config(
                "thinking_budget must be a number".to_string(),
            ));
        };

        Ok(budget)
    }
}

#[async_trait]
impl FeatureAdapter for AnthropicThinkingAdapter {
    fn feature_name(&self) -> &str {
        "extended_thinking"
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        let thinking_budget = Self::get_thinking_budget(params)?;

        if thinking_budget < Self::MIN_THINKING_BUDGET {
            return Err(AppError::Config(format!(
                "thinking_budget must be at least {} tokens (got {})",
                Self::MIN_THINKING_BUDGET,
                thinking_budget
            )));
        }

        // Store the budget in request extensions; the Anthropic provider
        // reads extensions.anthropic_thinking.thinking_budget and maps it to
        // the Messages API `thinking.budget_tokens` parameter.
        let mut extensions = request.extensions.clone().unwrap_or_default();
        extensions.insert(
            "anthropic_thinking".to_string(),
            serde_json::json!({ "thinking_budget": thinking_budget }),
        );
        request.extensions = Some(extensions);

        tracing::debug!(
            "Extended thinking enabled with budget of {} tokens",
            thinking_budget
        );

        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // The Anthropic provider parses thinking blocks directly into each
        // choice's `reasoning_content`, so no response rewriting is needed.
        // Report whether reasoning was produced as feature data.
        let has_reasoning = response
            .choices
            .iter()
            .any(|c| c.message.reasoning_content.is_some());

        if has_reasoning {
            Ok(Some(FeatureData::new(
                self.feature_name(),
                serde_json::json!({ "reasoning_present": true }),
            )))
        } else {
            Ok(None)
        }
    }

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        let thinking_budget = Self::get_thinking_budget(params)?;

        if thinking_budget < Self::MIN_THINKING_BUDGET {
            return Err(AppError::Config(format!(
                "thinking_budget must be at least {} tokens",
                Self::MIN_THINKING_BUDGET
            )));
        }

        // Reasonable upper limit to prevent excessive costs
        const MAX_THINKING_BUDGET: u32 = 100_000;
        if thinking_budget > MAX_THINKING_BUDGET {
            return Err(AppError::Config(format!(
                "thinking_budget must be at most {} tokens",
                MAX_THINKING_BUDGET
            )));
        }

        Ok(())
    }

    fn cost_multiplier(&self) -> f64 {
        // Thinking tokens are charged at the same rate as input tokens
        // So using extended thinking increases cost proportionally
        1.0 // No extra multiplier beyond the token usage
    }

    fn help_text(&self) -> &str {
        "Extended Thinking: Allows Claude to use dedicated reasoning tokens before generating a response.\n\
         \n\
         Parameters:\n\
         - thinking_budget (required): Number of tokens to allocate for thinking (min: 1024, max: 100000)\n\
         \n\
         Supported models:\n\
         - Claude Opus 4.5\n\
         - Claude Sonnet 4.5\n\
         - Claude Haiku 4.5\n\
         \n\
         Example:\n\
         {\n\
           \"extensions\": {\n\
             \"anthropic_thinking\": {\n\
               \"thinking_budget\": 10000\n\
             }\n\
           }\n\
         }\n\
         \n\
         Note: Thinking tokens are charged at input token rates."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_feature_name() {
        let adapter = AnthropicThinkingAdapter;
        assert_eq!(adapter.feature_name(), "extended_thinking");
    }

    #[test]
    fn test_validate_params_valid() {
        let adapter = AnthropicThinkingAdapter;
        let mut params = HashMap::new();
        params.insert("thinking_budget".to_string(), json!(5000));

        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_too_small() {
        let adapter = AnthropicThinkingAdapter;
        let mut params = HashMap::new();
        params.insert("thinking_budget".to_string(), json!(500));

        assert!(adapter.validate_params(&params).is_err());
    }

    #[test]
    fn test_validate_params_too_large() {
        let adapter = AnthropicThinkingAdapter;
        let mut params = HashMap::new();
        params.insert("thinking_budget".to_string(), json!(200_000));

        assert!(adapter.validate_params(&params).is_err());
    }

    #[test]
    fn test_validate_params_missing() {
        let adapter = AnthropicThinkingAdapter;
        let params = HashMap::new();

        assert!(adapter.validate_params(&params).is_err());
    }

    #[test]
    fn test_cost_multiplier() {
        let adapter = AnthropicThinkingAdapter;
        assert_eq!(adapter.cost_multiplier(), 1.0);
    }

    fn blank_request() -> CompletionRequest {
        CompletionRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![],
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
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
            n: None,
            logit_bias: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
            pre_computed_routing: None,
        }
    }

    #[test]
    fn test_adapt_request_writes_budget_to_extensions() {
        let adapter = AnthropicThinkingAdapter;
        let mut request = blank_request();
        let mut params = HashMap::new();
        params.insert("thinking_budget".to_string(), json!(5000));

        adapter.adapt_request(&mut request, &params).unwrap();

        let ext = request.extensions.expect("extensions populated");
        assert_eq!(
            ext.get("anthropic_thinking")
                .and_then(|cfg| cfg.get("thinking_budget"))
                .and_then(|v| v.as_u64()),
            Some(5000)
        );
    }

    #[test]
    fn test_adapt_request_rejects_small_budget() {
        let adapter = AnthropicThinkingAdapter;
        let mut request = blank_request();
        let mut params = HashMap::new();
        params.insert("thinking_budget".to_string(), json!(500));

        assert!(adapter.adapt_request(&mut request, &params).is_err());
        assert!(request.extensions.is_none());
    }
}
