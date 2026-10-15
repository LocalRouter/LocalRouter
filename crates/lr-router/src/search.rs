//! Fixed native ranked retrieval, with no classifier, affinity or fallback.
use crate::{Router, UsageInfo};
use lr_providers::search::{SearchRequest, SearchResponse};
use lr_types::{AppError, AppResult};
struct SearchDispatch<'a> {
    router: &'a Router,
    client: &'a str,
    provider: &'a str,
    model: &'a str,
    strategy: &'a str,
    started: std::time::Instant,
    success: bool,
}
impl Drop for SearchDispatch<'_> {
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
    pub fn authorize_ranked_search(
        &self,
        client_id: &str,
    ) -> AppResult<lr_config::UpstreamToolsPolicy> {
        let (client, strategy) = self.validate_client_and_strategy(client_id)?;
        let cost = self.rate_limiter.has_cost_limits(client_id)
            || strategy
                .rate_limits
                .iter()
                .any(|limit| limit.enabled && limit.limit_type == lr_config::RateLimitType::Cost);
        let mut context = lr_providers::server_tools::ServerToolsContext {
            tools: vec![lr_providers::server_tools::PositionedServerTool {
                position: 0,
                definition: lr_providers::server_tools::ServerToolDefinition::WebSearch {
                    filters: None,
                    search_context_size: None,
                },
            }],
            original_tool_count: 1,
            max_tool_calls: None,
            forced_type: None,
            policy_max_tool_calls: None,
        };
        context.authorize(
            &client.security.upstream_tools,
            &client.security.privacy,
            cost,
            strategy.free_tier_only,
        )?;
        Ok(client.security.upstream_tools)
    }
    pub async fn ranked_search(
        &self,
        client_id: &str,
        mut request: SearchRequest,
    ) -> AppResult<SearchResponse> {
        let policy = self.authorize_ranked_search(client_id)?;
        request.validate()?;
        if request.model.is_none() {
            request.model = policy.default_search_model;
        }
        let model = request.model.as_deref().ok_or_else(|| {
            AppError::InvalidParams(
                "Provide a qualified search model or configure a client search default".into(),
            )
        })?;
        let (provider, model) = Self::parse_model_string(model);
        if provider.is_empty() || provider == "localrouter" || model.is_empty() {
            return Err(AppError::InvalidParams(
                "Ranked search requires an explicit provider/model".into(),
            ));
        }
        let (_, strategy) = self.validate_client_and_strategy(client_id)?;
        if !strategy.is_model_allowed(&provider, &model) {
            return Err(AppError::InvalidParams(
                "Search model is not permitted by the current strategy".into(),
            ));
        }
        self.require_enabled_model(&provider, &model)?;
        let instance = self
            .provider_registry
            .get_provider(&provider)
            .ok_or_else(|| AppError::InvalidParams("Search provider is unavailable".into()))?;
        let protocol = instance.search_protocol().ok_or_else(|| {
            AppError::InvalidParams("Provider has no native ranked Search API".into())
        })?;
        request.model = Some(model.clone());
        request.wire_body(protocol)?;
        self.check_client_rate_limits(client_id).await?;
        self.check_strategy_rate_limits(&strategy, &provider, &model)?;
        // A search has no token meter. Unknown request fees are kept unknown in
        // response/monitor metadata; no cost-limited request reaches this point.
        let usage = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        // Count dispatch even if cancelled or its remote outcome becomes unknown.
        self.rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await?;
        self.authorize_ranked_search(client_id)?;
        let mut dispatch = SearchDispatch {
            router: self,
            client: client_id,
            provider: &provider,
            model: &model,
            strategy: &strategy.id,
            started: std::time::Instant::now(),
            success: false,
        };
        let response = instance.search(request).await;
        dispatch.success = response.is_ok();
        response
    }
}
