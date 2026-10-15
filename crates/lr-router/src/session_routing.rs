//! Bounded, process-local affinity. Entries contain hashes and routing facts, never prompts.
use crate::{explanation::fingerprint, Router};
use lr_config::{SessionRoutingConfig, Strategy};
use lr_providers::{session::SessionOptions, CompletionRequest, EndpointType, PreComputedRouting};
use lr_types::{AppError, AppResult};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
const MAX_SESSIONS: usize = 4096;
#[derive(Clone)]
struct Pin {
    provider: String,
    model: String,
    route: String,
}
struct Entry {
    at: Instant,
    pin: Option<Pin>,
    classification: Option<(String, PreComputedRouting)>,
    classifier_generation: u64,
    reset_token: Option<String>,
}
impl Default for Entry {
    fn default() -> Self {
        Self {
            at: Instant::now(),
            pin: None,
            classification: None,
            classifier_generation: 0,
            reset_token: None,
        }
    }
}
pub(crate) struct SessionStore {
    salt: String,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
}
impl Default for SessionStore {
    fn default() -> Self {
        Self {
            salt: uuid::Uuid::new_v4().to_string(),
            entries: Mutex::new(HashMap::new()),
        }
    }
}
pub(crate) struct ClassificationSlot {
    client_id: String,
    entry: Arc<Mutex<Entry>>,
    generation: u64,
    context: String,
}
pub(crate) struct SessionHandle {
    status: Mutex<&'static str>,
    entry: Arc<Mutex<Entry>>,
    options: SessionOptions,
    route: String,
    pub fingerprint: String,
}
impl SessionHandle {
    pub fn record(&self, provider: &str, model: &str) {
        let mut entry = self.entry.lock();
        entry.at = Instant::now();
        entry.pin = Some(Pin {
            provider: provider.into(),
            model: model.into(),
            route: self.route.clone(),
        });
    }
    pub fn metadata(&self) -> Value {
        json!({"session_fingerprint":self.fingerprint,"scope":"client_policy","expires_on_restart":true,"status":*self.status.lock(),"continuity_required":self.options.require_continuity})
    }
    pub fn prioritize(
        &self,
        models: &mut Vec<(String, String)>,
        eligible: impl Fn(&str, &str) -> bool,
        group: impl Fn(&str, &str) -> usize,
    ) -> AppResult<()> {
        let pin = self.entry.lock().pin.clone();
        let bound = self.options.require_continuity;
        let Some(pin) = pin else {
            return if bound { Err(reset_required()) } else { Ok(()) };
        };
        let index = models
            .iter()
            .position(|(p, m)| p == &pin.provider && m == &pin.model);
        if pin.route != self.route || index.is_none() || !eligible(&pin.provider, &pin.model) {
            if bound {
                return Err(reset_required());
            }
            *self.status.lock() = "invalidated";
            self.entry.lock().pin = None;
            return Ok(());
        }
        *self.status.lock() = "hit";
        let index = index.unwrap();
        let pin_group = group(&pin.provider, &pin.model);
        if bound {
            if models
                .iter()
                .any(|(p, m)| group(p, m) < pin_group && eligible(p, m))
            {
                return Err(reset_required());
            }
            *models = vec![(pin.provider, pin.model)];
        } else {
            let first = models
                .iter()
                .position(|(p, m)| group(p, m) == pin_group)
                .unwrap_or(index);
            let pinned = models.remove(index);
            models.insert(first, pinned);
        }
        Ok(())
    }
}
fn reset_required() -> AppError {
    AppError::InvalidParams("Session continuity requires the original eligible provider/model; retry after temporary limits, or reset and start a new conversation without provider-bound continuation state".into())
}
impl Router {
    fn session_config<'a>(&self, strategy: &'a Strategy) -> Option<&'a SessionRoutingConfig> {
        strategy
            .auto_config
            .as_ref()
            .map(|a| &a.session_routing)
            .filter(|s| s.enabled || s.classifier_reuse || s.forward_cache_key)
    }
    fn session_key(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
        options: &SessionOptions,
    ) -> String {
        fingerprint(json!([
            self.sessions.salt,
            client_id,
            self.client_privacy(client_id),
            options.id,
            strategy,
            self.provider_registry.routing_revision(),
            self.config_manager.get().pricing_overrides,
            format!("{endpoint:?}"),
            crate::decision_routing::explicit_mode(request.metadata.as_ref()),
            request
                .extensions
                .as_ref()
                .and_then(|e| e.get(lr_providers::gateway::EXTENSION))
        ]))
    }
    fn session_entry(
        &self,
        key: &str,
        config: &SessionRoutingConfig,
        reset: bool,
    ) -> Arc<Mutex<Entry>> {
        self.session_entry_with_token(key, config, reset, None)
    }
    fn session_request_entry(
        &self,
        key: &str,
        config: &SessionRoutingConfig,
        reset: bool,
        request: &CompletionRequest,
    ) -> Arc<Mutex<Entry>> {
        let token = request
            .extensions
            .as_ref()
            .and_then(|e| e.get(lr_providers::session::CONTROL_TOKEN))
            .and_then(Value::as_str);
        self.session_entry_with_token(key, config, reset, token)
    }
    fn session_entry_with_token(
        &self,
        key: &str,
        config: &SessionRoutingConfig,
        reset: bool,
        token: Option<&str>,
    ) -> Arc<Mutex<Entry>> {
        let mut entries = self.sessions.entries.lock();
        let reset = reset
            && entries.get(key).is_none_or(|entry| {
                token.is_none() || entry.lock().reset_token.as_deref() != token
            });
        if reset
            || entries.get(key).is_some_and(|entry| {
                entry.lock().at.elapsed() >= Duration::from_secs(config.ttl_secs)
            })
        {
            entries.remove(key);
        }
        if let Some(entry) = entries.get(key) {
            return entry.clone();
        }
        if entries.len() >= MAX_SESSIONS {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.lock().at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        let entry = Arc::new(Mutex::new(Entry {
            reset_token: token.map(str::to_owned),
            ..Default::default()
        }));
        entries.insert(key.into(), entry.clone());
        entry
    }
    pub(crate) fn session_handle(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
        route: &str,
    ) -> AppResult<Option<SessionHandle>> {
        if self.client_privacy(client_id).no_retention {
            if SessionOptions::from_request(request)?
                .is_some_and(|options| options.require_continuity)
            {
                return Err(reset_required());
            }
            return Ok(None);
        }
        let options = SessionOptions::from_request(request)?;
        let Some(options) = options else {
            return Ok(None);
        };
        let Some(config) = self.session_config(strategy).filter(|c| c.enabled) else {
            return if options.require_continuity {
                Err(reset_required())
            } else {
                Ok(None)
            };
        };
        let key = self.session_key(client_id, strategy, request, endpoint, &options);
        if options.require_continuity {
            if let Some(entry) = self.sessions.entries.lock().get(&key) {
                if entry.lock().pin.as_ref().is_some_and(|pin| {
                    self.provider_registry
                        .get_provider(&pin.provider)
                        .is_some_and(|p| p.supports_gateway_options())
                }) {
                    return Err(reset_required());
                }
            }
        }
        Ok(Some(SessionHandle {
            status: Mutex::new("miss"),
            entry: self.session_request_entry(&key, config, options.reset, request),
            options,
            route: route.into(),
            fingerprint: key,
        }))
    }
    pub(crate) fn reused_classification(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        context: &Value,
        endpoint: EndpointType,
    ) -> Option<PreComputedRouting> {
        if self.client_privacy(client_id).no_retention {
            return None;
        }
        let config = self
            .session_config(strategy)
            .filter(|c| c.classifier_reuse)?;
        let options = SessionOptions::from_request(request).ok()??;
        if options.reset || options.reclassify {
            return None;
        }
        let key = self.session_key(client_id, strategy, request, endpoint, &options);
        let entry = self.session_entry(&key, config, false);
        let entry = entry.lock();
        let (hash, decision) = entry.classification.as_ref()?;
        if hash != &fingerprint(context.clone()) {
            return None;
        }
        let mut decision = decision.clone();
        decision.source = "classifier_reuse".into();
        decision.reason = "unchanged_routing_context".into();
        decision.latency_ms = 0;
        decision.classifier_cost_usd = Some(0.0);
        decision.classifier_cost_estimated = Some(false);
        Some(decision)
    }
    pub(crate) fn classification_slot(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        context: &Value,
        endpoint: EndpointType,
    ) -> Option<ClassificationSlot> {
        if self.client_privacy(client_id).no_retention {
            return None;
        }
        let config = self
            .session_config(strategy)
            .filter(|c| c.classifier_reuse)?;
        let options = SessionOptions::from_request(request).ok()??;
        let key = self.session_key(client_id, strategy, request, endpoint, &options);
        let entry = self.session_request_entry(&key, config, options.reset, request);
        let generation = {
            let mut state = entry.lock();
            if options.reclassify {
                state.classifier_generation = state.classifier_generation.wrapping_add(1);
                state.classification = None;
            }
            state.classifier_generation
        };
        Some(ClassificationSlot {
            client_id: client_id.into(),
            entry,
            generation,
            context: fingerprint(context.clone()),
        })
    }
    pub(crate) fn save_classification(
        &self,
        slot: Option<&ClassificationSlot>,
        decision: &PreComputedRouting,
    ) {
        if decision.source != "decision_model" {
            return;
        }
        let Some(slot) = slot else {
            return;
        };
        if self.client_privacy(&slot.client_id).no_retention {
            return;
        }
        let mut entry = slot.entry.lock();
        if entry.classifier_generation == slot.generation {
            entry.at = Instant::now();
            entry.classification = Some((slot.context.clone(), decision.clone()));
        }
    }
}

