use std::collections::HashMap;
use std::time::{Duration, Instant};

use lr_config::{AppConfig, Client, Strategy};
use lr_mcp::gateway::firewall::{FirewallApprovalAction, InterceptCategory, InterceptRule};
use lr_server::state::AppState;
use lr_types::AppError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use super::{AdminMcp, Tool, ID};
use crate::ui::{
    commands, commands_clients, commands_decision_routing, commands_local_models, commands_mcp,
    commands_providers, commands_routing_profiles, commands_usage,
};

const CONFIG_SECTIONS: &[&str] = &[
    "server",
    "proxy",
    "request_dedupe",
    "logging",
    "ui",
    "update",
    "model_cache",
    "streaming",
    "start_on_boot",
    "health_check",
    "context_management",
    "prompt_compression",
    "json_repair",
    "mcp_via_llm",
    "mcp_gateway",
    "responses",
    "systemone",
    "push_relay",
    "roots",
    "pricing_overrides",
    "skills",
    "marketplace",
    "guardrails",
    "coding_agents",
    "secret_scanning",
    "memory",
    "usage_tracking",
];

const CLIENT_FIELDS: &[&str] = &[
    "name",
    "enabled",
    "strategy_id",
    "llm_mode",
    "mcp_mode",
    "reverse_proxy",
    "roots",
    "guardrails",
    "prompt_compression",
    "json_repair",
    "secret_scanning",
    "context_management_enabled",
    "catalog_compression_enabled",
    "client_tools_indexing",
    "marketplace_permission",
    "coding_agent_permission",
    "coding_agent_type",
];

const STRATEGY_FIELDS: &[&str] = &[
    "name",
    "auto_config",
    "rate_limits",
    "free_tier_only",
    "free_tier_fallback",
    "response_cache",
    "routing_profile",
    "route_versions",
    "model_permissions",
];

const FIREWALL_FIELDS: &[&str] = &[
    "mcp_permissions",
    "skills_permissions",
    "model_permissions",
    "marketplace_permission",
    "coding_agent_permission",
    "guardrails",
    "secret_scanning",
    "push_permissions",
];

