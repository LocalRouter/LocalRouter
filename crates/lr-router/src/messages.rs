//! Native Messages operations that are not generation calls.
use lr_providers::{
    messages::{MessagesOperation, MessagesRequest},
    CompletionRequest,
};
use lr_types::{AppError, AppResult};

impl crate::Router {
    /// Resolve a bare native SDK model only when exactly one currently allowed
    /// provider advertises that exact id. Never move bound history by picking
    /// the first provider or by fuzzy matching a model name.
    pub async fn resolve_messages_model(&self, client_id: &str, model: &str) -> AppResult<String> {
        let strategy = if matches!(client_id, "internal-test" | "memory-service") {
            None
        } else {
            Some(self.validate_client_and_strategy(client_id)?.1)
        };
        let mut matches = Vec::new();
        for provider in self.provider_registry.list_providers() {
            let name = &provider.instance_name;
            if self.provider_registry.get_provider(name).is_none()
                || self.provider_registry.is_model_disabled(name, model)
                || strategy
                    .as_ref()
                    .is_some_and(|strategy| !strategy.is_model_allowed(name, model))
            {
                continue;
            }
            let Ok(models) = self
                .provider_registry
                .list_provider_models_cached(name)
                .await
            else {
                continue;
            };
            for entry in models
                .iter()
                .filter(|entry| entry.id.eq_ignore_ascii_case(model))
            {
                if !matches
                    .iter()
                    .any(|(provider, id): &(String, String)| provider == name && id == &entry.id)
                {
                    matches.push((name.clone(), entry.id.clone()));
                }
            }
        }
        if matches.len() != 1 {
            return Err(AppError::InvalidParams("Native Messages model is not unambiguous among current allowed providers; use an explicit provider/model".into()));
        }
        let (provider, model) = matches.pop().expect("exactly one native SDK model match");
        Ok(format!("{provider}/{model}"))
    }

    /// Count tokens only on a verified native transport. The returned number is
    /// a provider token count, not newly generated or billed prompt usage.
    pub async fn count_message_tokens(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<serde_json::Value> {
        let privileged = matches!(client_id, "internal-test" | "memory-service");
        let (provider, model) = Self::parse_model_string(&request.model);
        if provider.is_empty() || request.model == "localrouter/auto" {
            return Err(AppError::InvalidParams(
                "Token counting requires explicit provider/model".into(),
            ));
        }
        if !privileged {
            let (_, strategy) = self.validate_client_and_strategy(client_id)?;
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Unauthorized);
            }
            self.check_client_rate_limits(client_id).await?;
        }
        self.require_enabled_model(&provider, &model)?;
        let instance = self
            .provider_registry
            .get_provider(&provider)
            .ok_or_else(|| {
                AppError::Provider("Native token-count provider is unavailable".into())
            })?;
        if !instance.supports_native_message_count() {
            return Err(AppError::InvalidParams(
                "Selected provider has no verified native token-count transport".into(),
            ));
        }
        instance.validate_gateway_options(&request)?;
        let context = request.native_messages.as_ref().ok_or_else(|| {
            AppError::InvalidParams("Native token counting requires Messages context".into())
        })?;
        let mut native: MessagesRequest = context.prepared(&request)?;
        native.model = model;
        let response = instance
            .native_messages(&native, &context.headers, MessagesOperation::CountTokens)
            .await?;
        if !response.status().is_success() {
            return Err(lr_providers::messages::response_error(response).await);
        }
        let body = lr_providers::messages::read_json_response(response, 64 * 1024).await?;
        if body
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .is_none()
        {
            return Err(AppError::Provider(
                "Native token count requires input_tokens".into(),
            ));
        }
        // Counting is not inference and consumes no billed-token allowance.
        // Finish this request-only accounting even if its caller disconnects.
        if privileged {
            return Ok(body);
        }
        let limiter = self.rate_limiter.clone();
        let client = client_id.to_owned();
        let job = lr_types::spawn_traced(async move {
            limiter
                .record_api_key_usage(
                    &client,
                    &crate::UsageInfo {
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                    },
                )
                .await
        });
        job.await
            .map_err(|e| AppError::Provider(format!("Token-count accounting job failed: {e}")))??;
        Ok(body)
    }
}