impl Router {
    pub(crate) fn provider_session_request(
        &self,
        client_id: &str,
        provider: &str,
        request: &CompletionRequest,
        endpoint: EndpointType,
    ) -> AppResult<(CompletionRequest, Option<SessionHandle>)> {
        let mut output =
            self.privacy_provider_request(client_id, provider, request.clone(), endpoint)?;
        let mut handle = None;
        if self.client_privacy(client_id).no_retention {
            if SessionOptions::from_request(request)?
                .is_some_and(|options| options.require_continuity)
            {
                return Err(reset_required());
            }
            if let Some(extensions) = output.extensions.as_mut() {
                extensions.remove(lr_providers::session::CACHE_KEY);
            }
            return Ok((output, None));
        }
        let options = SessionOptions::from_request(request)?;
        let supports = self
            .provider_registry
            .get_provider(provider)
            .is_some_and(|p| p.supports_prompt_cache_key());
        if !supports {
            if let Some(extensions) = output.extensions.as_mut() {
                extensions.remove(lr_providers::session::CACHE_KEY);
            }
        }
        if options.is_none() {
            return Ok((output, None));
        }
        if matches!(client_id, "internal-test" | "memory-service") {
            if options.as_ref().is_some_and(|o| o.require_continuity) {
                return Err(reset_required());
            }
            return Ok((output, None));
        }
        let (_, strategy) = self.validate_client_and_strategy(client_id)?;
        if let Some(options) = &options {
            let config = self.session_config(&strategy);
            if options.require_continuity
                && self
                    .provider_registry
                    .get_provider(provider)
                    .is_some_and(|p| p.supports_gateway_options())
            {
                return Err(reset_required());
            }
            if request.model != "localrouter/auto"
                && config.is_none_or(|c| !c.enabled)
                && options.require_continuity
            {
                return Err(reset_required());
            }
            if let Some(config) =
                config.filter(|c| c.enabled && request.model != "localrouter/auto")
            {
                let key = self.session_key(client_id, &strategy, request, endpoint, options);
                let entry = self.session_request_entry(&key, config, options.reset, request);
                let entry_handle = entry.clone();
                let mut entry = entry.lock();
                if let Some(pin) = &entry.pin {
                    if provider != pin.provider
                        || request
                            .model
                            .strip_prefix(&format!("{provider}/"))
                            .unwrap_or(&request.model)
                            != pin.model
                    {
                        if options.require_continuity {
                            return Err(reset_required());
                        }
                        entry.pin = None;
                        entry.classification = None;
                    }
                } else if options.require_continuity {
                    return Err(reset_required());
                }
                let route = entry
                    .pin
                    .as_ref()
                    .map(|p| p.route.clone())
                    .unwrap_or_else(|| "explicit".into());
                drop(entry);
                handle = Some(SessionHandle {
                    status: Mutex::new("explicit"),
                    entry: entry_handle,
                    options: options.clone(),
                    route,
                    fingerprint: key,
                });
            }
        }
        let supports = self
            .provider_registry
            .get_provider(provider)
            .is_some_and(|p| p.supports_prompt_cache_key());
        if supports
            && self
                .session_config(&strategy)
                .is_some_and(|c| c.forward_cache_key)
        {
            if let Some(options) = options {
                output
                    .extensions
                    .get_or_insert_with(Default::default)
                    .entry(lr_providers::session::CACHE_KEY.into())
                    .or_insert_with(|| {
                        json!(fingerprint(json!([
                            self.sessions.salt,
                            client_id,
                            options.id
                        ])))
                    });
            }
        }
        if !supports {
            if let Some(extensions) = output.extensions.as_mut() {
                extensions.remove(lr_providers::session::CACHE_KEY);
            }
        }
        Ok((output, handle))
    }
}

