//! No document can fall through a text-only provider conversion.
use crate::Router;
use lr_providers::{documents::DocumentProtocol, gateway::GatewayPreferences, CompletionRequest};
use lr_types::{AppError, AppResult};
impl Router {
    pub fn prepare_document_request(
        &self,
        client_id: &str,
        request: &mut CompletionRequest,
    ) -> AppResult<()> {
        if !request.has_file_inputs() {
            return Ok(());
        }
        if lr_providers::replay::tools_disabled(request.extensions.as_ref()) {
            return Err(AppError::InvalidParams("Native document replay requires explicit extracted text; opaque files are not automatically re-executed".into()));
        }
        request.documents = Some(Box::new(
            lr_providers::documents::DocumentContext::from_request(request)?,
        ));
        let (_, strategy) = self.validate_client_and_strategy(client_id)?;
        if strategy.free_tier_only
            || self.rate_limiter.has_cost_limits(client_id)
            || strategy
                .rate_limits
                .iter()
                .any(|limit| limit.enabled && limit.limit_type == lr_config::RateLimitType::Cost)
        {
            return Err(AppError::InvalidParams("Opaque document input has no reliable pre-dispatch cost ceiling; cost-limited/free-only requests require extracted text".into()));
        }
        let (provider, model) = Self::parse_model_string(&request.model);
        if provider.is_empty() || provider == "localrouter" || model.is_empty() {
            return Err(AppError::InvalidParams("Native documents require an explicit provider/model; text-only classifiers cannot inspect opaque files".into()));
        }
        let instance = self
            .provider_registry
            .get_provider(&provider)
            .ok_or_else(|| AppError::InvalidParams("Document provider is unavailable".into()))?;
        let mut native_request = request.clone();
        native_request.model = model;
        lr_providers::documents::native_input(instance.as_ref(), &native_request)?;
        let protocol=instance.document_protocol(&native_request).ok_or_else(||AppError::InvalidParams("Provider/model has no verified native document mapping; supply extracted text instead".into()))?;
        if matches!(
            protocol,
            DocumentProtocol::OpenRouterNative | DocumentProtocol::LlmGatewayGoogle
        ) {
            let controls = GatewayPreferences {
                allow_fallbacks: Some(false),
                ..Default::default()
            }
            .effective(request.extensions.as_ref())?;
            request
                .extensions
                .get_or_insert_with(Default::default)
                .insert(
                    lr_providers::gateway::EXTENSION.into(),
                    serde_json::to_value(controls)?,
                );
        }
        Ok(())
    }
}
