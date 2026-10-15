//! Client privacy narrows routing; evidence never grants model authorization.
use crate::Router;
use lr_config::ClientPrivacyConfig;
use lr_providers::{
    gateway::{DataCollection, GatewayPreferences},
    privacy::INTERNAL_EXTENSION,
    CompletionRequest, EndpointType,
};
use lr_types::{AppError, AppResult};
impl Router {
    pub fn client_privacy(&self, client_id: &str) -> ClientPrivacyConfig {
        self.config_manager.client_privacy(client_id)
    }
    pub fn prepare_privacy_request(
        &self,
        client_id: &str,
        request: &mut CompletionRequest,
    ) -> AppResult<()> {
        let policy = self.client_privacy(client_id);
        let extensions = request.extensions.get_or_insert_with(Default::default);
        // Private derived state is always replaced by the current configured
        // client policy. Caller-supplied values cannot relax it.
        extensions.remove(INTERNAL_EXTENSION);
        if policy.restricted() {
            extensions.insert(
                INTERNAL_EXTENSION.into(),
                serde_json::to_value(policy)
                    .map_err(|_| AppError::InvalidParams("Invalid privacy policy".into()))?,
            );
        }
        if extensions.is_empty() {
            request.extensions = None;
        }
        Ok(())
    }
    pub fn check_provider_privacy(
        &self,
        client_id: &str,
        provider: &str,
        controls_supported: bool,
    ) -> AppResult<lr_providers::privacy::ProviderPrivacyAssessment> {
        let policy = self.client_privacy(client_id);
        let assessment = self.provider_registry.privacy_assessment(provider, &policy);
        if !assessment.allowed {
            return Err(AppError::InvalidParams(
                assessment
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Provider does not meet client privacy policy".into()),
            ));
        }
        if assessment.requires_gateway_zdr && !controls_supported {
            return Err(AppError::InvalidParams(
                "This endpoint has no verified gateway ZDR mapping".into(),
            ));
        }
        Ok(assessment)
    }
    pub(crate) fn privacy_provider_request(
        &self,
        client_id: &str,
        provider: &str,
        mut request: CompletionRequest,
        endpoint: EndpointType,
    ) -> AppResult<CompletionRequest> {
        let controls_supported = matches!(endpoint, EndpointType::Chat | EndpointType::Messages);
        let assessment = self.check_provider_privacy(client_id, provider, controls_supported)?;
        self.prepare_privacy_request(client_id, &mut request)?;
        if assessment.requires_gateway_zdr {
            let required = GatewayPreferences {
                zdr: Some(true),
                data_collection: Some(DataCollection::Deny),
                ..Default::default()
            };
            let effective = required.effective(request.extensions.as_ref())?;
            request
                .extensions
                .get_or_insert_with(Default::default)
                .insert(
                    lr_providers::gateway::EXTENSION.into(),
                    serde_json::to_value(effective).map_err(|_| {
                        AppError::InvalidParams("Invalid privacy routing controls".into())
                    })?,
                );
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Arc};
    #[tokio::test]
    async fn strict_gateway_policy_survives_caller_controls_and_never_leaks_internal_state() {
        let mut config = lr_config::AppConfig::default();
        let strategy = lr_config::Strategy::new("privacy".into());
        let mut client =
            lr_config::Client::new_with_strategy("privacy".into(), strategy.id.clone());
        config.strategies.push(strategy);
        client.id = "privacy-client".into();
        client.security.privacy.no_retention = true;
        config.clients.push(client);
        let manager = Arc::new(lr_config::ConfigManager::new(
            config,
            std::env::temp_dir().join(format!("privacy-config-{}.yaml", uuid::Uuid::new_v4())),
        ));
        let registry = Arc::new(lr_providers::registry::ProviderRegistry::new());
        registry.register_factory(Arc::new(lr_providers::factory::OpenRouterProviderFactory));
        let mut params = HashMap::from([("api_key".into(), "fixture-credential".into())]);
        let sealed = lr_providers::privacy::seal_attestation(
            "openrouter",
            &params,
            lr_providers::privacy::ProviderPrivacyAttestation {
                local_execution: false,
                no_retention: true,
                source: "Reviewed gateway account and downstream chain".into(),
                reviewed_at: chrono::Utc::now().timestamp(),
                expires_at: None,
            },
        )
        .unwrap();
        params.insert(lr_providers::privacy::ATTESTATION_CONFIG.into(), sealed);
        registry
            .create_provider("gateway".into(), "openrouter".into(), params)
            .await
            .unwrap();
        let db = Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(
                std::env::temp_dir().join(format!("privacy-metrics-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
        );
        let router = Router::new_without_free_tier(
            manager,
            registry,
            Arc::new(crate::RateLimiterManager::new(None)),
            Arc::new(lr_monitoring::metrics::MetricsCollector::new(db)),
        );
        let mut request = CompletionRequest::new("test", vec![]);
        request.extensions = Some(HashMap::from([
            (
                INTERNAL_EXTENSION.into(),
                serde_json::json!({"no_retention":false}),
            ),
            (
                lr_providers::gateway::EXTENSION.into(),
                serde_json::json!({"zdr":false,"data_collection":"allow"}),
            ),
        ]));
        let output = router
            .privacy_provider_request("privacy-client", "gateway", request, EndpointType::Chat)
            .unwrap();
        let preferences = &output.extensions.as_ref().unwrap()[lr_providers::gateway::EXTENSION];
        assert_eq!(preferences["zdr"], true);
        assert_eq!(preferences["data_collection"], "deny");
        assert_eq!(
            output.extensions.as_ref().unwrap()[INTERNAL_EXTENSION]["no_retention"],
            true
        );
        let cleaned = output.clone().without_routing_metadata();
        assert!(!cleaned
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(INTERNAL_EXTENSION)));
        assert!(router
            .privacy_provider_request("privacy-client", "gateway", output, EndpointType::SystemOne)
            .is_err());
    }
}