impl Router {
    /// Validate signed continuation before any classifier or answer inference.
    pub(crate) fn bound_routing_decision(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
    ) -> AppResult<Option<PreComputedRouting>> {
        if self.client_privacy(client_id).no_retention {
            if SessionOptions::from_request(request)?
                .is_some_and(|options| options.require_continuity)
            {
                return Err(reset_required());
            }
            return Ok(None);
        }
        let Some(options) = SessionOptions::from_request(request)?.filter(|o| o.require_continuity)
        else {
            return Ok(None);
        };
        let config = self
            .session_config(strategy)
            .filter(|c| c.enabled)
            .ok_or_else(reset_required)?;
        let key = self.session_key(client_id, strategy, request, endpoint, &options);
        let entry = self.session_entry(&key, config, false);
        let pin = entry.lock().pin.clone().ok_or_else(reset_required)?;
        if self
            .provider_registry
            .get_provider(&pin.provider)
            .is_some_and(|p| p.supports_gateway_options())
        {
            return Err(AppError::InvalidParams("Strict continuation through an upstream gateway requires verified inference-provider pinning; choose a direct provider/model".into()));
        }
        if !self
            .assess_candidate(strategy, &pin.provider, &pin.model, request, 0, endpoint)
            .eligible
        {
            return Err(reset_required());
        }
        let auto = strategy.auto_config.as_ref().ok_or_else(reset_required)?;
        let policy = auto.routing_policy.clone().unwrap_or_default();
        let pool = if policy.enabled {
            if !policy.options.iter().any(|o| o.id == pin.route) {
                return Err(reset_required());
            }
            crate::decision_routing::route_models(&policy, &auto.prioritized_models, &pin.route)
        } else {
            auto.prioritized_models.clone()
        };
        if !pool
            .iter()
            .any(|(p, m)| p == &pin.provider && m == &pin.model)
        {
            return Err(reset_required());
        }
        let mut decision = crate::decision_routing::fallback(&policy, "provider_bound_session");
        decision.route = pin.route;
        decision.source = "session_affinity".into();
        decision.classifier_cost_usd = Some(0.0);
        decision.classifier_cost_estimated = Some(false);
        Ok(Some(decision))
    }
}