impl AdminMcp {
    pub(super) async fn execute(
        &self,
        tool: &Tool,
        args: Value,
        actor: &str,
        actor_name: &str,
    ) -> Result<Value, String> {
        let state = self.state.upgrade().ok_or("LocalRouter is shutting down")?;
        let desktop_config = self.app.state::<lr_config::ConfigManager>();
        let config = desktop_config.inner();
        let name = tool.name;
        validate_args(tool, &args)?;
        if tool.confirm || self.grant(actor).is_some_and(|grant| grant.ask) {
            let review = summarize_operation(name, &args);
            let approval = state
                .mcp_gateway
                .firewall_manager
                .request_approval(
                    actor.to_owned(),
                    actor_name.to_owned(),
                    name.to_owned(),
                    ID.to_owned(),
                    review.to_string(),
                    Some(120),
                    Some(review),
                )
                .await
                .map_err(|_| "Unable to request desktop confirmation")?;
            if approval.action != FirewallApprovalAction::AllowOnce
                || approval.edited_arguments.is_some()
            {
                return Err("Choose Allow Once in LocalRouter to approve this operation".into());
            }
            if !self
                .grant(actor)
                .is_some_and(|grant| tool.access.granted(&grant))
            {
                return Err("The client's administration grant changed".into());
            }
        }

        match name {
            "lr_clients_credentials" => {
                let id = string(&args, "client_id")?;
                if id == actor {
                    return Err(
                        "Change the active administration credential in the desktop.".into(),
                    );
                }
                find_client(&config.get(), id)?;
                match string(&args, "action")? {
                    "rotate" => {
                        let _secret = commands_clients::rotate_client_secret(
                            id.into(),
                            self.app.state(),
                            self.app.state(),
                            self.app.state(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?;
                    }
                    "revoke_tokens" => {
                        state.token_store.revoke_client_tokens(id);
                    }
                    _ => return Err("Expected rotate or revoke_tokens".into()),
                }
                Ok(
                    json!({"client_id": id, "action": args["action"], "next": "Open the client's Connection tab."}),
                )
            }
            "lr_routing_profiles_manage" => self.manage_profile(&args).await,
            "lr_local_models_list" => {
                let library = commands_local_models::local_models_library(self.app.state()).await?;
                let jobs = commands_local_models::local_models_downloads(self.app.state()).await?;
                let models = if let Some(provider) = args["provider"].as_str() {
                    commands_local_models::local_models_states(provider.into(), self.app.state())
                        .await?
                } else {
                    Vec::new()
                };
                Ok(json!({"library": library, "downloads": jobs, "states": models}))
            }
            "lr_local_models_manage" => self.manage_local_model(&args).await,
            "lr_mcp_servers_check" => {
                let id = string(&args, "server_id")?;
                if state.mcp_server_manager.get_config(id).is_none() {
                    return Err("MCP server not found".into());
                }
                let health = state.mcp_server_manager.get_server_health(id).await;
                Ok(
                    json!({"server_id": id, "status": health.status, "latency_ms": health.latency_ms,
                    "has_error": health.error.is_some()}),
                )
            }
            "lr_skill_sources_manage" => {
                match string(&args, "action")? {
                    "list" => {}
                    "add" => {
                        commands::add_skill_source(
                            string(&args, "path")?.into(),
                            self.app.state(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    "remove" => {
                        commands::remove_skill_source(
                            string(&args, "path")?.into(),
                            self.app.state(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    _ => return Err("Expected list, add or remove".into()),
                }
                let sources = commands::list_skill_sources(
                    self.app.state(),
                    self.app.state(),
                    self.app.state(),
                )
                .await?;
                Ok(json!({"sources": sources}))
            }
            "lr_installed_skills_list" => {
                Ok(json!({"skills": commands::list_skills(self.app.state()).await?}))
            }
            "lr_installed_skills_manage" => {
                let skill = string(&args, "name")?;
                let enabled = args["enabled"]
                    .as_bool()
                    .ok_or("Expected enabled boolean")?;
                commands::set_skill_enabled(
                    skill.into(),
                    enabled,
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                )
                .await?;
                Ok(json!({"name": skill, "enabled": enabled}))
            }
            "lr_accounts_manage" => {
                match string(&args, "action")? {
                    "list" => {}
                    "update" => {
                        let cfg = config.get();
                        let current = json!(cfg.usage_tracking);
                        let fields: Vec<_> = current
                            .as_object()
                            .into_iter()
                            .flatten()
                            .map(|(key, _)| key.as_str())
                            .filter(|key| *key != "tray_items_added")
                            .collect();
                        let next = patched(&cfg.usage_tracking, &args["settings"], &fields)?;
                        commands_usage::update_usage_tracking_config(
                            next,
                            self.app.clone(),
                            self.app.state(),
                            self.app.state(),
                        )
                        .await?;
                    }
                    "forget" => {
                        commands_usage::forget_usage_account(
                            string(&args, "account_id")?.into(),
                            self.app.state(),
                        )
                        .await?;
                    }
                    _ => return Err("Expected list, update or forget".into()),
                }
                Ok(
                    json!({"settings": config.get().usage_tracking, "accounts": commands_usage::get_usage_limits(self.app.state()).await?}),
                )
            }
            "lr_monitor_metrics" => {
                let end = chrono::Utc::now();
                let minutes = args["minutes"].as_u64().unwrap_or(60).clamp(1, 1_440);
                let start = end - chrono::Duration::minutes(minutes as i64);
                let metrics = &state.metrics_collector;
                let points = match (
                    args["client_id"].as_str(),
                    args["provider"].as_str(),
                    args["model"].as_str(),
                ) {
                    (None, None, None) => metrics.get_global_range(start, end),
                    (Some(id), None, None) => metrics.get_key_range(id, start, end),
                    (None, Some(instance), None) => {
                        metrics.get_provider_range(instance, start, end)
                    }
                    (None, Some(instance), Some(model)) => {
                        metrics.get_model_range(&format!("{instance}/{model}"), start, end)
                    }
                    _ => return Err("Select a client, provider, or model with its provider".into()),
                };
                let requests: u64 = points.iter().map(|p| p.requests).sum();
                Ok(json!({"start": start, "end": end, "requests": requests,
                    "errors": points.iter().map(|p|p.failed_requests).sum::<u64>(),
                    "tokens": points.iter().map(|p|p.total_tokens).sum::<u64>(),
                    "cost_usd": points.iter().map(|p|p.cost_usd).sum::<f64>(),
                    "mean_latency_ms": (requests > 0).then(|| points.iter().map(|p|p.total_latency_ms).sum::<u64>() as f64 / requests as f64)}))
            }
            "lr_monitor_events_wait" => {
                let cursor = args["after_sequence"]
                    .as_u64()
                    .ok_or("Expected after_sequence")?;
                let seconds = args["seconds"].as_u64().unwrap_or(10).clamp(1, 20);
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let filter = lr_monitor::MonitorEventFilter {
                    client_id: optional(&args, "client_id")?,
                    ..Default::default()
                };
                loop {
                    if !self
                        .grant(actor)
                        .is_some_and(|grant| tool.access.granted(&grant))
                    {
                        return Err("Monitor access changed".into());
                    }
                    if let Some(event) = state
                        .monitor_store
                        .list(0, 100, Some(&filter))
                        .events
                        .into_iter()
                        .take_while(|event| event.sequence > cursor)
                        .find(|event| {
                            event.client_id.as_deref() != Some(actor)
                                || event.event_type != lr_monitor::MonitorEventType::McpToolCall
                                || state.monitor_store.get(&event.id).is_some_and(|event| {
                                    !matches!(event.data, lr_monitor::MonitorEventData::McpToolCall { server_id, .. } if server_id == ID)
                                })
                        })
                    {
                        return Ok(
                            json!({"event_id": event.id, "sequence": event.sequence, "available": true}),
                        );
                    }
                    if Instant::now() >= deadline {
                        return Ok(json!({"available": false}));
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }

            "lr_clients_list" => Ok(
                json!({"clients": config.get().clients.iter().map(client_view).collect::<Vec<_>>()}),
            ),
            "lr_clients_get" => Ok(client_view(find_client(
                &config.get(),
                string(&args, "client_id")?,
            )?)),
            "lr_clients_create" => {
                let (_, client) = commands_clients::create_client(
                    string(&args, "name")?.to_owned(),
                    optional(&args, "llm_mode")?,
                    optional(&args, "mcp_mode")?,
                    None,
                    None,
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                )
                .await?;
                Ok(
                    json!({"client_id": client.id, "strategy_id": client.strategy_id,
                    "next": "Use lr_clients_connection to find the desktop connection instructions."}),
                )
            }
            "lr_clients_update" => {
                let id = string(&args, "client_id")?;
                let view = config
                    .try_update(|cfg| {
                        let current = find_client(cfg, id).map_err(AppError::Config)?;
                        let updated: Client = patched(current, &args["changes"], CLIENT_FIELDS)
                            .map_err(AppError::Config)?;
                        find_strategy(cfg, &updated.strategy_id).map_err(AppError::Config)?;
                        if id == actor && !updated.enabled {
                            return Err(AppError::Config(
                                "Disable the active admin client in the desktop".into(),
                            ));
                        }
                        let view = client_view(&updated);
                        *find_client_mut(cfg, id)? = updated;
                        Ok(view)
                    })
                    .map_err(display)?;
                config.save().await.map_err(display)?;
                if !find_client(&config.get(), id)?.enabled {
                    state.token_store.revoke_client_tokens(id);
                }
                let _ = self.app.emit("clients-changed", ());
                Ok(view)
            }
            "lr_clients_delete" => {
                let id = string(&args, "client_id")?;
                if id == actor {
                    return Err("Use the desktop to remove the active admin client".into());
                }
                commands_clients::delete_client(
                    id.into(),
                    self.app.state(),
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                )
                .await?;
                Ok(json!({"deleted": id}))
            }
            "lr_clients_connection" => {
                let cfg = config.get();
                let client = find_client(&cfg, string(&args, "client_id")?)?;
                let port = self
                    .app
                    .state::<std::sync::Arc<lr_server::ServerManager>>()
                    .get_actual_port()
                    .unwrap_or(cfg.server.port);
                Ok(
                    json!({"client_id": client.id, "mcp_url": format!("http://localhost:{port}/mcp"),
                    "localrouter_mcp_url": format!("http://localhost:{port}{}", lr_mcp::gateway::types::LOCALROUTER_MCP_PATH),
                "openai_base_url": format!("http://localhost:{port}/v1"),
                    "credential": format!("Open Clients → {} → Connection in LocalRouter.", client.name)}),
                )
            }
            "lr_provider_types_list" => {
                Ok(json!({"types": state.provider_registry.list_provider_types()}))
            }
            "lr_providers_list" => {
                Ok(json!({"providers": state.provider_registry.list_providers()}))
            }
            "lr_providers_get" | "lr_providers_credentials" => {
                let instance = string(&args, "provider")?;
                let cfg = config.get();
                let provider = cfg
                    .providers
                    .iter()
                    .find(|p| p.name == instance)
                    .ok_or("Provider not found")?;
                let key_configured = commands_providers::has_provider_api_key(instance.into())
                    .await
                    .unwrap_or(false);
                Ok(
                    json!({"name": provider.name, "provider_type": provider.provider_type, "enabled": provider.enabled,
                    "api_key_configured": key_configured, "credential_setup": "Use this instance's Configuration tab in Resources → Providers."}),
                )
            }
            "lr_providers_create" => {
                let instance = string(&args, "provider")?.to_owned();
                let provider_type = string(&args, "provider_type")?.to_owned();
                let settings: HashMap<String, String> =
                    optional(&args, "settings")?.unwrap_or_default();
                let provider = state
                    .provider_registry
                    .list_provider_types()
                    .into_iter()
                    .find(|p| p.provider_type == provider_type)
                    .ok_or("Unknown provider type")?;
                let fields = json!(provider.setup_parameters);
                for key in settings.keys() {
                    if !matches!(
                        key.as_str(),
                        "base_url" | "endpoint" | "timeout" | "timeout_seconds"
                    ) {
                        return Err("Supply only a provider URL or timeout here. Use the desktop provider dialog for other setup fields and credentials.".into());
                    }
                }
                for key in ["base_url", "endpoint"] {
                    if let Some(url) = settings.get(key) {
                        validate_provider_url(url)?;
                    }
                }
                commands_providers::create_provider_instance(
                    self.app.state(), self.app.state(), self.app.clone(), instance.clone(), provider_type, settings,
                ).await.map_err(|_| format!("Provider setup requires attention in Resources → Providers. Supported setup fields: {fields}"))?;
                Ok(json!({"provider": instance}))
            }
            "lr_providers_update" => {
                let provider = string(&args, "provider")?;
                commands_providers::set_provider_enabled(
                    self.app.state(),
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                    provider.into(),
                    args["enabled"]
                        .as_bool()
                        .ok_or("Expected enabled boolean")?,
                )
                .await?;
                Ok(json!({"provider": provider, "enabled": args["enabled"]}))
            }
            "lr_providers_delete" => {
                let provider = string(&args, "provider")?;
                commands_providers::remove_provider_instance(
                    self.app.state(),
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                    provider.into(),
                )
                .await?;
                Ok(json!({"deleted": provider}))
            }
            "lr_providers_check" => {
                let instance = string(&args, "provider")?;
                let provider = state
                    .provider_registry
                    .get_provider(instance)
                    .ok_or("Provider is unknown or disabled")?;
                let health = tokio::time::timeout(Duration::from_secs(20), provider.health_check())
                    .await
                    .map_err(|_| "Provider health check timed out")?;
                Ok(
                    json!({"provider": instance, "status": health.status, "latency_ms": health.latency_ms,
                    "checked_at": health.last_checked, "has_error": health.error_message.is_some()}),
                )
            }
            "lr_providers_test_model" => test_model(&state, &args).await,
            "lr_models_list" => {
                if let Some(provider) = args["provider"].as_str() {
                    if state
                        .provider_registry
                        .get_provider_type_for_instance(provider)
                        .is_none()
                    {
                        return Err("Provider not found".into());
                    }
                }
                let mut models = state.provider_registry.get_all_cached_models_instant();
                models.retain(|m| {
                    args["provider"]
                        .as_str()
                        .is_none_or(|provider| m.provider == provider)
                });
                models.sort_by(|a, b| (&a.provider, &a.id).cmp(&(&b.provider, &b.id)));
                let offset = page_offset(&args);
                let limit = page_limit(&args);
                let total = models.len();
                let next = offset.saturating_add(limit);
                let page: Vec<_> = models.into_iter().skip(offset).take(limit).collect();
                Ok(
                    json!({"models": page, "total": total, "source": "cache", "next_offset": (next < total).then_some(next)}),
                )
            }
            "lr_models_get" => {
                let provider = string(&args, "provider")?;
                let id = string(&args, "model")?;
                let model = state
                    .provider_registry
                    .get_all_cached_models_instant()
                    .into_iter()
                    .find(|model| model.provider == provider && model.id == id)
                    .ok_or("Model is not cached. Refresh its provider first.")?;
                Ok(
                    json!({"model": model, "pricing_override": state.provider_registry.effective_pricing_override(provider, id, &config.get().pricing_overrides)}),
                )
            }
            "lr_models_refresh" => {
                let instances = args["provider"].as_str().map_or_else(
                    || state.provider_registry.get_enabled_instance_names(),
                    |provider| vec![provider.to_string()],
                );
                let refreshed = futures::future::join_all(instances.into_iter().map(|provider| {
                    let registry = &state.provider_registry;
                    async move {
                        registry.invalidate_provider_cache(&provider);
                        let fetch = tokio::time::timeout(Duration::from_secs(20), registry.list_provider_models_cached(&provider)).await;
                        let summary = match fetch {
                            Ok(Ok(models)) => {
                                let fallback = registry.managed_models(&provider).is_some_and(|listing| listing.discovery_error.is_some());
                                json!({"provider": provider, "status": if fallback { "fallback" } else { "live" }, "models": models.len()})
                            }
                            Ok(Err(_)) => json!({"provider": provider, "status": "failed"}),
                            Err(_) => json!({"provider": provider, "status": "timed_out"}),
                        };
                        registry.sync_cached_models(&provider);
                        summary
                    }
                })).await;
                let _ = self.app.emit("models-changed", ());
                Ok(json!({"providers": refreshed}))
            }
            "lr_models_configure" => {
                let instance = string(&args, "provider")?;
                commands_providers::set_provider_model_settings(
                    instance.into(),
                    parse(&args["settings"])?,
                    self.app.state(),
                    self.app.state(),
                    self.app.clone(),
                )
                .await?;
                Ok(json!({"updated": instance}))
            }
            "lr_routing_strategies_list" => {
                let cfg = config.get();
                Ok(
                    json!({"strategies": cfg.strategies.iter().map(|s| strategy_view(&cfg,s)).collect::<Vec<_>>()}),
                )
            }
            "lr_routing_strategies_get" => {
                let cfg = config.get();
                Ok(strategy_view(
                    &cfg,
                    find_strategy(&cfg, string(&args, "strategy_id")?)?,
                ))
            }
            "lr_routing_strategies_manage" => {
                let updated = config
                    .try_update(|cfg| change_strategy(cfg, &args).map_err(AppError::Config))
                    .map_err(display)?;
                config.save().await.map_err(display)?;
                let _ = self.app.emit("strategies-changed", ());
                Ok(updated)
            }
            "lr_routing_profiles_list" => Ok(json!(
                commands_routing_profiles::get_routing_profiles(None, self.app.state()).await?
            )),
            "lr_routing_profiles_get" => {
                let id = string(&args, "profile_id")?;
                let profiles =
                    commands_routing_profiles::get_routing_profiles(None, self.app.state())
                        .await?
                        .profiles;
                Ok(json!(profiles
                    .iter()
                    .find(|p| p.profile.id == id)
                    .ok_or("Profile not found")?))
            }
            "lr_routing_preview" => {
                let cfg = config.get();
                let client = find_client(&cfg, string(&args, "client_id")?)?;
                let strategy = find_strategy(&cfg, &client.strategy_id)?;
                let mut effective = strategy.clone();
                if let Some(binding) = &strategy.routing_profile {
                    let profile = cfg
                        .routing_profiles
                        .iter()
                        .find(|p| p.id == binding.profile_id)
                        .ok_or("Routing profile not found")?;
                    profile.apply(&mut effective)?;
                }
                let policy = effective
                    .auto_config
                    .as_ref()
                    .and_then(|auto| auto.routing_policy.clone())
                    .unwrap_or_default();
                let result = commands_decision_routing::preview_routing_policy(
                    strategy.id.clone(),
                    Some(client.id.clone()),
                    policy,
                    String::new(),
                    None,
                    Some(false),
                    None,
                    None,
                    effective
                        .auto_config
                        .as_ref()
                        .map(lr_config::RouteSnapshot::from),
                    None,
                    self.app.state(),
                )
                .await?;
                Ok(json!(result))
            }
            "lr_firewall_get" => firewall_view(&config.get(), string(&args, "client_id")?),
            "lr_firewall_preview" | "lr_config_preview" => {
                let before = config.get();
                let mut after = before.clone();
                let (previous, proposed) = change_config(name, &mut after, &args)?;
                lr_config::validate_config(&after).map_err(display)?;
                Ok(
                    json!({"before": previous, "after": proposed, "revision": preview_revision(&before, name, &args),
                    "runtime": "Some global settings take effect at restart."}),
                )
            }
            "lr_firewall_apply" | "lr_config_apply" => {
                let expected = string(&args, "revision")?;
                let applied = config
                    .try_update(|cfg| {
                        if preview_revision(cfg, name, &args) != expected {
                            return Err(AppError::Config(
                                "Settings changed. Run preview again.".into(),
                            ));
                        }
                        let (_, after) =
                            change_config(name, cfg, &args).map_err(AppError::Config)?;
                        Ok(after)
                    })
                    .map_err(display)?;
                config.save().await.map_err(display)?;
                if name == "lr_firewall_apply" {
                    let _ = self.app.emit("clients-changed", ());
                    let _ = self.app.emit("strategies-changed", ());
                } else {
                    match args["section"].as_str() {
                        Some("model_cache") => state
                            .provider_registry
                            .update_cache_config(config.get().model_cache),
                        Some("push_relay") => state.sync_push_relay_settings(),
                        Some("secret_scanning") => {
                            state.rebuild_secret_scanner().map_err(display)?;
                        }
                        _ => {}
                    }
                }
                Ok(
                    json!({"applied": applied, "restart_may_be_required": name == "lr_config_apply"}),
                )
            }
            "lr_config_list_sections" => Ok(json!({"sections": CONFIG_SECTIONS})),
            "lr_config_schema" => {
                let section = config_section(&args)?;
                Ok(
                    json!({"section": section, "defaults": section_view(&AppConfig::default(), section),
                    "changes": "An object keyed by settings to update. For a scalar or list section use {value: ...}."}),
                )
            }
            "lr_config_get" => {
                let section = config_section(&args)?;
                let cfg = config.get();
                Ok(
                    json!({"section": section, "value": section_view(&cfg,section), "revision": revision(&cfg)}),
                )
            }
            "lr_monitor_stats" => Ok(json!(state.monitor_store.stats())),
            "lr_monitor_events_list" => monitor_list(&state, &args),
            "lr_monitor_event_get" => {
                let event = state
                    .monitor_store
                    .get(string(&args, "event_id")?)
                    .ok_or("Event expired or not found")?;
                Ok(
                    json!({"id": event.id, "sequence": event.sequence, "timestamp": event.timestamp,
                    "type": event.event_type, "client_id": event.client_id, "status": event.status,
                    "duration_ms": event.duration_ms, "session_id": event.session_id, "payload_redacted": event.payload_redacted}),
                )
            }
            "lr_monitor_clear" => {
                state.monitor_store.clear();
                Ok(json!({"cleared": true}))
            }
            "lr_intercept_get" => Ok(
                json!({"rule": state.mcp_gateway.firewall_manager.get_intercept_rule(),
                "rule_id": state.mcp_gateway.firewall_manager.owned_intercept_id(actor)}),
            ),
            "lr_intercept_start" => {
                let client_ids: Vec<String> = parse(&args["client_ids"])?;
                let categories: Vec<InterceptCategory> = parse(&args["categories"])?;
                let seconds = args["seconds"]
                    .as_u64()
                    .filter(|s| (1..=300).contains(s))
                    .ok_or("seconds must be 1–300")?;
                let cfg = config.get();
                if client_ids.is_empty()
                    || client_ids
                        .iter()
                        .any(|id| id == actor || find_client(&cfg, id).is_err())
                {
                    return Err(
                        "Select at least one existing client other than this admin client".into(),
                    );
                }
                if categories.is_empty()
                    || categories
                        .iter()
                        .any(|c| !matches!(c, InterceptCategory::Llm | InterceptCategory::Mcp))
                {
                    return Err("Select llm or mcp for this review session".into());
                }
                let rule = InterceptRule {
                    categories,
                    client_ids,
                };
                let id = state
                    .mcp_gateway
                    .firewall_manager
                    .start_owned_intercept(actor, rule, seconds)
                    .map_err(display)?;
                Ok(
                    json!({"rule_id": id, "expires_in_seconds": seconds, "event": "_localrouter_admin__intercept.changed"}),
                )
            }
            "lr_intercept_stop" => {
                state
                    .mcp_gateway
                    .firewall_manager
                    .stop_owned_intercept(actor, string(&args, "rule_id")?)
                    .map_err(display)?;
                Ok(json!({"stopped": true}))
            }
            "lr_intercept_list_pending" | "lr_intercept_get_pending" => {
                let manager = &state.mcp_gateway.firewall_manager;
                let pending = manager.owned_intercept_pending(actor, args["request_id"].as_str());
                if name == "lr_intercept_get_pending" {
                    pending
                        .into_iter()
                        .next()
                        .ok_or_else(|| "Request expired or outside your active rule".into())
                } else {
                    Ok(json!({"requests": pending}))
                }
            }
            "lr_intercept_resolve" => {
                let request_id = string(&args, "request_id")?;
                let action = match string(&args, "action")? {
                    "allow" => FirewallApprovalAction::AllowOnce,
                    "deny" => FirewallApprovalAction::Deny,
                    _ => return Err("Expected allow or deny".into()),
                };
                state
                    .mcp_gateway
                    .firewall_manager
                    .resolve_owned_intercept(actor, request_id, action)
                    .map_err(display)?;
                Ok(json!({"request_id": request_id, "resolved": true}))
            }
            "lr_mcp_servers_list" => {
                let entries: Vec<_> = state
                    .mcp_server_manager
                    .list_configs()
                    .iter()
                    .map(|server| mcp_view(&state, server))
                    .collect();
                Ok(json!({"servers": entries, "built_in": [{"id": ID, "name": "LocalRouter"}]}))
            }
            "lr_mcp_servers_get" | "lr_mcp_servers_capabilities" => {
                let id = string(&args, "server_id")?;
                let server = state
                    .mcp_server_manager
                    .get_config(id)
                    .ok_or("MCP server not found")?;
                if name == "lr_mcp_servers_get" {
                    Ok(mcp_view(&state, &server))
                } else {
                    Ok(json!({"server_id": id, "remembered": server.capability_snapshot}))
                }
            }
            "lr_mcp_servers_manage" => {
                if args["action"] == "create" {
                    let url = string(&args, "url")?;
                    validate_provider_url(url)?;
                    let created = commands_mcp::create_mcp_server(
                        string(&args, "name")?.into(),
                        "http_sse".into(),
                        json!({"type":"http_sse","url":url}),
                        None,
                        self.app.state(),
                        self.app.state(),
                        self.app.state(),
                        self.app.clone(),
                    )
                    .await?;
                    return Ok(json!({"server_id": created.id, "name": created.name}));
                }
                let id = string(&args, "server_id")?.to_owned();
                match string(&args, "action")? {
                    "start" => {
                        commands_mcp::start_mcp_server(
                            id.clone(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    "stop" => {
                        commands_mcp::stop_mcp_server(
                            id.clone(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    "delete" => {
                        commands_mcp::delete_mcp_server(
                            id.clone(),
                            self.app.state(),
                            self.app.state(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    "rename" => {
                        commands_mcp::update_mcp_server(
                            id.clone(),
                            json!({"name":string(&args,"name")?}),
                            self.app.state(),
                            self.app.state(),
                            self.app.clone(),
                        )
                        .await?
                    }
                    _ => return Err("Expected create, start, stop, rename or delete".into()),
                }
                Ok(json!({"server_id": id, "action": args["action"]}))
            }
            _ => Err("Unknown management tool".into()),
        }
    }

    async fn manage_profile(&self, args: &Value) -> Result<Value, String> {
        let action = string(args, "action")?;
        let strategy = optional(args, "strategy_id")?;
        let profile: Option<String> = optional(args, "profile_id")?;
        let settings = &args["settings"];
        let result = match action {
            "create" => {
                commands_routing_profiles::create_routing_profile(
                    string(args, "name")?.into(),
                    strategy,
                    profile,
                    optional(settings, "fingerprint")?,
                    self.app.state(),
                    self.app.clone(),
                )
                .await?
            }
            "bind" | "unbind" => {
                commands_routing_profiles::bind_routing_profile(
                    strategy.ok_or("Expected strategy_id")?,
                    profile,
                    string(settings, "strategy_fingerprint")?.into(),
                    optional(settings, "profile_fingerprint")?,
                    self.app.state(),
                    self.app.clone(),
                )
                .await?
            }
            _ => {
                let mut operation = settings
                    .get("operation")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if !operation.is_object() {
                    return Err("Expected an operation object".into());
                }
                operation["action"] = json!(action);
                commands_routing_profiles::mutate_routing_profile(
                    profile.ok_or("Expected profile_id")?,
                    string(settings, "fingerprint")?.into(),
                    parse(&operation)?,
                    self.app.state(),
                    self.app.clone(),
                )
                .await?
            }
        };
        Ok(json!(result))
    }

    async fn manage_local_model(&self, args: &Value) -> Result<Value, String> {
        let action = string(args, "action")?;
        match action {
            "download" => {
                let id = commands_local_models::local_models_download_start(
                    string(args, "repo")?.into(),
                    optional(args, "revision")?,
                    parse(&args["files"])?,
                    self.app.state(),
                )
                .await?;
                return Ok(json!({"download_id": id}));
            }
            "import" => {
                let model = commands_local_models::local_models_import(
                    string(args, "path")?.into(),
                    self.app.state(),
                )
                .await?;
                return Ok(json!({"model": model}));
            }
            "load" => {
                commands_local_models::local_models_load(
                    string(args, "provider")?.into(),
                    string(args, "model")?.into(),
                    self.app.state(),
                )
                .await?
            }
            "unload" => {
                commands_local_models::local_models_unload(
                    string(args, "provider")?.into(),
                    string(args, "model")?.into(),
                    self.app.state(),
                )
                .await?
            }
            "pause" => {
                commands_local_models::local_models_download_pause(
                    string(args, "id")?.into(),
                    self.app.state(),
                )
                .await?
            }
            "resume" => {
                commands_local_models::local_models_download_resume(
                    string(args, "id")?.into(),
                    self.app.state(),
                )
                .await?
            }
            "cancel" => {
                commands_local_models::local_models_download_cancel(
                    string(args, "id")?.into(),
                    self.app.state(),
                )
                .await?
            }
            "remove" => {
                commands_local_models::local_models_remove(
                    string(args, "id")?.into(),
                    false,
                    self.app.state(),
                    self.app.state(),
                )
                .await?
            }
            _ => return Err("Unknown local model operation".into()),
        }
        Ok(json!({"action": action, "completed": true}))
    }
}

fn display(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args[name]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Expected a nonempty {name}"))
}

fn parse<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone())
        .map_err(|_| "Arguments do not match the expected settings types".into())
}

fn optional<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>, String> {
    args.get(name).map(parse).transpose()
}

fn validate_args(tool: &Tool, args: &Value) -> Result<(), String> {
    let object = args.as_object().ok_or("Expected an arguments object")?;
    if args.to_string().len() > 32_768 {
        return Err("Arguments exceed 32 KiB".into());
    }
    let definition = super::definition(tool.name, tool.description, tool.arguments);
    let fields = definition.input_schema["properties"]
        .as_object()
        .expect("tool properties");
    for name in object.keys() {
        if !fields.contains_key(name) {
            return Err(format!("Unknown argument {name}"));
        }
    }
    for field in tool.arguments.split_whitespace() {
        let (name, kind) = field.split_once(':').expect("argument descriptor");
        let valid = match (object.get(name), kind.trim_end_matches('?')) {
            (None, _) => kind.ends_with('?'),
            (Some(value), "s") => value.as_str().is_some_and(|s| !s.trim().is_empty()),
            (Some(value), "i") => value.as_u64().is_some_and(|n| n > 0),
            (Some(value), "u") => value.as_u64().is_some(),
            (Some(value), "o") => value.is_object(),
            (Some(value), "b") => value.is_boolean(),
            (Some(value), "a") => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid or missing argument {name}"));
        }
    }
    Ok(())
}

fn summarize_operation(name: &str, args: &Value) -> Value {
    let mut review = json!({"operation": name});
    for field in [
        "name",
        "client_id",
        "client_ids",
        "provider",
        "provider_type",
        "model",
        "strategy_id",
        "server_id",
        "section",
        "request_id",
        "rule_id",
        "action",
        "categories",
        "seconds",
        "enabled",
        "profile_id",
        "account_id",
        "path",
        "repo",
        "files",
        "id",
        "url",
    ] {
        if let Some(value) = args.get(field) {
            review[field] = value.clone();
        }
    }
    for field in ["changes", "settings"] {
        if let Some(values) = args[field].as_object() {
            review[format!("{field}_keys")] = json!(values.keys().collect::<Vec<_>>());
        }
    }
    review
}

fn find_client<'a>(config: &'a AppConfig, id: &str) -> Result<&'a Client, String> {
    config
        .clients
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "Client not found".into())
}

fn find_client_mut<'a>(config: &'a mut AppConfig, id: &str) -> Result<&'a mut Client, AppError> {
    config
        .clients
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| AppError::Config("Client not found".into()))
}

fn find_strategy<'a>(config: &'a AppConfig, id: &str) -> Result<&'a Strategy, String> {
    config
        .strategies
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "Strategy not found".into())
}

fn client_view(client: &Client) -> Value {
    json!({"client_id": client.id, "name": client.name, "enabled": client.enabled,
        "llm_mode": client.llm_mode, "mcp_mode": client.mcp_mode, "strategy_id": client.strategy_id,
        "template_id": client.template_id})
}

fn strategy_view(config: &AppConfig, strategy: &Strategy) -> Value {
    json!({"strategy": strategy,
        "clients": config.clients.iter().filter(|c| c.strategy_id == strategy.id).map(client_view).collect::<Vec<_>>()})
}

fn mcp_view(state: &AppState, server: &lr_config::McpServerConfig) -> Value {
    json!({"server_id": server.id, "name": server.name, "transport": server.transport,
        "enabled": server.enabled, "running": state.mcp_server_manager.is_running(&server.id)})
}

fn change_strategy(config: &mut AppConfig, args: &Value) -> Result<Value, String> {
    let action = string(args, "action")?;
    let changes = args.get("changes").cloned().unwrap_or_else(|| json!({}));
    match action {
        "create" => {
            let strategy = patched(
                &Strategy::new(string(&changes, "name")?.into()),
                &changes,
                STRATEGY_FIELDS,
            )?;
            let view = strategy_view(config, &strategy);
            config.strategies.push(strategy);
            Ok(view)
        }
        "update" => {
            let id = string(args, "strategy_id")?;
            let strategy = patched(find_strategy(config, id)?, &changes, STRATEGY_FIELDS)?;
            config
                .strategies
                .iter_mut()
                .filter(|s| s.id == id)
                .for_each(|s| *s = strategy.clone());
            Ok(strategy_view(config, &strategy))
        }
        "delete" => {
            let id = string(args, "strategy_id")?;
            find_strategy(config, id)?;
            if config.clients.iter().any(|c| c.strategy_id == id) {
                return Err("Reassign clients before deleting their strategy".into());
            }
            config.strategies.retain(|s| s.id != id);
            Ok(json!({"deleted": id}))
        }
        _ => Err("Expected create, update, or delete".into()),
    }
}

fn patch(value: &mut Value, changes: &Value) {
    if let (Some(current), Some(changes)) = (value.as_object_mut(), changes.as_object()) {
        for (key, update) in changes {
            if update.is_null() {
                current.remove(key);
            } else {
                patch(current.entry(key.clone()).or_insert(Value::Null), update);
            }
        }
    } else {
        *value = changes.clone();
    }
}

fn patched<T: Serialize + DeserializeOwned>(
    current: &T,
    changes: &Value,
    allowed: &[&str],
) -> Result<T, String> {
    let fields = changes.as_object().ok_or("Expected an object of changes")?;
    if let Some(field) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Field {field} is unavailable for this operation"));
    }
    let mut value = serde_json::to_value(current).map_err(display)?;
    patch(&mut value, changes);
    parse(&value)
}

fn config_section(args: &Value) -> Result<&str, String> {
    let section = string(args, "section")?;
    CONFIG_SECTIONS
        .contains(&section)
        .then_some(section)
        .ok_or_else(|| "Unknown configuration section".into())
}

fn section_view(config: &AppConfig, section: &str) -> Value {
    match section {
        "context_management" => json!(config.context_management),
        "push_relay" => json!({
            "default_permission": config.push_relay.default_permission,
            "channel_messages_per_minute": config.push_relay.channel_messages_per_minute,
            "history_retention_hours": config.push_relay.history_retention_hours,
            "history_max_events": config.push_relay.history_max_events,
        }),
        _ => json!(config)[section].clone(),
    }
}

fn revision(config: &AppConfig) -> String {
    digest(&json!(config))
}

fn digest(value: &Value) -> String {
    lr_config::content_hash(value.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn preview_revision(config: &AppConfig, name: &str, args: &Value) -> String {
    let scope = if name.starts_with("lr_firewall") {
        "firewall"
    } else {
        "config"
    };
    digest(&json!([
        revision(config),
        scope,
        args["section"],
        args["client_id"],
        args["changes"]
    ]))
}

fn firewall_view(cfg: &AppConfig, id: &str) -> Result<Value, String> {
    let client = find_client(cfg, id)?;
    let strategy = find_strategy(cfg, &client.strategy_id)?;
    Ok(json!({"client_id": id, "strategy_id": strategy.id,
        "mcp_permissions": client.mcp_permissions, "skills_permissions": client.skills_permissions,
        "marketplace_permission": client.marketplace_permission, "coding_agent_permission": client.coding_agent_permission,
        "push_permissions": client.push_permissions, "model_permissions": strategy.model_permissions,
        "affected_clients": cfg.clients.iter().filter(|c| c.strategy_id == strategy.id).map(client_view).collect::<Vec<_>>() }))
}

fn change_config(name: &str, cfg: &mut AppConfig, args: &Value) -> Result<(Value, Value), String> {
    if name.starts_with("lr_firewall") {
        let id = string(args, "client_id")?;
        let previous = firewall_view(cfg, id)?;
        let mut changes = args["changes"]
            .as_object()
            .cloned()
            .ok_or("Expected firewall rules")?;
        if let Some(field) = changes
            .keys()
            .find(|key| !FIREWALL_FIELDS.contains(&key.as_str()))
        {
            return Err(format!("Unknown firewall rule {field}"));
        }
        let model_changes = changes.remove("model_permissions");
        let client = find_client_mut(cfg, id).map_err(display)?;
        *client = patched(client, &Value::Object(changes), FIREWALL_FIELDS)?;
        if let Some(changes) = model_changes {
            let strategy_id = client.strategy_id.clone();
            let strategy = cfg
                .strategies
                .iter_mut()
                .find(|s| s.id == strategy_id)
                .ok_or("Strategy not found")?;
            let mut rules = json!(strategy.model_permissions);
            patch(&mut rules, &changes);
            strategy.model_permissions = parse(&rules)?;
        }
        Ok((previous, firewall_view(cfg, id)?))
    } else {
        let section = config_section(args)?;
        let previous = section_view(cfg, section);
        let defaults = section_view(&AppConfig::default(), section);
        let fields = args["changes"]
            .as_object()
            .ok_or("Expected settings changes")?;
        let mut value = previous.clone();
        if matches!(section, "roots" | "start_on_boot" | "pricing_overrides") {
            if fields.len() != 1 || !fields.contains_key("value") {
                return Err("Send an object with a value field to replace this setting".into());
            }
            value = fields["value"].clone();
        } else {
            let optional_fields: &[&str] = match section {
                "memory" => &["compaction_model"],
                "context_management" => &["embedding"],
                _ => &[],
            };
            if let Some(key) = fields.keys().find(|key| {
                previous.get(*key).is_none()
                    && defaults.get(*key).is_none()
                    && !optional_fields.contains(&key.as_str())
            }) {
                return Err(format!("Unknown field {key} in {section}"));
            }
            patch(&mut value, &args["changes"]);
        }
        let mut next = json!(cfg);
        next[section] = value;
        let updated: AppConfig = parse(&next)?;
        updated.push_relay.validate()?;
        *cfg = updated;
        Ok((previous, section_view(cfg, section)))
    }
}

fn page_limit(args: &Value) -> usize {
    args["limit"].as_u64().unwrap_or(25).clamp(1, 100) as usize
}

fn page_offset(args: &Value) -> usize {
    args["offset"]
        .as_u64()
        .unwrap_or_default()
        .min(usize::MAX as u64) as usize
}

fn monitor_list(state: &AppState, args: &Value) -> Result<Value, String> {
    let filter = lr_monitor::MonitorEventFilter {
        client_id: optional(args, "client_id")?,
        ..Default::default()
    };
    let before = args["before_sequence"].as_u64().unwrap_or(u64::MAX);
    let limit = page_limit(args);
    let mut page = Vec::new();
    let mut offset = 0;
    let mut next = None;
    loop {
        let batch = state.monitor_store.list(offset, 100, Some(&filter));
        for event in batch
            .events
            .into_iter()
            .filter(|event| event.sequence < before)
        {
            if page.len() == limit {
                next = page
                    .last()
                    .and_then(|value: &Value| value["sequence"].as_u64());
                break;
            }
            page.push(
                json!({"id": event.id, "sequence": event.sequence, "timestamp": event.timestamp,
                "type": event.event_type, "client_id": event.client_id, "status": event.status,
                "duration_ms": event.duration_ms, "session_id": event.session_id}),
            );
        }
        offset += 100;
        if next.is_some() || offset >= batch.total {
            break;
        }
    }
    Ok(json!({"events": page, "before_sequence": next}))
}

fn validate_provider_url(value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid provider URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an HTTP(S) provider endpoint without credentials, query, or fragment".into(),
        );
    }
    Ok(())
}

async fn test_model(state: &AppState, args: &Value) -> Result<Value, String> {
    let instance = string(args, "provider")?;
    let model = string(args, "model")?;
    let provider = state
        .provider_registry
        .get_provider(instance)
        .ok_or("Provider is unknown or disabled")?;
    if !provider.supports_chat() || state.provider_registry.is_model_disabled(instance, model) {
        return Err("Choose an enabled chat model".into());
    }
    let request = parse(
        &json!({"model": model, "messages": [{"role": "user", "content": "Reply with OK."}], "max_tokens": 16, "store": false}),
    )?;
    let started = Instant::now();
    let completion = tokio::time::timeout(Duration::from_secs(25), provider.complete(request))
        .await
        .map_err(|_| "Model test timed out")?
        .map_err(|_| "Model test failed. Check the provider in LocalRouter.")?;
    let pricing = state.provider_registry.get_pricing(instance, model).await;
    Ok(
        json!({"provider": instance, "model": completion.model, "ok": !completion.choices.is_empty(),
        "latency_ms": started.elapsed().as_millis(), "usage": completion.usage, "estimated_cost_usd": pricing.cost(&completion.usage)}),
    )
}
