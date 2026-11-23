//! OpenAI reasoning tokens feature adapter
//!
//! OpenAI reasoning models spend "reasoning tokens" on their internal chain
//! of thought. These tokens are:
//! - Not included in the visible output
//! - Counted in `completion_tokens` and billed as output
//! - Reported separately in `usage.completion_tokens_details`
//!
//! This adapter reads the reasoning token count the provider reports in
//! `usage.completion_tokens_details.reasoning_tokens` and exposes it in the
//! response's `reasoning_tokens` extension. Responses that report no count
//! get no extension: whether a model reasons is taken from the usage it
//! reports, not from its name.

use serde_json::json;

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::{CompletionRequest, CompletionResponse};
use lr_types::errors::AppResult;

/// Feature adapter for OpenAI reasoning tokens.
pub struct OpenAIReasoningAdapter;

impl FeatureAdapter for OpenAIReasoningAdapter {
    fn feature_name(&self) -> &str {
        "reasoning_tokens"
    }

    fn validate_params(&self, _params: &FeatureParams) -> AppResult<()> {
        // No parameters: the count comes from the response's usage.
        Ok(())
    }

    fn adapt_request(
        &self,
        _request: &mut CompletionRequest,
        _params: &FeatureParams,
    ) -> AppResult<()> {
        // Reasoning models report the count without being asked.
        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        let Some(reasoning_tokens) = response
            .usage
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
        else {
            return Ok(None);
        };
        let completion_tokens = response.usage.completion_tokens;
        let data = json!({
            "reasoning_tokens": reasoning_tokens,
            "completion_tokens": completion_tokens,
            "visible_completion_tokens": completion_tokens.saturating_sub(reasoning_tokens),
        });
        response
            .extensions
            .get_or_insert_with(Default::default)
            .insert(self.feature_name().to_string(), data.clone());
        Ok(Some(FeatureData::new(self.feature_name(), data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn response_with(
        model: &str,
        completion_tokens_details: Option<crate::CompletionTokensDetails>,
    ) -> CompletionResponse {
        CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: model.to_string(),
            provider: "openai".to_string(),
            choices: vec![],
            usage: crate::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        }
    }

    fn reasoning(tokens: Option<u32>) -> Option<crate::CompletionTokensDetails> {
        Some(crate::CompletionTokensDetails {
            reasoning_tokens: tokens,
            thinking_tokens: None,
            audio_tokens: None,
        })
    }

    #[test]
    fn test_feature_name() {
        let adapter = OpenAIReasoningAdapter;
        assert_eq!(adapter.feature_name(), "reasoning_tokens");
    }

    #[test]
    fn test_validate_params() {
        let adapter = OpenAIReasoningAdapter;
        let params = HashMap::new();
        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_adapt_request_no_changes() {
        let adapter = OpenAIReasoningAdapter;
        let mut request = CompletionRequest {
            model: "o1-preview".to_string(),
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
            native_messages: None,
            server_tools: None,
            documents: None,
        };

        let params = HashMap::new();
        assert!(adapter.adapt_request(&mut request, &params).is_ok());
        // Request should be unchanged
        assert_eq!(request.model, "o1-preview");
    }

    #[test]
    fn reported_reasoning_tokens_are_exposed_whatever_the_model_name() {
        let adapter = OpenAIReasoningAdapter;
        // A name no prefix list would know: the reported usage decides.
        let mut response = response_with("some-new-reasoner", reasoning(Some(30)));

        let data = adapter.adapt_response(&mut response).unwrap().unwrap();
        let expected = json!({
            "reasoning_tokens": 30,
            "completion_tokens": 50,
            "visible_completion_tokens": 20,
        });
        assert_eq!(data.feature, "reasoning_tokens");
        assert_eq!(data.data, expected);
        assert_eq!(
            response.extensions.as_ref().unwrap()["reasoning_tokens"],
            expected
        );
        // Usage itself is untouched.
        assert_eq!(response.usage.completion_tokens, 50);
    }

    #[test]
    fn existing_extensions_are_kept() {
        let adapter = OpenAIReasoningAdapter;
        let mut response = response_with("o3", reasoning(Some(10)));
        response.extensions = Some(HashMap::from([("other".to_string(), json!(1))]));

        adapter.adapt_response(&mut response).unwrap();
        let extensions = response.extensions.unwrap();
        assert_eq!(extensions["other"], json!(1));
        assert_eq!(extensions["reasoning_tokens"]["reasoning_tokens"], 10);
    }

    #[test]
    fn no_reported_count_means_no_extension() {
        let adapter = OpenAIReasoningAdapter;
        for details in [None, reasoning(None)] {
            // Even an o1 model: without a reported count there is nothing to say.
            let mut response = response_with("o1-preview", details);
            assert!(adapter.adapt_response(&mut response).unwrap().is_none());
            assert!(response.extensions.is_none());
        }
    }
}