impl Router {
    /// Inspect a session without resetting it, refreshing TTL, allocating entries or doing IO.
    pub fn preview_session(
        &self,
        client_id: &str,
        strategy: &Strategy,
        request: &CompletionRequest,
        endpoint: EndpointType,
    ) -> AppResult<Option<Value>> {
        let Some(options) = SessionOptions::from_request(request)? else {
            return Ok(None);
        };
        let Some(config) = self.session_config(strategy) else {
            return Ok(Some(json!({"status":"disabled","read_only":true})));
        };
        let key = self.session_key(client_id, strategy, request, endpoint, &options);
        let entry = self.sessions.entries.lock().get(&key).cloned();
        let mut result = json!({"session_fingerprint":key,"status":"no_current_pin_or_configuration_changed","read_only":true,"expires_on_restart":true,"continuity_required":options.require_continuity,"ttl_secs":config.ttl_secs});
        if options.reset {
            result["status"] = json!("reset_requested");
            return Ok(Some(result));
        }
        let Some(entry) = entry else {
            return Ok(Some(result));
        };
        let (at, pin, classification) = {
            let state = entry.lock();
            (
                state.at,
                state.pin.clone(),
                state.classification.as_ref().map(|(hash, _)| hash.clone()),
            )
        };
        if at.elapsed() >= Duration::from_secs(config.ttl_secs) {
            result["status"] = json!("expired");
            return Ok(Some(result));
        }
        if let Some(pin) = &pin {
            let eligible = self
                .assess_candidate(strategy, &pin.provider, &pin.model, request, 0, endpoint)
                .eligible;
            result["status"] = json!(if eligible {
                "eligible_pin"
            } else {
                "ineligible_pin"
            });
            result["pin"] = json!({"provider":pin.provider,"model":pin.model,"route":pin.route,"eligible":eligible});
        }
        let reusable = strategy
            .auto_config
            .as_ref()
            .and_then(|a| a.routing_policy.as_ref())
            .is_some_and(|policy| {
                let context = crate::decision_routing::routing_context(
                    &request.messages,
                    request.metadata.as_ref(),
                    policy,
                );
                config.classifier_reuse
                    && !options.reclassify
                    && classification
                        .as_ref()
                        .is_some_and(|hash| hash == &fingerprint(context))
            });
        result["classifier_context_reusable"] = json!(reusable);
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn router() -> Router {
        let config = Arc::new(lr_config::ConfigManager::new(
            lr_config::AppConfig::default(),
            std::env::temp_dir().join(format!("session-config-{}.yaml", uuid::Uuid::new_v4())),
        ));
        let db = Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(
                std::env::temp_dir().join(format!("session-metrics-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
        );
        Router::new_without_free_tier(
            config,
            Arc::new(lr_providers::registry::ProviderRegistry::new()),
            Arc::new(crate::RateLimiterManager::new(None)),
            Arc::new(lr_monitoring::metrics::MetricsCollector::new(db)),
        )
    }
    fn strategy() -> Strategy {
        let mut strategy = Strategy::new("sessions".into());
        let config = &mut strategy.auto_config.as_mut().unwrap().session_routing;
        config.enabled = true;
        config.classifier_reuse = true;
        strategy
    }
    fn request() -> CompletionRequest {
        let mut request = CompletionRequest::new("localrouter/auto", vec![]);
        request.extensions = Some(
            [(
                lr_providers::session::EXTENSION.into(),
                json!({"id":"same-session"}),
            )]
            .into_iter()
            .collect(),
        );
        request
    }
    #[tokio::test]
    async fn strict_continuity_skips_classification_and_checks_the_original_pin() {
        let router = router();
        let mut strategy = strategy();
        let mut request = request();
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["require_continuity"] = json!(true);
        let policy = lr_config::RoutingPolicy {
            decision_model: Some(("paid".into(), "classifier".into())),
            enabled: true,
            options: vec![
                lr_config::RoutingOption {
                    id: "general".into(),
                    description: "General work".into(),
                    models: vec![],
                },
                lr_config::RoutingOption {
                    id: "other".into(),
                    description: "Other work".into(),
                    models: vec![],
                },
            ],
            ..Default::default()
        };
        strategy.auto_config.as_mut().unwrap().routing_policy = Some(policy.clone());
        policy.validate().unwrap();
        let rejected = router
            .evaluate_routing_policy("a", &strategy, &policy, &request)
            .await;
        assert_eq!(rejected.reason, "session_reset_required");
        router.provider_registry.register_factory(Arc::new(
            lr_providers::factory::OpenAICompatibleProviderFactory,
        ));
        router
            .provider_registry
            .create_provider(
                "p".into(),
                "openai_compatible".into(),
                [
                    ("base_url".into(), "http://127.0.0.1:1".into()),
                    ("api_key".into(), "key".into()),
                ]
                .into_iter()
                .collect(),
            )
            .await
            .unwrap();
        strategy.auto_config.as_mut().unwrap().prioritized_models =
            vec![("p".into(), "lr-session-fixture-native-chat".into())];
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["require_continuity"] = json!(false);
        router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap()
            .record("p", "lr-session-fixture-native-chat");
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["require_continuity"] = json!(true);
        let decision = router
            .evaluate_routing_policy("a", &strategy, &policy, &request)
            .await;
        assert_eq!(decision.source, "session_affinity");
        assert_eq!(decision.route, "general");
        assert_eq!(decision.classifier_cost_usd, Some(0.0));
        strategy
            .auto_config
            .as_mut()
            .unwrap()
            .prioritized_models
            .clear();
        assert!(router
            .bound_routing_decision("a", &strategy, &request, EndpointType::Chat)
            .is_err());
    }
    #[tokio::test]
    async fn read_only_preview_and_live_ordering_agree_on_session_preference() {
        let router = router();
        let mut strategy = strategy();
        let mut request = request();
        router.provider_registry.register_factory(Arc::new(
            lr_providers::factory::OpenAICompatibleProviderFactory,
        ));
        router
            .provider_registry
            .create_provider(
                "p".into(),
                "openai_compatible".into(),
                [
                    ("base_url".into(), "http://127.0.0.1:1".into()),
                    ("api_key".into(), "key".into()),
                ]
                .into_iter()
                .collect(),
            )
            .await
            .unwrap();
        let models = vec![
            ("p".into(), "lr-session-fixture-alpha".into()),
            ("p".into(), "lr-session-fixture-beta".into()),
        ];
        strategy.auto_config.as_mut().unwrap().prioritized_models = models.clone();
        let handle = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "baseline")
            .unwrap()
            .unwrap();
        handle.record("p", "lr-session-fixture-beta");
        let at = handle.entry.lock().at;
        let policy = lr_config::RoutingPolicy::default();
        let mut preview = router.routing_explanation(&strategy, &policy, &request, None);
        router
            .apply_session_preview("a", &strategy, &request, EndpointType::Chat, &mut preview)
            .unwrap();
        let (live, _, _) = router
            .order_auto_models("a", &strategy, &request, &models, None, EndpointType::Chat)
            .unwrap();
        assert_eq!(
            preview.routes[0]
                .candidates
                .iter()
                .map(|c| (c.provider.clone(), c.model.clone()))
                .collect::<Vec<_>>(),
            live
        );
        assert_eq!(
            live[0].1,
            "lr-session-fixture-beta",
            "preview={preview:?}; pin={:?}; assessment={:?}",
            handle
                .entry
                .lock()
                .pin
                .as_ref()
                .map(|p| (&p.provider, &p.model, &p.route)),
            router.assess_candidate(
                &strategy,
                "p",
                "lr-session-fixture-beta",
                &request,
                0,
                EndpointType::Chat
            )
        );
        assert_eq!(handle.entry.lock().at, at);
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["require_continuity"] = json!(true);
        router
            .apply_session_preview("a", &strategy, &request, EndpointType::Chat, &mut preview)
            .unwrap();
        assert!(!preview.classifier_required);
        assert_eq!(preview.routes[0].candidates.len(), 1);
        handle.entry.lock().at = Instant::now() - Duration::from_secs(3601);
        router
            .apply_session_preview("a", &strategy, &request, EndpointType::Chat, &mut preview)
            .unwrap();
        assert!(preview.no_route);
        assert_eq!(preview.reason, "session_reset_required");
        assert!(Arc::ptr_eq(
            router
                .sessions
                .entries
                .lock()
                .get(&handle.fingerprint)
                .unwrap(),
            &handle.entry
        ));
        let fingerprint = router
            .routing_explanation(&strategy, &policy, &request, None)
            .config_fingerprint;
        strategy
            .auto_config
            .as_mut()
            .unwrap()
            .session_routing
            .enabled = false;
        assert_ne!(
            fingerprint,
            router
                .routing_explanation(&strategy, &policy, &request, None)
                .config_fingerprint
        );
    }
    #[test]
    fn pins_are_isolated_by_client_policy_mode_endpoint_and_restart() {
        let router = router();
        let strategy = strategy();
        let request = request();
        let a = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        a.record("p", "m");
        let b = router
            .session_handle("b", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        assert!(b.entry.lock().pin.is_none());
        assert_ne!(a.fingerprint, b.fingerprint);
        let other = router
            .session_handle("a", &strategy, &request, EndpointType::SystemOne, "general")
            .unwrap()
            .unwrap();
        assert!(other.entry.lock().pin.is_none());
        let mut mode = request.clone();
        mode.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        assert_ne!(
            a.fingerprint,
            router
                .session_handle("a", &strategy, &mode, EndpointType::Chat, "general")
                .unwrap()
                .unwrap()
                .fingerprint
        );
        let mut changed = strategy.clone();
        changed.name = "new configuration".into();
        assert!(router
            .session_handle("a", &changed, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap()
            .entry
            .lock()
            .pin
            .is_none());
        assert!(!a.fingerprint.contains("same-session"));
        assert!(!a.metadata().to_string().contains("same-session"));
        let restarted = self::router();
        assert_ne!(
            a.fingerprint,
            restarted
                .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
                .unwrap()
                .unwrap()
                .fingerprint
        );
    }
    #[test]
    fn pin_cannot_jump_groups_or_bypass_revocation_or_continuity() {
        let router = router();
        let strategy = strategy();
        let request = request();
        let mut handle = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        handle.record("p", "c");
        let mut models = vec![
            ("p".into(), "a".into()),
            ("p".into(), "b".into()),
            ("p".into(), "c".into()),
        ];
        handle
            .prioritize(&mut models, |_, _| true, |_, m| usize::from(m != "a"))
            .unwrap();
        assert_eq!(
            models.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>(),
            vec!["a", "c", "b"]
        );
        handle.options.require_continuity = true;
        assert!(handle
            .prioritize(&mut models, |_, _| true, |_, m| usize::from(m != "a"))
            .is_err());
        assert!(handle
            .prioritize(&mut models, |_, m| m != "c", |_, _| 0)
            .is_err());
        handle.options.require_continuity = false;
        handle
            .prioritize(&mut models, |_, m| m != "c", |_, _| 0)
            .unwrap();
        assert!(handle.entry.lock().pin.is_none());
    }
    #[test]
    fn reset_applies_once_per_turn_even_when_only_classifier_reuse_is_enabled() {
        let router = router();
        let mut strategy = strategy();
        strategy
            .auto_config
            .as_mut()
            .unwrap()
            .session_routing
            .enabled = false;
        let mut request = request();
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["reset"] = json!(true);
        lr_providers::session::prepare_turn(&mut request).unwrap();
        let context = json!({"request":"same"});
        let first = router
            .classification_slot("a", &strategy, &request, &context, EndpointType::Chat)
            .unwrap();
        let again = router
            .classification_slot("a", &strategy, &request, &context, EndpointType::Chat)
            .unwrap();
        assert!(Arc::ptr_eq(&first.entry, &again.entry));
        let mut decision =
            crate::decision_routing::fallback(&lr_config::RoutingPolicy::default(), "new");
        decision.source = "decision_model".into();
        router.save_classification(Some(&first), &decision);
        lr_providers::session::consume_turn_controls(&mut request);
        assert!(router
            .reused_classification("a", &strategy, &request, &context, EndpointType::Chat)
            .is_some());
        request
            .extensions
            .as_mut()
            .unwrap()
            .remove(lr_providers::session::CONTROL_TOKEN);
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["reset"] = json!(true);
        lr_providers::session::prepare_turn(&mut request).unwrap();
        let fresh = router
            .classification_slot("a", &strategy, &request, &context, EndpointType::Chat)
            .unwrap();
        assert!(!Arc::ptr_eq(&first.entry, &fresh.entry));
        router.save_classification(Some(&first), &decision);
        lr_providers::session::consume_turn_controls(&mut request);
        assert!(router
            .reused_classification("a", &strategy, &request, &context, EndpointType::Chat)
            .is_none());
    }
    #[test]
    fn session_preview_does_not_allocate_reset_or_refresh_expiry() {
        let router = router();
        let strategy = strategy();
        let mut request = request();
        let before = router.sessions.entries.lock().len();
        assert_eq!(
            router
                .preview_session("a", &strategy, &request, EndpointType::Chat)
                .unwrap()
                .unwrap()["status"],
            "no_current_pin_or_configuration_changed"
        );
        assert_eq!(router.sessions.entries.lock().len(), before);
        let handle = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "baseline")
            .unwrap()
            .unwrap();
        handle.record("p", "m");
        let at = handle.entry.lock().at;
        request
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["reset"] = json!(true);
        assert_eq!(
            router
                .preview_session("a", &strategy, &request, EndpointType::Chat)
                .unwrap()
                .unwrap()["status"],
            "reset_requested"
        );
        assert_eq!(handle.entry.lock().at, at);
        assert!(handle.entry.lock().pin.is_some());
        assert!(Arc::ptr_eq(
            router
                .sessions
                .entries
                .lock()
                .get(&handle.fingerprint)
                .unwrap(),
            &handle.entry
        ));
    }
    #[test]
    fn ttl_reset_and_eviction_do_not_allow_old_turns_to_repopulate_new_entries() {
        let router = router();
        let strategy = strategy();
        let request = request();
        let old = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        old.record("p", "old");
        old.entry.lock().at = Instant::now() - Duration::from_secs(3601);
        let current = router
            .session_handle("a", &strategy, &request, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        old.record("p", "late");
        assert!(current.entry.lock().pin.is_none());
        assert!(!Arc::ptr_eq(&old.entry, &current.entry));
        let mut reset = request.clone();
        reset
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["reset"] = json!(true);
        let newer = router
            .session_handle("a", &strategy, &reset, EndpointType::Chat, "general")
            .unwrap()
            .unwrap();
        current.record("p", "late-again");
        assert!(newer.entry.lock().pin.is_none());
        let config = &strategy.auto_config.as_ref().unwrap().session_routing;
        for index in 0..MAX_SESSIONS + 1 {
            router.session_entry(&index.to_string(), config, false);
        }
        assert_eq!(router.sessions.entries.lock().len(), MAX_SESSIONS);
    }
    #[test]
    fn classifier_reuse_is_exact_and_reclassification_rejects_stale_inflight_results() {
        let router = router();
        let strategy = strategy();
        let request = request();
        let context = json!({"latest_user_request":"classify"});
        let mut decision =
            crate::decision_routing::fallback(&lr_config::RoutingPolicy::default(), "test");
        decision.source = "decision_model".into();
        decision.classifier_cost_usd = Some(0.5);
        decision.latency_ms = 50;
        let old_slot = router
            .classification_slot("a", &strategy, &request, &context, EndpointType::Chat)
            .unwrap();
        router.save_classification(Some(&old_slot), &decision);
        let reused = router
            .reused_classification("a", &strategy, &request, &context, EndpointType::Chat)
            .unwrap();
        assert_eq!(reused.source, "classifier_reuse");
        assert_eq!(reused.latency_ms, 0);
        assert_eq!(reused.classifier_cost_usd, Some(0.0));
        assert!(router
            .reused_classification("b", &strategy, &request, &context, EndpointType::Chat)
            .is_none());
        assert!(router
            .reused_classification(
                "a",
                &strategy,
                &request,
                &json!({"latest_user_request":"changed"}),
                EndpointType::Chat
            )
            .is_none());
        let mut reclassify = request.clone();
        reclassify
            .extensions
            .as_mut()
            .unwrap()
            .get_mut(lr_providers::session::EXTENSION)
            .unwrap()["reclassify"] = json!(true);
        let fresh_slot = router
            .classification_slot("a", &strategy, &reclassify, &context, EndpointType::Chat)
            .unwrap();
        router.save_classification(Some(&old_slot), &decision);
        assert!(router
            .reused_classification("a", &strategy, &request, &context, EndpointType::Chat)
            .is_none());
        router.save_classification(Some(&fresh_slot), &decision);
        assert!(router
            .reused_classification("a", &strategy, &request, &context, EndpointType::Chat)
            .is_some());
    }
}
