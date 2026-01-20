//! Anthropic Extended Thinking feature adapter
//!
//! Supports Claude's extended thinking capability which allows models to
//! process complex reasoning tasks with dedicated thinking tokens.

use async_trait::async_trait;

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::providers::{CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

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
        _request: &mut CompletionRequest,
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

        // Store thinking budget in request metadata
        // The Anthropic provider will read this and add it to the API request
        // Note: We use a simple approach where we could extend CompletionRequest
        // to have a metadata field, but for now we'll rely on the provider
        // to check the extensions field in the original ChatCompletionRequest

        // Log for debugging
        tracing::debug!(
            "Extended thinking enabled with budget of {} tokens",
            thinking_budget
        );

        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Extract thinking blocks from response if present
        // In Anthropic's API, thinking content comes as separate content blocks
        // with type "thinking"

        // For now, we return None as we need to integrate with the actual
        // Anthropic response structure. This will be implemented when we
        // update the Anthropic provider to parse thinking blocks.

        let _ = response; // Suppress unused warning

        // TODO: Parse thinking blocks from Anthropic response
        // The response structure will be:
        // {
        //   "content": [
        //     { "type": "thinking", "thinking": "..." },
        //     { "type": "text", "text": "..." }
        //   ]
        // }

        Ok(None)
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
}
