//! Current client authorization is checked before routing and again at dispatch.
use crate::Router;
use lr_providers::{gateway::GatewayPreferences, session::SessionOptions, CompletionRequest};
use lr_types::{AppError, AppResult};
impl Router {
    pub fn prepare_upstream_tools(
        &self,
        client_id: &str,
        request: &mut CompletionRequest,
    ) -> AppResult<()> {
        let Some(context) = request.server_tools.as_mut() else {
            return Ok(());
        };
        let (client, strategy) = self.validate_client_and_strategy(client_id)?;
        let cost_ceiling = self.rate_limiter.has_cost_limits(client_id)
            || strategy
                .rate_limits
                .iter()
                .any(|limit| limit.enabled && limit.limit_type == lr_config::RateLimitType::Cost);
        context.authorize(
            &client.security.upstream_tools,
            &client.security.privacy,
            cost_ceiling,
            strategy.free_tier_only,
        )?;
        if request.native_messages.is_some() {
            return Err(AppError::InvalidParams(
                "Server tools have no native Messages mapping".into(),
            ));
        }
        if SessionOptions::from_request(request)?.is_some_and(|options| options.require_continuity)
        {
            return Err(AppError::InvalidParams(
                "Provider-executed tools do not support bound session continuity".into(),
            ));
        }
        let controls = GatewayPreferences {
            allow_fallbacks: Some(false),
            ..Default::default()
        }
        .effective(request.extensions.as_ref())?;
        let extensions = request.extensions.get_or_insert_with(Default::default);
        extensions.remove(lr_providers::session::CACHE_KEY);
        extensions.insert(
            lr_providers::gateway::EXTENSION.into(),
            serde_json::to_value(controls)?,
        );
        Ok(())
    }
    pub(crate) fn check_upstream_tools_provider(
        &self,
        provider: &str,
        request: &CompletionRequest,
    ) -> AppResult<()> {
        let Some(context) = &request.server_tools else {
            return Ok(());
        };
        let protocol = self
            .provider_registry
            .get_provider(provider)
            .and_then(|provider| provider.server_tools_protocol(request))
            .ok_or_else(|| {
                AppError::InvalidParams(
                    "Provider/endpoint does not support these server tools".into(),
                )
            })?;
        // Reuse the wire validator, including cap support and projection checks,
        // before the adapter can perform any network IO.
        let mut body = serde_json::json!({"tools":request.tools});
        context.wire_body(protocol, &mut body, None)
    }
}
