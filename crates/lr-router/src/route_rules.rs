//! Local deterministic conditions; never discovers models or invokes a classifier.
use crate::{decision_routing, Router};
use lr_config::{RouteRule, RoutingPolicy, Strategy};
use lr_providers::{ChatMessageContent, CompletionRequest, ContentPart, PreComputedRouting};
use sha2::{Digest, Sha256};

fn text_chars(request: &CompletionRequest) -> usize {
    request
        .messages
        .iter()
        .map(|message| match &message.content {
            ChatMessageContent::Text(text) => text.chars().count(),
            ChatMessageContent::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text { text } => text.chars().count(),
                    _ => 0,
                })
                .sum(),
        })
        .sum()
}
fn branch<'a>(
    rule: &'a RouteRule,
    strategy: &Strategy,
    client: &str,
    session: Option<&str>,
) -> &'a str {
    let mut hash = Sha256::new();
    for value in [
        strategy.id.as_str(),
        rule.id.as_str(),
        client,
        session.unwrap_or(""),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    let digest = hash.finalize();
    let bucket = u64::from_be_bytes(digest[..8].try_into().unwrap()) % 10_000;
    let mut cumulative = 0u64;
    for branch in &rule.branches {
        cumulative += u64::from(branch.weight);
        if bucket < cumulative {
            return &branch.route;
        }
    }
    // Only validated policies enter this function.
    &rule
        .branches
        .last()
        .expect("validated route branches")
        .route
}
impl Router {
    pub(crate) fn rule_routing_decision(
        &self,
        strategy: &Strategy,
        policy: &RoutingPolicy,
        request: &CompletionRequest,
    ) -> Option<PreComputedRouting> {
        if !policy.enabled || policy.validate().is_err() {
            return None;
        }
        let client = request
            .extensions
            .as_ref()
            .and_then(|e| e.get(lr_providers::privacy::ROUTE_CLIENT_EXTENSION))
            .and_then(|value| value.as_str());
        let session = lr_providers::session::SessionOptions::from_request(request)
            .ok()
            .flatten();
        let has_tools = request
            .tools
            .as_ref()
            .is_some_and(|tools| !tools.is_empty());
        let has_images = request
            .messages
            .iter()
            .any(|message| message.content.has_images());
        let chars = text_chars(request);
        for rule in &policy.rules {
            let c = &rule.condition;
            if (!c.client_ids.is_empty()
                && !client.is_some_and(|id| c.client_ids.iter().any(|allowed| allowed == id)))
                || c.has_tools.is_some_and(|expected| expected != has_tools)
                || c.has_images.is_some_and(|expected| expected != has_images)
                || c.min_context_chars
                    .is_some_and(|min| chars < (min as usize))
                || c.max_context_chars
                    .is_some_and(|max| chars > (max as usize))
            {
                continue;
            }
            // Without a verified client context, percentage assignment is unknown.
            let route = if rule.branches.len() == 1 {
                rule.branches[0].route.as_str()
            } else {
                branch(
                    rule,
                    strategy,
                    match client {
                        Some(client) => client,
                        None => continue,
                    },
                    session.as_ref().map(|s| s.id.as_str()),
                )
            };
            if let Some(ceiling) = c.max_estimated_cost_usd {
                let baseline = strategy
                    .auto_config
                    .as_ref()
                    .map(|a| a.prioritized_models.as_slice())
                    .unwrap_or_default();
                let models = decision_routing::route_models(policy, baseline, route);
                let mut known = false;
                let mut within = true;
                for (priority, (provider, model)) in models.iter().enumerate() {
                    let candidate = self.assess_candidate(
                        strategy,
                        provider,
                        model,
                        request,
                        priority,
                        request.endpoint_type(),
                    );
                    if candidate.eligible {
                        known = true;
                        if candidate
                            .estimated_request_cost_usd
                            .is_none_or(|cost| cost > ceiling)
                        {
                            within = false;
                            break;
                        }
                    }
                }
                if !known || !within {
                    continue;
                }
            }
            let mut decision =
                decision_routing::fallback(policy, format!("rule:{};branch:{}", rule.id, route));
            decision.route = route.into();
            decision.source = "deterministic_rule".into();
            return Some(decision);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    fn fixture() -> (tempfile::TempDir, Router, Strategy, RoutingPolicy) {
        let dir = tempfile::tempdir().unwrap();
        let router = Router::new_without_free_tier(
            Arc::new(lr_config::ConfigManager::new(
                lr_config::AppConfig::default(),
                dir.path().join("config.yaml"),
            )),
            Arc::new(lr_providers::registry::ProviderRegistry::new()),
            Arc::new(crate::RateLimiterManager::new(None)),
            Arc::new(lr_monitoring::metrics::MetricsCollector::new(Arc::new(
                lr_monitoring::storage::MetricsDatabase::new(dir.path().join("metrics.db"))
                    .unwrap(),
            ))),
        );
        let mut strategy = Strategy::new("rules".into());
        strategy.id = "rules-fixture".into();
        let policy:RoutingPolicy=serde_json::from_value(json!({"enabled":true,"default_route":"b","options":[{"id":"a","description":"A","models":[["p","a"]]},{"id":"b","description":"B","models":[["p","b"]]}],"rules":[{"id":"first","condition":{"client_ids":["alice"],"min_context_chars":2,"max_context_chars":2},"branches":[{"route":"a","weight":10000}]}]})).unwrap();
        strategy.auto_config.as_mut().unwrap().routing_policy = Some(policy.clone());
        (dir, router, strategy, policy)
    }
    fn request(router: &Router, client: &str) -> CompletionRequest {
        let mut request = CompletionRequest::new(
            "localrouter/auto",
            vec![serde_json::from_value(json!({"role":"user","content":"é🙂"})).unwrap()],
        );
        request.extensions = Some(
            [(
                lr_providers::privacy::ROUTE_CLIENT_EXTENSION.into(),
                json!("alice"),
            )]
            .into_iter()
            .collect(),
        );
        router
            .prepare_privacy_request(client, &mut request)
            .unwrap();
        request
    }
    #[test]
    fn trusted_identity_unicode_boundaries_and_mode_precedence_are_shared_with_preview() {
        let (_dir, router, strategy, mut policy) = fixture();
        let alice = request(&router, "alice");
        assert_eq!(
            router
                .deterministic_routing_decision(&strategy, &policy, &alice)
                .unwrap()
                .route,
            "a"
        );
        let bob = request(&router, "bob");
        assert_eq!(
            router
                .deterministic_routing_decision(&strategy, &policy, &bob)
                .unwrap()
                .route,
            "b"
        );
        policy.mode_rules.push(lr_config::ModeRouteRule {
            mode: "plan".into(),
            route: "b".into(),
        });
        let mut explicit = alice;
        explicit.metadata = Some(
            [("localrouter.mode".into(), "plan".into())]
                .into_iter()
                .collect(),
        );
        let decision = router
            .deterministic_routing_decision(&strategy, &policy, &explicit)
            .unwrap();
        assert_eq!(decision.route, "b");
        assert_eq!(decision.source, "client_mode");
        let cleaned = explicit.without_routing_metadata();
        assert!(!cleaned
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(lr_providers::privacy::ROUTE_CLIENT_EXTENSION)));
    }
    #[test]
    fn percentage_assignment_is_stable_scoped_and_unknown_without_authenticated_context() {
        let (_dir, router, strategy, mut policy) = fixture();
        policy.rules[0].condition = Default::default();
        policy.rules[0].branches = vec![
            lr_config::RouteBranch {
                route: "a".into(),
                weight: 5000,
            },
            lr_config::RouteBranch {
                route: "b".into(),
                weight: 5000,
            },
        ];
        let mut routes = std::collections::HashSet::new();
        for i in 0..100 {
            let mut request = request(&router, &format!("client-{i}"));
            request.extensions.as_mut().unwrap().insert(
                lr_providers::session::EXTENSION.into(),
                json!({"id":"same-session"}),
            );
            let first = router
                .deterministic_routing_decision(&strategy, &policy, &request)
                .unwrap();
            for _ in 0..4 {
                assert_eq!(
                    router
                        .deterministic_routing_decision(&strategy, &policy, &request)
                        .unwrap()
                        .route,
                    first.route
                );
            }
            routes.insert(first.route);
        }
        assert_eq!(routes.len(), 2);
        let unknown = request(&router, "internal-test");
        assert!(router
            .deterministic_routing_decision(&strategy, &policy, &unknown)
            .is_none());
    }
    #[test]
    fn unknown_quotes_do_not_satisfy_budget_conditions_and_ordered_fallback_still_works() {
        let (_dir, router, strategy, mut policy) = fixture();
        policy.rules[0].condition = lr_config::RouteCondition {
            max_estimated_cost_usd: Some(1.0),
            ..Default::default()
        };
        policy.rules.push(lr_config::RouteRule {
            id: "otherwise".into(),
            condition: Default::default(),
            branches: vec![lr_config::RouteBranch {
                route: "b".into(),
                weight: 10000,
            }],
        });
        let decision = router
            .deterministic_routing_decision(&strategy, &policy, &request(&router, "alice"))
            .unwrap();
        assert_eq!(decision.route, "b");
        assert_eq!(decision.reason, "rule:otherwise;branch:b");
    }
    #[tokio::test]
    async fn publication_during_preflight_rejects_stale_decisions_before_dispatch() {
        let (_dir, router, mut strategy, policy) = fixture();
        let mut request = request(&router, "alice");
        request.pre_computed_routing = Some(
            router
                .evaluate_routing_policy_for_endpoint(
                    "alice",
                    &strategy,
                    &policy,
                    &request,
                    lr_providers::EndpointType::Chat,
                )
                .await,
        );
        strategy
            .auto_config
            .as_mut()
            .unwrap()
            .prioritized_models
            .push(("p".into(), "changed".into()));
        let error = router
            .select_models_for_auto_routing(
                "alice",
                &strategy,
                strategy.auto_config.as_ref().unwrap(),
                &request,
                lr_providers::EndpointType::Chat,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed during preflight"));
    }
}
