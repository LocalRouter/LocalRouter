//! Explicit default-off native operations with no inferred text-token bill.
use crate::{Router, UsageInfo};
use lr_config::{NativeOperation, Strategy};
use lr_types::{AppError, AppResult};

pub(crate) struct NativeDispatch<'a> {
    router: &'a Router,
    client: &'a str,
    provider: &'a str,
    model: &'a str,
    strategy: &'a str,
    started: std::time::Instant,
    pub success: bool,
}
impl<'a> NativeDispatch<'a> {
    pub fn new(
        router: &'a Router,
        client: &'a str,
        provider: &'a str,
        model: &'a str,
        strategy: &'a str,
    ) -> Self {
        Self {
            router,
            client,
            provider,
            model,
            strategy,
            started: std::time::Instant::now(),
            success: false,
        }
    }
}
impl Drop for NativeDispatch<'_> {
    fn drop(&mut self) {
        let latency = self.started.elapsed().as_millis() as u64;
        if self.success {
            self.router
                .metrics_collector
                .record_success(&lr_monitoring::metrics::RequestMetrics {
                    api_key_name: self.client,
                    provider: self.provider,
                    model: self.model,
                    strategy_id: self.strategy,
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                    latency_ms: latency,
                });
        } else {
            self.router.metrics_collector.record_failure(
                self.client,
                self.provider,
                self.model,
                self.strategy,
                latency,
            );
        }
    }
}
impl Router {
    pub fn authorize_native_operation(
        &self,
        client_id: &str,
        operation: NativeOperation,
    ) -> AppResult<Strategy> {
        let (client, strategy) = self.validate_client_and_strategy(client_id)?;
        if !client.security.native_operations.contains(&operation) {
            return Err(AppError::InvalidParams(
                "This native operation is not enabled for the client".into(),
            ));
        }
        if client.security.privacy.restricted() {
            return Err(AppError::InvalidParams(
                "This native operation has no verified local-only/no-retention transport".into(),
            ));
        }
        if strategy.free_tier_only
            || self.rate_limiter.has_cost_limits(client_id)
            || strategy
                .rate_limits
                .iter()
                .any(|limit| limit.enabled && limit.limit_type == lr_config::RateLimitType::Cost)
        {
            return Err(AppError::InvalidParams("Native operation charges cannot be reliably bounded before dispatch; cost-limited/free-only requests are unsupported".into()));
        }
        Ok(strategy)
    }
    pub async fn ocr(
        &self,
        client_id: &str,
        mut request: lr_providers::ocr::OcrRequest,
    ) -> AppResult<lr_providers::ocr::OcrResponse> {
        let strategy = self.authorize_native_operation(client_id, NativeOperation::Ocr)?;
        request.validate()?;
        let (provider, model) = Self::parse_model_string(&request.model);
        if provider.is_empty() || provider == "localrouter" || model.is_empty() {
            return Err(AppError::InvalidParams(
                "OCR requires an explicit qualified provider/model".into(),
            ));
        }
        if !strategy.is_model_allowed(&provider, &model) {
            return Err(AppError::InvalidParams(
                "OCR model is not permitted by the current strategy".into(),
            ));
        }
        self.require_enabled_model(&provider, &model)?;
        let instance = self
            .provider_registry
            .get_provider(&provider)
            .ok_or_else(|| AppError::InvalidParams("OCR provider is unavailable".into()))?;
        let protocol = instance.ocr_protocol().ok_or_else(|| {
            AppError::InvalidParams("Provider has no verified native OCR mapping".into())
        })?;
        request.model = model.clone();
        request.wire_body(protocol)?;
        self.check_client_rate_limits(client_id).await?;
        self.check_strategy_rate_limits(&strategy, &provider, &model)?;
        self.rate_limiter
            .record_api_key_usage(
                client_id,
                &UsageInfo {
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                },
            )
            .await?;
        self.authorize_native_operation(client_id, NativeOperation::Ocr)?;
        let mut dispatch = NativeDispatch::new(self, client_id, &provider, &model, &strategy.id);
        let result = instance.ocr(request).await;
        dispatch.success = result.is_ok();
        result
    }
}
