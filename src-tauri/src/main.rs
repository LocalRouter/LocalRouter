// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod launcher;
mod ui;
mod updater;

use std::sync::Arc;

use tauri::{Emitter, Listener, Manager};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Re-exported crate aliases from lib.rs
use localrouter::{
    api_keys, clients, config, marketplace, mcp, monitoring, oauth_browser, oauth_clients,
    providers, routellm, router, server, skills, utils,
};

use lr_providers::factory::{
    AnthropicProviderFactory, CerebrasProviderFactory, CohereProviderFactory,
    DeepInfraProviderFactory, GPT4AllProviderFactory, GeminiProviderFactory,
    GitHubCopilotProviderFactory, GroqProviderFactory, JanProviderFactory, LMStudioProviderFactory,
    LlamaCppProviderFactory, LocalAIProviderFactory, MistralProviderFactory, OllamaProviderFactory,
    OpenAICodexProviderFactory, OpenAICompatibleProviderFactory, OpenAIProviderFactory,
    OpenRouterProviderFactory, PerplexityProviderFactory, TogetherAIProviderFactory,
    XAIProviderFactory,
};
use lr_providers::registry::ProviderRegistry;
use lr_server::ServerManager;

/// CompactionLlm implementation that uses the Router to call an LLM for summarization.
struct RouterCompactionLlm {
    router: Arc<router::Router>,
}

#[async_trait::async_trait]
impl lr_memory::CompactionLlm for RouterCompactionLlm {
    async fn summarize(
        &self,
        model: &str,
        transcript: &str,
        thinking: bool,
    ) -> Result<lr_memory::compaction::CompactionResult, String> {
        use lr_providers::{ChatMessage, ChatMessageContent, CompletionRequest};

        let request = CompletionRequest {
            model: model.to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: ChatMessageContent::Text(
                        lr_memory::compaction::system_prompt().to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: ChatMessageContent::Text(transcript.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
            ],
            temperature: Some(0.0),
            max_tokens: Some(4096),
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
            reasoning_effort: if thinking {
                None
            } else {
                Some("none".to_string())
            },
            pre_computed_routing: None,
        };

        // Serialize request for monitor event observability
        let request_body = serde_json::to_value(&request).ok();

        let response = self
            .router
            .complete("memory-service", request)
            .await
            .map_err(|e| format!("LLM compaction failed: {}", e))?;

        // Serialize response for monitor event observability
        let response_body = serde_json::to_value(&response).ok();

        let choice = response
            .choices
            .first()
            .ok_or_else(|| "Empty LLM response: no choices".to_string())?;

        let summary = choice.message.content.as_text();
        let reasoning_tokens = response
            .usage
            .completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens.or(d.thinking_tokens));

        Ok(lr_memory::compaction::CompactionResult {
            summary,
            input_tokens: response.usage.prompt_tokens,
            output_tokens: response.usage.completion_tokens,
            reasoning_tokens,
            finish_reason: choice.finish_reason.clone(),
            request_body,
            response_body,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI arguments
    let args = cli::Cli::parse_args();

    // Initialize logging (always to stderr for bridge mode)
    init_logging();

    // Branch based on mode
    if args.mcp_bridge {
        run_bridge_mode(args.client_id).await
    } else {
        run_gui_mode().await
    }
}

/// Initialize logging to stderr
///
/// In bridge mode, stdout is reserved for JSON-RPC responses,
/// so all logging must go to stderr.
fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "localrouter=info,lr_mcp=info,lr_server=info,lr_providers=info,lr_clients=info,lr_router=info".into()),
        )
        .with(
            tracing_subscriber::fmt::layer().with_writer(std::io::stderr), // Always to stderr
        )
        .init();
}

/// Run in MCP bridge mode (STDIO ↔ HTTP proxy)
///
/// This is a lightweight mode that reads JSON-RPC requests from stdin,
/// forwards them to the running LocalRouter HTTP server, and writes
/// responses back to stdout.
///
/// # Arguments
/// * `client_id` - Optional client ID (auto-detects if None)
///
/// # Returns
/// Ok on clean shutdown, Err on fatal errors
async fn run_bridge_mode(client_id: Option<String>) -> anyhow::Result<()> {
    eprintln!("==========================================================");
    eprintln!("LocalRouter - MCP Bridge Mode");
    eprintln!("==========================================================");
    eprintln!();
    eprintln!("Connecting to LocalRouter server at http://localhost:3625");
    eprintln!("Make sure the LocalRouter GUI is running!");
    eprintln!();

    // Create and run bridge (loads config for client secret only)
    let bridge = mcp::StdioBridge::new(client_id, None).await.map_err(|e| {
        eprintln!("ERROR: Failed to initialize bridge: {}", e);
        eprintln!();
        eprintln!("Common issues:");
        eprintln!("  - LocalRouter GUI not running (start it first)");
        eprintln!("  - Client not configured in config.yaml");
        eprintln!("  - Client secret not found (run GUI once)");
        eprintln!("  - LOCALROUTER_CLIENT_SECRET not set (if using env var)");
        eprintln!();
        e
    })?;

    eprintln!("Bridge ready! Forwarding JSON-RPC requests...");
    eprintln!("==========================================================");
    eprintln!();

    bridge.run().await.map_err(|e| {
        eprintln!("ERROR: Bridge stopped: {}", e);
        e.into()
    })
}

/// Run in GUI mode (full desktop application)
///
/// This is the default mode that starts the HTTP server, managers,
/// and Tauri desktop window.
async fn run_gui_mode() -> anyhow::Result<()> {
    info!("Starting LocalRouter...");

    // Log configuration directory
    let config_dir =
        lr_utils::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("unknown"));
    #[cfg(debug_assertions)]
    info!("Running in DEVELOPMENT mode");
    #[cfg(not(debug_assertions))]
    info!("Running in PRODUCTION mode");
    info!("Configuration directory: {}", config_dir.display());

    // Initialize managers
    let mut config_manager = config::ConfigManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::ConfigManager::new(
            config::AppConfig::default(),
            lr_utils::paths::config_file().unwrap(),
        )
    });

    // Initialize unified client manager
    // Replaces both API key manager and OAuth client manager
    // Client secrets are stored in OS keychain, only metadata in config
    let client_manager = {
        let config = config_manager.get();
        Arc::new(clients::ClientManager::new(config.clients.clone()))
    };

    // Register client sync callback to keep ClientManager in sync with config
    // This prevents bugs where config changes aren't reflected in ClientManager
    {
        let client_manager_for_sync = client_manager.clone();
        config_manager.set_client_sync_callback(std::sync::Arc::new(move |clients| {
            client_manager_for_sync.sync_clients(clients);
        }));
    }

    // Initialize OAuth token store for short-lived access tokens
    // Tokens are stored in-memory only (1 hour expiry)
    let token_store = Arc::new(clients::TokenStore::new());

    // Initialize OAuth client manager for MCP server authentication
    let oauth_client_manager = {
        let config = config_manager.get();
        Arc::new(oauth_clients::OAuthClientManager::new(
            config.oauth_clients.clone(),
        ))
    };

    // Initialize keychain for secure storage
    let keychain = api_keys::keychain_trait::CachedKeychain::auto().unwrap_or_else(|e| {
        error!("Failed to initialize keychain: {}", e);
        api_keys::keychain_trait::CachedKeychain::system()
    });

    // Initialize MCP server manager
    let mcp_server_manager = {
        let config = config_manager.get();
        let manager = Arc::new(mcp::McpServerManager::new());
        manager.load_configs(config.mcp_servers.clone());
        manager
    };

    // Initialize MCP OAuth managers
    let mcp_oauth_manager = Arc::new(mcp::oauth::McpOAuthManager::new());
    let mcp_oauth_browser_manager = Arc::new(mcp::oauth_browser::McpOAuthBrowserManager::new(
        keychain.clone(),
        mcp_oauth_manager.clone(),
    ));

    // Initialize unified OAuth flow manager for inline OAuth flows
    let oauth_flow_manager = Arc::new(oauth_browser::OAuthFlowManager::new(keychain.clone()));

    // Initialize provider registry
    info!("Initializing provider registry...");
    let provider_registry = Arc::new(ProviderRegistry::new());

    // Register provider factories
    info!("Registering provider factories...");
    provider_registry.register_factory(Arc::new(OllamaProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAIProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    provider_registry.register_factory(Arc::new(AnthropicProviderFactory));
    provider_registry.register_factory(Arc::new(GeminiProviderFactory));
    provider_registry.register_factory(Arc::new(OpenRouterProviderFactory));
    provider_registry.register_factory(Arc::new(GroqProviderFactory));
    provider_registry.register_factory(Arc::new(MistralProviderFactory));
    provider_registry.register_factory(Arc::new(CohereProviderFactory));
    provider_registry.register_factory(Arc::new(TogetherAIProviderFactory));
    provider_registry.register_factory(Arc::new(PerplexityProviderFactory));
    provider_registry.register_factory(Arc::new(DeepInfraProviderFactory));
    provider_registry.register_factory(Arc::new(CerebrasProviderFactory));
    provider_registry.register_factory(Arc::new(XAIProviderFactory));
    provider_registry.register_factory(Arc::new(LMStudioProviderFactory));
    provider_registry.register_factory(Arc::new(JanProviderFactory));
    provider_registry.register_factory(Arc::new(GPT4AllProviderFactory));
    provider_registry.register_factory(Arc::new(LocalAIProviderFactory));
    provider_registry.register_factory(Arc::new(LlamaCppProviderFactory));
    // Subscription providers (OAuth-based)
    provider_registry.register_factory(Arc::new(GitHubCopilotProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICodexProviderFactory));
    info!(
        "Registered {} provider factories",
        provider_registry.list_provider_types().len()
    );

    // On first startup, discover local LLM providers (Ollama, LM Studio)
    {
        let config = config_manager.get();
        if !config.setup_wizard_shown && config.providers.is_empty() {
            info!("First startup detected, discovering local LLM providers...");
            let discovered = providers::factory::discover_local_providers().await;

            if !discovered.is_empty() {
                info!("Discovered {} local provider(s)", discovered.len());
                if let Err(e) = config_manager.update(|cfg| {
                    for provider in &discovered {
                        let provider_config = match provider.provider_type.as_str() {
                            "ollama" => config::ProviderConfig::default_ollama(),
                            "lmstudio" => config::ProviderConfig::default_lmstudio(),
                            "jan" => config::ProviderConfig::default_jan(),
                            "gpt4all" => config::ProviderConfig::default_gpt4all(),
                            _ => continue,
                        };
                        info!(
                            "Auto-configuring discovered provider: {}",
                            provider.instance_name
                        );
                        cfg.providers.push(provider_config);
                    }
                }) {
                    warn!("Failed to save discovered providers to config: {}", e);
                }
            } else {
                info!("No local LLM providers discovered");
            }
        }
    }

    // Load provider instances from configuration
    info!("Loading provider instances from configuration...");
    let providers = config_manager.get().providers;
    for provider_config in providers {
        let provider_type = match provider_config.provider_type {
            config::ProviderType::Ollama => "ollama",
            config::ProviderType::LMStudio => "lmstudio",
            config::ProviderType::OpenAI => "openai",
            config::ProviderType::Anthropic => "anthropic",
            config::ProviderType::Gemini => "gemini",
            config::ProviderType::OpenRouter => "openrouter",
            config::ProviderType::Groq => "groq",
            config::ProviderType::Mistral => "mistral",
            config::ProviderType::Cohere => "cohere",
            config::ProviderType::TogetherAI => "togetherai",
            config::ProviderType::Perplexity => "perplexity",
            config::ProviderType::DeepInfra => "deepinfra",
            config::ProviderType::Cerebras => "cerebras",
            config::ProviderType::XAI => "xai",
            config::ProviderType::Jan => "jan",
            config::ProviderType::GPT4All => "gpt4all",
            config::ProviderType::LocalAI => "localai",
            config::ProviderType::LlamaCpp => "llamacpp",
            config::ProviderType::Custom => "openai_compatible",
        };

        // Convert provider_config JSON to HashMap
        let mut config_map = std::collections::HashMap::new();
        if let Some(provider_cfg) = provider_config.provider_config {
            if let Some(obj) = provider_cfg.as_object() {
                for (key, value) in obj {
                    if let Some(value_str) = value.as_str() {
                        config_map.insert(key.clone(), value_str.to_string());
                    } else {
                        config_map.insert(key.clone(), value.to_string());
                    }
                }
            }
        }

        // Inject api_key from keychain if not already in config_map
        // (post-migration: key in keychain only; legacy: key still in JSON)
        if !config_map.contains_key("api_key") {
            match lr_providers::key_storage::get_provider_key(&provider_config.name) {
                Ok(Some(api_key)) => {
                    config_map.insert("api_key".to_string(), api_key);
                }
                Ok(None) => {} // No key — fine for local providers (Ollama, LMStudio, etc.)
                Err(e) => {
                    tracing::warn!(
                        "Failed to retrieve API key for provider '{}' from keychain: {}",
                        provider_config.name,
                        e
                    );
                }
            }
        }

        // Create the provider instance
        if let Err(e) = provider_registry
            .create_provider(
                provider_config.name.clone(),
                provider_type.to_string(),
                config_map,
            )
            .await
        {
            tracing::warn!("Failed to load provider '{}': {}", provider_config.name, e);
            continue;
        }

        // Set enabled state
        if let Err(e) =
            provider_registry.set_provider_enabled(&provider_config.name, provider_config.enabled)
        {
            tracing::warn!(
                "Failed to set provider '{}' enabled state: {}",
                provider_config.name,
                e
            );
        }
    }
    info!(
        "Loaded {} provider instances",
        config_manager.get().providers.len()
    );

    // Initialize OAuth manager for subscription-based providers
    info!("Initializing OAuth manager...");
    let oauth_storage_path = lr_utils::paths::config_dir()
        .expect("Failed to get config directory")
        .join("oauth_credentials.json");
    let oauth_storage = Arc::new(
        providers::oauth::storage::OAuthStorage::new(oauth_storage_path)
            .await
            .expect("Failed to initialize OAuth storage"),
    );
    let oauth_manager = Arc::new(providers::oauth::OAuthManager::new(oauth_storage));

    // Register OAuth providers
    info!("Registering OAuth providers...");
    oauth_manager.register_provider(Arc::new(
        providers::oauth::github_copilot::GitHubCopilotOAuthProvider::new(),
    ));
    oauth_manager.register_provider(Arc::new(
        providers::oauth::openai_codex::OpenAICodexOAuthProvider::new(keychain.clone()),
    ));
    oauth_manager.register_provider(Arc::new(
        providers::oauth::anthropic_claude::AnthropicClaudeOAuthProvider::new(keychain.clone()),
    ));
    info!("Registered 3 OAuth providers");

    // Initialize rate limiter
    info!("Initializing rate limiter...");
    let rate_limiter = Arc::new(router::RateLimiterManager::new(None));

    // Initialize metrics collector
    info!("Initializing metrics collector...");
    let metrics_db_path = lr_utils::paths::config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("metrics.db");
    let metrics_db = Arc::new(
        monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap_or_else(|e| {
            tracing::error!("Failed to initialize metrics database: {}", e);
            panic!("Metrics database initialization failed");
        }),
    );
    let metrics_collector = Arc::new(monitoring::metrics::MetricsCollector::new(metrics_db));

    // Initialize RouteLLM intelligent routing service
    info!("Initializing RouteLLM service...");
    let routellm_service = {
        let config = config_manager.get();
        let idle_timeout = config.routellm_settings.idle_timeout_secs;

        match routellm::RouteLLMService::new_with_defaults(idle_timeout) {
            Ok(service) => {
                let service_arc = Arc::new(service);
                // Start auto-unload background task
                tokio::spawn(service_arc.clone().start_auto_unload_task());
                info!(
                    "RouteLLM service initialized with idle timeout: {}s",
                    idle_timeout
                );
                Some(service_arc)
            }
            Err(e) => {
                info!("RouteLLM service not initialized: {}", e);
                None
            }
        }
    };

    // Initialize free tier manager
    info!("Initializing free tier manager...");
    let free_tier_persist_path: Option<std::path::PathBuf> = lr_utils::paths::config_dir()
        .ok()
        .map(|d| d.join("free_tier_state.json"));
    let free_tier_manager = Arc::new(if let Some(ref path) = free_tier_persist_path {
        lr_router::FreeTierManager::load(path)
    } else {
        lr_router::FreeTierManager::new(free_tier_persist_path.clone())
    });

    // Initialize shared health cache (used by both router and server)
    let health_cache = Arc::new(providers::health_cache::HealthCacheManager::new());

    // Initialize router
    info!("Initializing router...");
    let config_manager_arc = Arc::new(config_manager.clone());
    let mut app_router = router::Router::new(
        config_manager_arc.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        free_tier_manager.clone(),
    )
    .with_health_cache(health_cache.clone());

    // Add RouteLLM service to router
    app_router = app_router.with_routellm(routellm_service);
    let app_router = Arc::new(app_router);

    // Initialize server manager and start server
    info!("Initializing web server...");
    let server_manager = Arc::new(ServerManager::new());

    // Get server config from configuration
    let server_config = {
        let config = config_manager.get();
        server::ServerConfig {
            host: config.server.host.clone(),
            port: config.server.port,
            enable_cors: config.server.enable_cors,
        }
    };

    // Start the server (tray_graph_manager will be added later in setup())
    server_manager
        .start(
            server_config,
            lr_server::manager::ServerDependencies {
                router: app_router.clone(),
                mcp_server_manager: mcp_server_manager.clone(),
                rate_limiter: rate_limiter.clone(),
                provider_registry: provider_registry.clone(),
                config_manager: config_manager_arc.clone(),
                client_manager: client_manager.clone(),
                token_store: token_store.clone(),
                metrics_collector: metrics_collector.clone(),
                health_cache: Some(health_cache.clone()),
            },
        )
        .await?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            info!("Tauri app initialized");

            // Set app handle on config manager for event emission
            config_manager.set_app_handle(app.handle().clone());

            // File watcher disabled - it was causing duplicate config-changed events
            // when saving from the app. The update() method already emits events.
            // TODO: Re-enable with proper suppression if external file editing is needed
            // let watcher = config_manager.start_watching().map_err(|e| {
            //     tracing::error!("Failed to start config file watcher: {}", e);
            //     e
            // })?;
            // app.manage(watcher);

            // Store managers
            app.manage(config_manager.clone());
            app.manage(client_manager.clone());
            app.manage(token_store.clone());
            app.manage(oauth_client_manager.clone());
            app.manage(mcp_server_manager.clone());
            app.manage(mcp_oauth_manager.clone());
            app.manage(mcp_oauth_browser_manager.clone());
            app.manage(oauth_flow_manager.clone());
            app.manage(provider_registry.clone());
            app.manage(server_manager.clone());
            app.manage(app_router.clone());
            app.manage(rate_limiter.clone());
            app.manage(free_tier_manager.clone());
            app.manage(oauth_manager.clone());
            app.manage(metrics_collector.clone());

            // Initialize skill manager and script executor
            let mut skill_manager = skills::SkillManager::new();
            skill_manager.set_app_handle(app.handle().clone());
            let skills_config = config_manager.get().skills.clone();
            skill_manager.initial_scan(&skills_config.paths, &skills_config.disabled_skills);
            skill_manager.start_cleanup_task();
            let skill_manager = Arc::new(skill_manager);
            let script_executor = Arc::new(skills::executor::ScriptExecutor::new());

            // Start file watcher for skill sources
            let skill_manager_for_watcher = skill_manager.clone();
            let config_manager_for_watcher = config_manager.clone();
            let watcher_paths = skills_config.paths.clone();
            match skills::SkillWatcher::start(
                watcher_paths,
                Arc::new(move |_affected_paths| {
                    let config = config_manager_for_watcher.get();
                    skill_manager_for_watcher
                        .rescan(&config.skills.paths, &config.skills.disabled_skills);
                }),
            ) {
                Ok(watcher) => {
                    app.manage(Arc::new(watcher));
                    info!("Skills file watcher started");
                }
                Err(e) => {
                    warn!("Failed to start skills file watcher: {}", e);
                }
            }

            app.manage(skill_manager.clone());
            app.manage(script_executor.clone());
            info!("Skills system initialized");

            // Initialize marketplace service (always created, checks enabled state internally)
            let marketplace_service: Option<Arc<marketplace::MarketplaceService>> = {
                let config = config_manager.get();
                let data_dir =
                    lr_utils::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                let service =
                    marketplace::MarketplaceService::new(config.marketplace.clone(), data_dir);
                info!(
                    "Marketplace service initialized (mcp_enabled: {}, skills_enabled: {})",
                    config.marketplace.mcp_enabled, config.marketplace.skills_enabled
                );
                Some(Arc::new(service))
            };
            app.manage(marketplace_service.clone());

            // Get AppState from server manager and manage it for Tauri commands
            if let Some(app_state) = server_manager.get_state() {
                info!("Managing AppState for Tauri commands");

                // Obtain context management config for virtual servers
                let context_management_config =
                    config_manager.get().context_management.clone();

                // Register skills virtual server
                let skills_config = config_manager.get().skills.clone();
                let skills_vs = Arc::new(
                    lr_mcp::gateway::virtual_skills::SkillsVirtualServer::new(
                        skill_manager.clone(),
                        context_management_config.clone(),
                        skills_config,
                    ),
                );
                app_state
                    .mcp_gateway
                    .register_virtual_server(skills_vs.clone());
                app.manage(skills_vs);
                info!("Skills virtual server registered");

                // Capture vector search setting before moving config
                let vector_search_enabled = context_management_config.vector_search_enabled;

                // Register context-mode virtual server
                let context_mode_vs = Arc::new(
                    lr_mcp::gateway::context_mode::ContextModeVirtualServer::new(
                        context_management_config,
                    ),
                );
                // Initialize embedding service for semantic vector search.
                // The service is always created (for status/download UI), but only
                // used for actual search when vector_search_enabled is true.
                let embedding_service: Option<Arc<lr_embeddings::EmbeddingService>> = {
                    match lr_utils::paths::config_dir() {
                        Ok(base_dir) => {
                            let service = Arc::new(lr_embeddings::EmbeddingService::new(&base_dir));
                            *app_state.embedding_service.write() = Some(service.clone());
                            // Auto-load if already downloaded and vector search is enabled
                            if vector_search_enabled && service.is_downloaded() {
                                if let Err(e) = service.ensure_loaded() {
                                    tracing::warn!("Failed to auto-load embedding model: {}", e);
                                }
                            }
                            info!("Embedding service initialized (downloaded: {}, loaded: {}, vector_search_enabled: {})",
                                service.is_downloaded(), service.is_loaded(), vector_search_enabled);
                            if vector_search_enabled {
                                Some(service)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to determine config dir for embeddings: {}", e);
                            None
                        }
                    }
                };

                // Pass embedding service to context-mode virtual server for session stores
                if let Some(ref es) = embedding_service {
                    context_mode_vs.set_embedding_service(Some(Arc::clone(es)));
                }

                app_state
                    .mcp_gateway
                    .register_virtual_server(context_mode_vs.clone());
                app.manage(context_mode_vs);
                info!("Context-mode virtual server registered");

                // Register marketplace virtual server if available
                if let Some(ref service) = marketplace_service {
                    let marketplace_vs = Arc::new(
                        lr_mcp::gateway::virtual_marketplace::MarketplaceVirtualServer::new(
                            service.clone(),
                        ),
                    );
                    app_state
                        .mcp_gateway
                        .register_virtual_server(marketplace_vs);
                    info!("Marketplace virtual server registered");
                }

                // Initialize memory service (per-client enablement checked at runtime)
                {
                    let memory_config = config_manager.get().memory.clone();
                    match lr_utils::paths::config_dir() {
                        Ok(base_dir) => {
                            let memory_dir = base_dir.join("memory");
                            let service = Arc::new(match embedding_service {
                                Some(ref es) => lr_memory::MemoryService::with_embedding_service(
                                    memory_config,
                                    memory_dir,
                                    Arc::clone(es),
                                ),
                                None => lr_memory::MemoryService::new(
                                    memory_config,
                                    memory_dir,
                                ),
                            });
                            *app_state.memory_service.write() = Some(service.clone());
                            app_state
                                .mcp_via_llm_manager
                                .set_memory_service(Some(service.clone()));

                            // Wire up compaction LLM (uses Router to call LLM for summarization)
                            service.set_compaction_llm(Arc::new(RouterCompactionLlm {
                                router: app_router.clone(),
                            }));

                            // Wire up monitor store for compaction events
                            service
                                .set_monitor_store(app_state.monitor_store.clone());

                            // Start session monitor (checks for expired sessions → triggers compaction)
                            service.start_session_monitor();

                            // Register _memory virtual server
                            let memory_vs = Arc::new(
                                lr_mcp::gateway::virtual_memory::MemoryVirtualServer::new(
                                    service,
                                ),
                            );
                            app_state
                                .mcp_gateway
                                .register_virtual_server(memory_vs);
                            info!("Memory virtual server registered");
                        }
                        Err(e) => {
                            tracing::warn!("Failed to determine config dir for memory: {}", e);
                        }
                    }
                }

                // Initialize coding agent manager
                {
                    let coding_agents_config = config_manager.get().coding_agents.clone();
                    let coding_agent_manager = Arc::new(
                        lr_coding_agents::manager::CodingAgentManager::new(coding_agents_config),
                    );
                    // Register coding agents virtual server
                    let coding_agents_vs = Arc::new(
                        lr_mcp::gateway::virtual_coding_agents::CodingAgentVirtualServer::new(
                            coding_agent_manager.clone(),
                        ),
                    );
                    app_state
                        .mcp_gateway
                        .register_virtual_server(coding_agents_vs);

                    // Subscribe to session changes and forward as Tauri events
                    {
                        let mut rx = coding_agent_manager.subscribe_changes();
                        let app_handle = app.handle().clone();
                        tokio::spawn(async move {
                            while rx.recv().await.is_ok() {
                                let _ = app_handle.emit("coding-agents-changed", ());
                            }
                        });
                    }

                    app.manage(coding_agent_manager);
                    info!("Coding agents virtual server registered");
                }

                // Initialize safety engine for guardrails
                {
                    let config_snapshot = config_manager.get();
                    let guardrails_config = &config_snapshot.guardrails;

                    if !guardrails_config.safety_models.is_empty() {
                        // Build provider lookup from configured providers
                        let mut provider_lookup = std::collections::HashMap::new();
                        for p in &config_snapshot.providers {
                            if !p.enabled {
                                continue;
                            }
                            let provider_type_str = match p.provider_type {
                                config::ProviderType::Ollama => "ollama",
                                config::ProviderType::LMStudio => "lmstudio",
                                config::ProviderType::OpenAI => "openai",
                                config::ProviderType::Groq => "groq",
                                config::ProviderType::DeepInfra => "deepinfra",
                                config::ProviderType::TogetherAI => "togetherai",
                                config::ProviderType::Mistral => "mistral",
                                config::ProviderType::Anthropic => "anthropic",
                                config::ProviderType::Cohere => "cohere",
                                config::ProviderType::OpenRouter => "openrouter",
                                config::ProviderType::Gemini => "gemini",
                                config::ProviderType::Perplexity => "perplexity",
                                config::ProviderType::Cerebras => "cerebras",
                                config::ProviderType::XAI => "xai",
                                config::ProviderType::Jan => "jan",
                                config::ProviderType::GPT4All => "gpt4all",
                                config::ProviderType::LocalAI => "localai",
                                config::ProviderType::LlamaCpp => "llamacpp",
                                _ => "openai_compatible",
                            };

                            // Extract endpoint from provider_config JSON
                            let endpoint = p
                                .provider_config
                                .as_ref()
                                .and_then(|cfg| cfg.get("endpoint"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| match p.provider_type {
                                    config::ProviderType::Ollama => {
                                        "http://localhost:11434".to_string()
                                    }
                                    config::ProviderType::LMStudio => {
                                        "http://localhost:1234".to_string()
                                    }
                                    config::ProviderType::Jan => {
                                        "http://localhost:1337".to_string()
                                    }
                                    config::ProviderType::GPT4All => {
                                        "http://localhost:4891".to_string()
                                    }
                                    config::ProviderType::LocalAI => {
                                        "http://localhost:8080".to_string()
                                    }
                                    config::ProviderType::LlamaCpp => {
                                        "http://localhost:8080".to_string()
                                    }
                                    config::ProviderType::OpenAI => {
                                        "https://api.openai.com/v1".to_string()
                                    }
                                    config::ProviderType::Groq => {
                                        "https://api.groq.com/openai/v1".to_string()
                                    }
                                    config::ProviderType::DeepInfra => {
                                        "https://api.deepinfra.com/v1/openai".to_string()
                                    }
                                    config::ProviderType::TogetherAI => {
                                        "https://api.together.xyz/v1".to_string()
                                    }
                                    config::ProviderType::Mistral => {
                                        "https://api.mistral.ai/v1".to_string()
                                    }
                                    config::ProviderType::Anthropic => {
                                        "https://api.anthropic.com/v1".to_string()
                                    }
                                    config::ProviderType::Cohere => {
                                        "https://api.cohere.com/v1".to_string()
                                    }
                                    config::ProviderType::OpenRouter => {
                                        "https://openrouter.ai/api/v1".to_string()
                                    }
                                    config::ProviderType::Gemini => {
                                        "https://generativelanguage.googleapis.com/v1beta"
                                            .to_string()
                                    }
                                    config::ProviderType::Perplexity => {
                                        "https://api.perplexity.ai".to_string()
                                    }
                                    config::ProviderType::Cerebras => {
                                        "https://api.cerebras.ai/v1".to_string()
                                    }
                                    config::ProviderType::XAI => {
                                        "https://api.x.ai/v1".to_string()
                                    }
                                    _ => "http://localhost:8080".to_string(),
                                });

                            // Try provider_config first, then fall back to keychain for cloud providers
                            let api_key = p
                                .provider_config
                                .as_ref()
                                .and_then(|cfg| cfg.get("api_key"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .or_else(|| {
                                    lr_providers::key_storage::get_provider_key(&p.name)
                                        .ok()
                                        .flatten()
                                });

                            provider_lookup.insert(
                                p.name.clone(),
                                lr_guardrails::ProviderInfo {
                                    name: p.name.clone(),
                                    base_url: endpoint,
                                    api_key,
                                    provider_type: provider_type_str.to_string(),
                                },
                            );
                        }

                        // Convert safety model configs
                        let model_inputs: Vec<lr_guardrails::SafetyModelConfigInput> =
                            guardrails_config
                                .safety_models
                                .iter()
                                .map(|m| lr_guardrails::SafetyModelConfigInput {
                                    id: m.id.clone(),
                                    model_type: m.model_type.clone(),
                                    provider_id: m.provider_id.clone(),
                                    model_name: m.model_name.clone(),
                                    enabled_categories: None,
                                })
                                .collect();

                        let engine = Arc::new(lr_guardrails::SafetyEngine::from_config(
                            &model_inputs,
                            guardrails_config.default_confidence_threshold,
                            &provider_lookup,
                        ));

                        info!(
                            "Guardrails enabled: {} models loaded",
                            engine.model_count()
                        );
                        *app_state.safety_engine.write() = Some(engine);
                    } else {
                        // Create empty engine so commands still work
                        *app_state.safety_engine.write() =
                            Some(Arc::new(lr_guardrails::SafetyEngine::empty()));
                        info!("Guardrails: no safety models configured");
                    }
                }

                // Initialize secret scanner if scanning is enabled
                {
                    let app_config = config_manager.get();
                    let ss_config = &app_config.secret_scanning;
                    if ss_config.action != lr_config::SecretScanAction::Off {
                        let engine_config = lr_secret_scanner::SecretScanEngineConfig {
                            entropy_threshold: ss_config.entropy_threshold,
                            allowlist: ss_config.allowlist.clone(),
                            scan_system_messages: ss_config.scan_system_messages,
                        };

                        match lr_secret_scanner::SecretScanEngine::new(&engine_config) {
                            Ok(engine) => {
                                *app_state.secret_scanner.write() = Some(Arc::new(engine));
                                info!("Secret scanner initialized");
                            }
                            Err(e) => {
                                error!("Failed to initialize secret scanner: {}", e);
                            }
                        }
                    } else {
                        info!("Secret scanning disabled");
                    }
                }

                // Set app handle on AppState for event emission
                app_state.set_app_handle(app.handle().clone());

                // Manage the MCP via LLM manager separately for Tauri command access
                app.manage(app_state.mcp_via_llm_manager.clone());

                let app_state = Arc::new(app_state);
                app.manage(app_state.clone());

                // Listen for client permission changes and notify connected MCP clients
                let app_state_for_clients = app_state.clone();
                let config_manager_for_notify = config_manager.clone();
                app.listen("clients-changed", move |_event| {
                    let config = config_manager_for_notify.get();
                    let all_enabled_server_ids: Vec<String> = config
                        .mcp_servers
                        .iter()
                        .filter(|s| s.enabled)
                        .map(|s| s.id.clone())
                        .collect();

                    let broadcast = app_state_for_clients.client_notification_broadcast.clone();
                    app_state_for_clients
                        .mcp_gateway
                        .check_and_notify_permission_changes(
                            &config.clients,
                            &all_enabled_server_ids,
                            |client_id, tools, resources, prompts| {
                                use mcp::gateway::streaming_notifications::StreamingNotificationType;
                                if tools {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::ToolsListChanged
                                            .to_notification(),
                                    ));
                                }
                                if resources {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::ResourcesListChanged
                                            .to_notification(),
                                    ));
                                }
                                if prompts {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::PromptsListChanged
                                            .to_notification(),
                                    ));
                                }
                                info!(
                                    "Sent permission change notifications to client {}: tools={}, resources={}, prompts={}",
                                    client_id, tools, resources, prompts
                                );
                            },
                        );
                });
                info!("Registered clients-changed listener for permission notifications");

                // Re-evaluate pending firewall approvals when permissions or strategies change.
                // clients-changed: covers client permission updates (MCP, model, guardrails)
                // strategies-changed: covers strategy updates (e.g. free_tier_fallback)
                let make_reeval_handler = |app_state: std::sync::Arc<lr_server::state::AppState>,
                                           config_manager: lr_config::ConfigManager,
                                           app_handle: tauri::AppHandle| {
                    move |_event: tauri::Event| {
                        crate::ui::commands_clients::reevaluate_pending_approvals(
                            &app_handle,
                            &app_state.mcp_gateway.firewall_manager,
                            &config_manager,
                            &app_state.model_approval_tracker,
                            &app_state.guardrail_approval_tracker,
                            &app_state.guardrail_denial_tracker,
                            &app_state.free_tier_approval_tracker,
                            &app_state.auto_router_approval_tracker,
                        );

                        if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app_handle) {
                            tracing::warn!(
                                "Failed to rebuild tray menu after re-evaluation: {}",
                                e
                            );
                        }
                        if let Some(tray_manager) = app_handle
                            .try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>(
                        ) {
                            tray_manager.notify_activity();
                        }
                    }
                };

                app.listen(
                    "clients-changed",
                    make_reeval_handler(
                        app_state.clone(),
                        config_manager.clone(),
                        app.handle().clone(),
                    ),
                );
                app.listen(
                    "strategies-changed",
                    make_reeval_handler(
                        app_state.clone(),
                        config_manager.clone(),
                        app.handle().clone(),
                    ),
                );
                info!("Registered clients/strategies-changed listeners for firewall re-evaluation");

                // Spawn firewall approval popup listener
                // Subscribes to MCP notification broadcast and opens popup windows
                // when firewall/approvalRequired notifications arrive
                let app_handle_for_firewall = app.handle().clone();
                let firewall_broadcast_rx =
                    app_state.mcp_notification_broadcast.subscribe();
                tokio::spawn(async move {
                    use tauri::WebviewWindowBuilder;
                    let mut rx = firewall_broadcast_rx;
                    loop {
                        match rx.recv().await {
                            Ok((channel, notification)) => {
                                if channel != "_firewall" {
                                    continue;
                                }
                                // Extract request_id from notification params
                                let request_id = notification
                                    .params
                                    .as_ref()
                                    .and_then(|p| p.get("request_id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if request_id.is_empty() {
                                    tracing::warn!(
                                        "Firewall notification missing request_id, skipping"
                                    );
                                    continue;
                                }

                                tracing::info!(
                                    "Opening firewall approval popup for request {}",
                                    request_id
                                );

                                // Rebuild tray menu to show the pending approval
                                if let Err(e) =
                                    crate::ui::tray::rebuild_tray_menu(&app_handle_for_firewall)
                                {
                                    tracing::warn!(
                                        "Failed to rebuild tray menu for firewall: {}",
                                        e
                                    );
                                }
                                if let Some(tgm) = app_handle_for_firewall
                                    .try_state::<Arc<crate::ui::tray::TrayGraphManager>>()
                                {
                                    tgm.notify_activity();
                                }

                                // Create popup window
                                match WebviewWindowBuilder::new(
                                    &app_handle_for_firewall,
                                    format!("firewall-approval-{}", request_id),
                                    tauri::WebviewUrl::App("index.html".into()),
                                )
                                .title("Approval Required")
                                .inner_size(400.0, 320.0)
                                .center()
                                .visible(true)
                                .resizable(false)
                                .decorations(true)
                                .always_on_top(true)
                                .build()
                                {
                                    Ok(window) => {
                                        let _ = window.set_focus();
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to create firewall popup: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(
                                    "Firewall listener lagged, missed {} notifications",
                                    n
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("Firewall broadcast channel closed, stopping listener");
                                break;
                            }
                        }
                    }
                });
                info!("Spawned firewall approval popup listener");

                // Spawn sampling approval popup listener
                let app_handle_for_sampling = app.handle().clone();
                let sampling_broadcast_rx =
                    app_state.mcp_notification_broadcast.subscribe();
                tokio::spawn(async move {
                    use tauri::WebviewWindowBuilder;
                    let mut rx = sampling_broadcast_rx;
                    loop {
                        match rx.recv().await {
                            Ok((channel, notification)) => {
                                if channel != "_sampling_approval" {
                                    continue;
                                }
                                let request_id = notification
                                    .params
                                    .as_ref()
                                    .and_then(|p| p.get("request_id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if request_id.is_empty() {
                                    tracing::warn!(
                                        "Sampling approval notification missing request_id"
                                    );
                                    continue;
                                }

                                tracing::info!(
                                    "Opening sampling approval popup for request {}",
                                    request_id
                                );

                                match WebviewWindowBuilder::new(
                                    &app_handle_for_sampling,
                                    format!("sampling-approval-{}", request_id),
                                    tauri::WebviewUrl::App("index.html".into()),
                                )
                                .title("Sampling Approval")
                                .inner_size(400.0, 320.0)
                                .center()
                                .visible(false)
                                .resizable(false)
                                .decorations(true)
                                .always_on_top(true)
                                .build()
                                {
                                    Ok(window) => {
                                        let _ = window.set_focus();
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to create sampling approval popup: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(
                                    "Sampling approval listener lagged, missed {} notifications",
                                    n
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("Sampling approval broadcast channel closed");
                                break;
                            }
                        }
                    }
                });
                info!("Spawned sampling approval popup listener");

                // Spawn elicitation form popup listener
                let app_handle_for_elicitation = app.handle().clone();
                let elicitation_broadcast_rx =
                    app_state.mcp_notification_broadcast.subscribe();
                tokio::spawn(async move {
                    use tauri::WebviewWindowBuilder;
                    let mut rx = elicitation_broadcast_rx;
                    loop {
                        match rx.recv().await {
                            Ok((channel, notification)) => {
                                if channel != "_elicitation" {
                                    continue;
                                }
                                let request_id = notification
                                    .params
                                    .as_ref()
                                    .and_then(|p| p.get("request_id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if request_id.is_empty() {
                                    tracing::warn!(
                                        "Elicitation notification missing request_id"
                                    );
                                    continue;
                                }

                                tracing::info!(
                                    "Opening elicitation form popup for request {}",
                                    request_id
                                );

                                match WebviewWindowBuilder::new(
                                    &app_handle_for_elicitation,
                                    format!("elicitation-form-{}", request_id),
                                    tauri::WebviewUrl::App("index.html".into()),
                                )
                                .title("Input Required")
                                .inner_size(400.0, 420.0)
                                .center()
                                .visible(false)
                                .resizable(true)
                                .decorations(true)
                                .always_on_top(true)
                                .build()
                                {
                                    Ok(window) => {
                                        let _ = window.set_focus();
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to create elicitation form popup: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(
                                    "Elicitation listener lagged, missed {} notifications",
                                    n
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("Elicitation broadcast channel closed");
                                break;
                            }
                        }
                    }
                });
                info!("Spawned elicitation form popup listener");
            } else {
                error!("Failed to get AppState from server manager");
            }

            // Set up server restart event listener
            let server_manager_clone = server_manager.clone();
            let app_router_clone = app_router.clone();
            let mcp_server_manager_clone = mcp_server_manager.clone();
            let rate_limiter_clone = rate_limiter.clone();
            let provider_registry_clone = provider_registry.clone();
            let client_manager_clone = client_manager.clone();
            let token_store_clone = token_store.clone();
            let metrics_collector_clone = metrics_collector.clone();
            let config_manager_clone = config_manager.clone();
            let app_handle_for_restart = app.handle().clone();
            // Clone health cache for restart handler
            let health_cache_for_restart =
                server_manager.get_state().map(|s| s.health_cache.clone());

            app.listen("server-restart-requested", move |_event| {
                info!("Server restart requested");

                // Get current server configuration
                let server_config = {
                    let config = config_manager_clone.get();
                    server::ServerConfig {
                        host: config.server.host.clone(),
                        port: config.server.port,
                        enable_cors: config.server.enable_cors,
                    }
                };

                // Restart the server (spawn async task)
                let server_manager_clone2 = server_manager_clone.clone();
                let app_router_clone2 = app_router_clone.clone();
                let mcp_server_manager_clone2 = mcp_server_manager_clone.clone();
                let rate_limiter_clone2 = rate_limiter_clone.clone();
                let provider_registry_clone2 = provider_registry_clone.clone();
                let config_manager_clone2 = Arc::new(config_manager_clone.clone());
                let client_manager_clone2 = client_manager_clone.clone();
                let token_store_clone2 = token_store_clone.clone();
                let metrics_collector_clone2 = metrics_collector_clone.clone();
                let app_handle = app_handle_for_restart.clone();
                let health_cache_clone = health_cache_for_restart.clone();

                tokio::spawn(async move {
                    use tauri::Emitter;

                    // Update health cache and emit stopped status before restart
                    if let Some(ref health_cache) = health_cache_clone {
                        health_cache.update_server_status(false, None, None);
                    }
                    let _ = app_handle.emit("server-status-changed", "stopped");

                    match server_manager_clone2
                        .start(
                            server_config.clone(),
                            lr_server::manager::ServerDependencies {
                                router: app_router_clone2,
                                mcp_server_manager: mcp_server_manager_clone2,
                                rate_limiter: rate_limiter_clone2,
                                provider_registry: provider_registry_clone2,
                                config_manager: config_manager_clone2,
                                client_manager: client_manager_clone2,
                                token_store: token_store_clone2,
                                metrics_collector: metrics_collector_clone2,
                                health_cache: health_cache_clone.clone(),
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            info!("Server restarted successfully");
                            // Update health cache with new server status
                            if let Some(ref health_cache) = health_cache_clone {
                                health_cache.update_server_status(
                                    true,
                                    Some(server_config.host.clone()),
                                    Some(server_config.port),
                                );
                            }
                            let _ = app_handle.emit("server-status-changed", "running");
                            let _ = app_handle.emit("server-restart-completed", ());
                        }
                        Err(e) => {
                            error!("Failed to restart server: {}", e);
                            // Keep server status as stopped in health cache
                            if let Some(ref health_cache) = health_cache_clone {
                                health_cache.update_server_status(false, None, None);
                            }
                            let _ = app_handle.emit("server-status-changed", "stopped");
                            let _ = app_handle.emit("server-restart-failed", e.to_string());
                        }
                    }
                });
            });

            // Set app handle on server state for event emission
            if let Some(state) = server_manager.get_state() {
                state.set_app_handle(app.handle().clone());

                // Spawn background aggregation task for metrics
                let metrics_db = state.metrics_collector.db();
                tokio::spawn(async move {
                    let _ = monitoring::aggregation_task::spawn_aggregation_task(metrics_db).await;
                });
                info!("Spawned metrics aggregation task");

                // Initialize health cache with current providers and MCP servers
                let health_cache = state.health_cache.clone();
                {
                    use providers::health_cache::ItemHealth;

                    // Initialize providers - disabled ones get disabled status
                    let providers = provider_registry.list_providers();
                    for provider_info in providers {
                        if provider_info.enabled {
                            health_cache.update_provider(
                                &provider_info.instance_name,
                                ItemHealth::pending(provider_info.instance_name.clone()),
                            );
                        } else {
                            health_cache.update_provider(
                                &provider_info.instance_name,
                                ItemHealth::disabled(provider_info.instance_name.clone()),
                            );
                        }
                    }

                    // Initialize MCP servers - disabled ones get disabled status
                    let mcp_configs = mcp_server_manager.list_configs();
                    for config in mcp_configs {
                        if config.enabled {
                            health_cache.update_mcp_server(
                                &config.id,
                                ItemHealth::pending(config.name.clone()),
                            );
                        } else {
                            health_cache
                                .update_mcp_server(&config.id, ItemHealth::disabled(config.name));
                        }
                    }
                }

                // Set server as running with the configured host and port
                let server_config = &config_manager.get().server;
                health_cache.update_server_status(
                    true,
                    Some(server_config.host.clone()),
                    Some(server_config.port),
                );
                info!(
                    "Health cache initialized with {} providers and {} MCP servers",
                    provider_registry.list_providers().len(),
                    mcp_server_manager.list_configs().len()
                );

                // Start periodic health check task (always spawned, checks enabled flag dynamically)
                let health_check_config = config_manager.get().health_check.clone();
                if health_check_config.mode == config::HealthCheckMode::Periodic {
                    // Set the initial runtime flag from config
                    state.health_cache.set_periodic_enabled(health_check_config.periodic_enabled);
                    let periodic_enabled_flag = state.health_cache.periodic_enabled_flag();

                    let health_cache_for_task = state.health_cache.clone();
                    let provider_registry_for_task = provider_registry.clone();
                    let mcp_server_manager_for_task = mcp_server_manager.clone();
                    let interval_secs = health_check_config.interval_secs;
                    let timeout_secs = health_check_config.timeout_secs;

                    tokio::spawn(async move {
                        use providers::health_cache::ItemHealth;
                        use std::sync::atomic::Ordering;

                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                        let mut provider_cycle_counters: std::collections::HashMap<String, u32> =
                            std::collections::HashMap::new();

                        loop {
                            interval.tick().await;

                            // Check runtime flag - skip if disabled
                            if !periodic_enabled_flag.load(Ordering::Relaxed) {
                                continue;
                            }

                            debug!("Running periodic health checks...");

                            // Check all providers
                            let providers = provider_registry_for_task.list_providers();
                            for provider_info in providers {
                                // Skip disabled providers - emit disabled status
                                if !provider_info.enabled {
                                    health_cache_for_task.update_provider(
                                        &provider_info.instance_name,
                                        ItemHealth::disabled(provider_info.instance_name.clone()),
                                    );
                                    continue;
                                }

                                if let Some(provider) = provider_registry_for_task
                                    .get_provider(&provider_info.instance_name)
                                {
                                    // Respect per-provider interval multiplier
                                    let multiplier = provider.health_check_interval_multiplier();
                                    if multiplier > 1 {
                                        let counter = provider_cycle_counters
                                            .entry(provider_info.instance_name.clone())
                                            .or_insert(0);
                                        *counter += 1;
                                        if !(*counter).is_multiple_of(multiplier) {
                                            continue;
                                        }
                                    }

                                    let health = tokio::time::timeout(
                                        std::time::Duration::from_secs(timeout_secs),
                                        provider.health_check(),
                                    )
                                    .await;

                                    let item_health = match health {
                                        Ok(h) => {
                                            use providers::HealthStatus;
                                            match h.status {
                                                HealthStatus::Healthy => ItemHealth::healthy(
                                                    provider_info.instance_name.clone(),
                                                    h.latency_ms,
                                                ),
                                                HealthStatus::Degraded => ItemHealth::degraded(
                                                    provider_info.instance_name.clone(),
                                                    h.latency_ms,
                                                    h.error_message
                                                        .unwrap_or_else(|| "Degraded".to_string()),
                                                ),
                                                HealthStatus::Unhealthy => ItemHealth::unhealthy(
                                                    provider_info.instance_name.clone(),
                                                    h.error_message
                                                        .unwrap_or_else(|| "Unhealthy".to_string()),
                                                ),
                                            }
                                        }
                                        Err(_) => ItemHealth::unhealthy(
                                            provider_info.instance_name.clone(),
                                            format!("Health check timeout ({}s)", timeout_secs),
                                        ),
                                    };
                                    health_cache_for_task
                                        .update_provider(&provider_info.instance_name, item_health);
                                }
                            }

                            // Check all MCP servers
                            let mcp_configs = mcp_server_manager_for_task.list_configs();
                            for config in mcp_configs {
                                // Skip disabled MCP servers - emit disabled status
                                if !config.enabled {
                                    health_cache_for_task.update_mcp_server(
                                        &config.id,
                                        ItemHealth::disabled(config.name),
                                    );
                                    continue;
                                }

                                let mcp_server_health = mcp_server_manager_for_task
                                    .get_server_health(&config.id)
                                    .await;
                                use mcp::manager::HealthStatus as McpHealthStatus;
                                let server_id = mcp_server_health.server_id.clone();
                                let server_name = mcp_server_health.server_name.clone();
                                let item_health = match mcp_server_health.status {
                                    McpHealthStatus::Ready => ItemHealth::ready(server_name),
                                    McpHealthStatus::Healthy => ItemHealth::healthy(
                                        server_name,
                                        mcp_server_health.latency_ms,
                                    ),
                                    McpHealthStatus::Unhealthy | McpHealthStatus::Unknown => {
                                        ItemHealth::unhealthy(
                                            server_name,
                                            mcp_server_health
                                                .error
                                                .unwrap_or_else(|| "Unhealthy".to_string()),
                                        )
                                    }
                                };
                                health_cache_for_task.update_mcp_server(&server_id, item_health);
                            }

                            health_cache_for_task.mark_refresh();
                            debug!("Periodic health checks completed");
                        }
                    });
                    info!(
                        "Started periodic health check task (interval: {}s, initially {})",
                        interval_secs,
                        if health_check_config.periodic_enabled { "enabled" } else { "disabled" }
                    );
                } else {
                    info!("Health check mode is on-failure, skipping periodic task");
                }

                // Spawn recovery task: re-checks unhealthy providers on a faster cadence
                // Wakes on notify (when a provider is marked unhealthy) or every recovery_interval
                {
                    let health_cache_for_recovery = state.health_cache.clone();
                    let provider_registry_for_recovery = provider_registry.clone();
                    let recovery_interval = std::time::Duration::from_secs(
                        health_check_config.recovery_interval_secs,
                    );
                    let timeout_secs = health_check_config.timeout_secs;
                    let notify = health_cache_for_recovery.recovery_notify();

                    tokio::spawn(async move {
                        use providers::health_cache::ItemHealth;

                        loop {
                            // Wait for either a notify signal or the recovery interval
                            tokio::select! {
                                _ = notify.notified() => {
                                    // Small delay to batch multiple rapid failures
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }
                                _ = tokio::time::sleep(recovery_interval) => {}
                            }

                            let unhealthy = health_cache_for_recovery.get_unhealthy_providers();
                            if unhealthy.is_empty() {
                                continue;
                            }

                            debug!(
                                "Recovery check: re-checking {} unhealthy provider(s): {:?}",
                                unhealthy.len(),
                                unhealthy
                            );

                            for provider_name in &unhealthy {
                                if let Some(provider) =
                                    provider_registry_for_recovery.get_provider(provider_name)
                                {
                                    let health = tokio::time::timeout(
                                        std::time::Duration::from_secs(timeout_secs),
                                        provider.health_check(),
                                    )
                                    .await;

                                    let item_health = match health {
                                        Ok(h) => {
                                            use providers::HealthStatus;
                                            match h.status {
                                                HealthStatus::Healthy => {
                                                    info!(
                                                        "Provider {} recovered",
                                                        provider_name
                                                    );
                                                    ItemHealth::healthy(
                                                        provider_name.clone(),
                                                        h.latency_ms,
                                                    )
                                                }
                                                HealthStatus::Degraded => ItemHealth::degraded(
                                                    provider_name.clone(),
                                                    h.latency_ms,
                                                    h.error_message.unwrap_or_else(|| {
                                                        "Degraded".to_string()
                                                    }),
                                                ),
                                                HealthStatus::Unhealthy => ItemHealth::unhealthy(
                                                    provider_name.clone(),
                                                    h.error_message.unwrap_or_else(|| {
                                                        "Unhealthy".to_string()
                                                    }),
                                                ),
                                            }
                                        }
                                        Err(_) => ItemHealth::unhealthy(
                                            provider_name.clone(),
                                            format!(
                                                "Recovery check timeout ({}s)",
                                                timeout_secs
                                            ),
                                        ),
                                    };
                                    health_cache_for_recovery
                                        .update_provider(provider_name, item_health);
                                }
                            }
                        }
                    });
                    info!(
                        "Started recovery health check task (interval: {}s)",
                        recovery_interval.as_secs()
                    );
                }
            }

            // Refresh model cache for tray menu
            info!("Refreshing model cache...");
            let provider_registry_clone = provider_registry.clone();
            let app_handle_clone = app.handle().clone();
            tokio::spawn(async move {
                if let Err(e) = provider_registry_clone.refresh_model_cache().await {
                    error!("Failed to refresh model cache: {}", e);
                } else {
                    info!("Model cache refreshed successfully");
                    // Rebuild tray menu with models
                    if let Err(e) = ui::tray::rebuild_tray_menu(&app_handle_clone) {
                        error!("Failed to rebuild tray menu after model refresh: {}", e);
                    }
                }
            });

            // Initialize update notification state
            let update_notification_state = Arc::new(ui::tray::UpdateNotificationState::new());
            app.manage(update_notification_state.clone());

            // Setup system tray
            ui::tray::setup_tray(app)?;

            // Configure window for test mode
            if utils::test_mode::is_test_mode() {
                info!("Running in TEST MODE - configuring window for testing");
                if let Some(window) = app.get_webview_window("main") {
                    // Add [TEST] to window title
                    let _ = window.set_title("LocalRouter [TEST]");
                    // Make window smaller (800x500)
                    let _ = window.set_size(tauri::LogicalSize::new(800.0, 500.0));
                    // Position in bottom-right corner
                    if let Ok(Some(monitor)) = window.current_monitor() {
                        let screen_size = monitor.size();
                        let scale = monitor.scale_factor();
                        // Position 50px from right and bottom edges
                        let x = (screen_size.width as f64 / scale) - 800.0 - 50.0;
                        let y = (screen_size.height as f64 / scale) - 500.0 - 50.0;
                        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
                    }
                    // Minimize the window so it doesn't take focus
                    // User can restore it from taskbar/dock if needed
                    let _ = window.minimize();
                    info!("Test mode window configured: 800x500, bottom-right corner, minimized");
                }
            }

            // Initialize tray graph manager
            info!("Initializing tray graph manager...");
            let ui_config = config_manager.get().ui.clone();
            let tray_graph_manager = Arc::new(ui::tray::TrayGraphManager::new(
                app.handle().clone(),
                ui_config,
            ));
            app.manage(tray_graph_manager.clone());
            info!("Tray graph manager initialized");

            // Set tray graph manager on AppState for request handlers
            if let Some(app_state) = server_manager.get_state() {
                app_state.set_tray_graph_manager(tray_graph_manager.clone());
                info!("Tray graph manager set on AppState");
            }

            // Set up metrics callback to notify graph manager after metrics are recorded
            let tray_graph_manager_for_metrics = tray_graph_manager.clone();
            metrics_collector.set_on_metrics_recorded(move || {
                tray_graph_manager_for_metrics.notify_activity();
            });
            info!("Metrics callback registered with tray graph manager");

            // Listen for server status changes to update tray icon
            let app_handle = app.handle().clone();
            app.listen("server-status-changed", move |event| {
                let status = event.payload();
                info!("Server status changed to: {}", status);
                if let Err(e) = ui::tray::update_tray_icon(&app_handle, status) {
                    error!("Failed to update tray icon: {}", e);
                }
            });

            // Listen for LLM request events to show "active" icon (when graph is disabled)
            let app_handle2 = app.handle().clone();
            let tray_graph_manager_clone = tray_graph_manager.clone();
            app.listen("llm-request", move |_event| {
                // Show "active" icon immediately (only if graph is disabled)
                // When graph is enabled, it will update via metrics callback
                if !tray_graph_manager_clone.is_enabled() {
                    if let Err(e) = ui::tray::update_tray_icon(&app_handle2, "active") {
                        error!("Failed to update tray icon for LLM request: {}", e);
                    }

                    // Restore to "running" after 2 seconds
                    let app_handle_restore = app_handle2.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        let _ = ui::tray::update_tray_icon(&app_handle_restore, "running");
                    });
                }
            });

            // Debounced config sync for clients with sync_config enabled
            {
                let (sync_tx, mut sync_rx) = tokio::sync::mpsc::channel::<()>(16);

                // Spawn debounced sync task
                let cm_for_sync = config_manager.clone();
                let client_mgr_for_sync = client_manager.clone();
                let pr_for_sync = provider_registry.clone();
                tokio::spawn(async move {
                    // Sync on startup
                    ui::commands_clients::sync_all_clients(
                        &cm_for_sync,
                        &client_mgr_for_sync,
                        &pr_for_sync,
                    )
                    .await;

                    loop {
                        // Wait for a signal
                        if sync_rx.recv().await.is_none() {
                            break;
                        }
                        // Drain any queued signals
                        while sync_rx.try_recv().is_ok() {}
                        // Debounce: wait then drain again
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        while sync_rx.try_recv().is_ok() {}
                        // Sync all clients
                        ui::commands_clients::sync_all_clients(
                            &cm_for_sync,
                            &client_mgr_for_sync,
                            &pr_for_sync,
                        )
                        .await;
                    }
                });

                // Listen for model and strategy changes to trigger sync
                let sync_tx_models = sync_tx.clone();
                app.listen("models-changed", move |_event| {
                    let _ = sync_tx_models.try_send(());
                });

                let sync_tx_strategies = sync_tx;
                app.listen("strategies-changed", move |_event| {
                    let _ = sync_tx_strategies.try_send(());
                });
            }

            // Start background free tier persistence (every 60 seconds)
            let free_tier_manager_persist = free_tier_manager.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    if let Err(e) = free_tier_manager_persist.persist() {
                        tracing::error!("Failed to persist free tier state: {}", e);
                    }
                }
            });

            // Start background update checker
            info!("Starting background update checker...");
            let app_handle_for_updater = app.handle().clone();
            let config_manager_for_updater = Arc::new(config_manager.clone());
            tokio::spawn(async move {
                updater::start_update_timer(app_handle_for_updater, config_manager_for_updater)
                    .await;
            });
            info!("Background update checker started");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::get_config,
            ui::commands::reload_config,
            ui::commands::set_provider_api_key,
            ui::commands::has_provider_api_key,
            ui::commands::delete_provider_api_key,
            ui::commands::list_providers_with_key_status,
            // Provider registry commands
            ui::commands::list_provider_types,
            ui::commands::list_provider_instances,
            ui::commands::create_provider_instance,
            ui::commands::get_provider_config,
            ui::commands::update_provider_instance,
            ui::commands::rename_provider_instance,
            ui::commands::get_provider_api_key,
            ui::commands::remove_provider_instance,
            ui::commands::clone_provider_instance,
            ui::commands::set_provider_enabled,
            ui::commands::get_providers_health,
            ui::commands::start_provider_health_checks,
            ui::commands::check_single_provider_health,
            // Centralized health cache commands
            ui::commands::get_health_cache,
            ui::commands::refresh_all_health,
            ui::commands::get_periodic_health_enabled,
            ui::commands::set_periodic_health_enabled,
            ui::commands::list_provider_models,
            ui::commands::list_all_models,
            ui::commands::list_all_models_detailed,
            ui::commands::get_cached_models,
            ui::commands::refresh_models_incremental,
            ui::commands::get_catalog_stats,
            ui::commands::get_catalog_metadata,
            // Feature support matrix commands
            ui::commands::get_provider_feature_support,
            ui::commands::get_all_provider_feature_support,
            ui::commands::get_feature_endpoint_matrix,
            // Server configuration commands
            ui::commands::get_server_config,
            ui::commands::update_server_config,
            ui::commands::restart_server,
            // Monitoring & statistics commands
            ui::commands::get_aggregate_stats,
            ui::commands::get_feature_stats,
            // Metrics commands
            ui::commands_metrics::get_global_metrics,
            ui::commands_metrics::get_api_key_metrics,
            ui::commands_metrics::get_provider_metrics,
            ui::commands_metrics::get_model_metrics,
            ui::commands_metrics::list_tracked_models,
            ui::commands_metrics::list_tracked_providers,
            ui::commands_metrics::list_tracked_api_keys,
            ui::commands_metrics::compare_api_keys,
            ui::commands_metrics::compare_providers,
            ui::commands_metrics::compare_models,
            ui::commands_metrics::get_strategy_metrics,
            ui::commands_metrics::list_tracked_strategies,
            ui::commands_metrics::compare_strategies,
            // MCP metrics commands
            ui::commands_mcp_metrics::get_global_mcp_metrics,
            ui::commands_mcp_metrics::get_client_mcp_metrics,
            ui::commands_mcp_metrics::get_mcp_server_metrics,
            ui::commands_mcp_metrics::get_mcp_method_breakdown,
            ui::commands_mcp_metrics::list_tracked_mcp_clients,
            ui::commands_mcp_metrics::list_tracked_mcp_servers,
            ui::commands_mcp_metrics::compare_mcp_clients,
            ui::commands_mcp_metrics::compare_mcp_servers,
            ui::commands_mcp_metrics::get_mcp_latency_percentiles,
            // Network interface commands
            ui::commands::get_network_interfaces,
            ui::commands::get_executable_path,
            // Server control commands
            ui::commands::get_server_status,
            ui::commands::stop_server,
            // OAuth commands
            ui::commands::list_oauth_providers,
            ui::commands::start_oauth_flow,
            ui::commands::poll_oauth_status,
            ui::commands::cancel_oauth_flow,
            ui::commands::list_oauth_credentials,
            ui::commands::delete_oauth_credentials,
            // OAuth client commands (for MCP)
            ui::commands::list_oauth_clients,
            ui::commands::create_oauth_client,
            ui::commands::get_oauth_client_secret,
            ui::commands::delete_oauth_client,
            ui::commands::update_oauth_client_name,
            ui::commands::toggle_oauth_client_enabled,
            ui::commands::link_mcp_server,
            ui::commands::unlink_mcp_server,
            ui::commands::get_oauth_client_linked_servers,
            // MCP server commands
            ui::commands::list_mcp_servers,
            ui::commands::create_mcp_server,
            ui::commands::delete_mcp_server,
            ui::commands::clone_mcp_server,
            ui::commands::start_mcp_server,
            ui::commands::stop_mcp_server,
            ui::commands::get_mcp_server_health,
            ui::commands::get_all_mcp_server_health,
            ui::commands::start_mcp_health_checks,
            ui::commands::check_single_mcp_health,
            ui::commands::update_mcp_server_name,
            ui::commands::update_mcp_server_config,
            ui::commands::update_mcp_server,
            ui::commands::toggle_mcp_server_enabled,
            ui::commands::list_mcp_tools,
            ui::commands::call_mcp_tool,
            ui::commands::get_mcp_token_stats,
            // MCP OAuth browser flow commands
            ui::commands::start_mcp_oauth_browser_flow,
            ui::commands::poll_mcp_oauth_browser_status,
            ui::commands::cancel_mcp_oauth_browser_flow,
            ui::commands::discover_mcp_oauth_endpoints,
            ui::commands::test_mcp_oauth_connection,
            ui::commands::revoke_mcp_oauth_tokens,
            // Inline OAuth flow commands (for MCP server creation)
            ui::commands::start_inline_oauth_flow,
            ui::commands::poll_inline_oauth_status,
            ui::commands::cancel_inline_oauth_flow,
            // Unified client management commands
            ui::commands::list_clients,
            ui::commands::create_client,
            ui::commands::delete_client,
            ui::commands::clone_client,
            ui::commands::update_client_name,
            ui::commands::toggle_client_enabled,
            ui::commands::rotate_client_secret,
            ui::commands::toggle_client_context_management,
            ui::commands::toggle_client_catalog_compression,
            ui::commands::get_client_value,
            ui::commands::get_client_effective_config,
            // Strategy management commands
            ui::commands::list_strategies,
            ui::commands::get_strategy,
            ui::commands::create_strategy,
            ui::commands::update_strategy,
            ui::commands::delete_strategy,
            ui::commands::get_clients_using_strategy,
            ui::commands::get_feature_clients_status,
            // Client template, mode & guardrails commands
            ui::commands::set_client_mode,
            ui::commands::set_client_template,
            ui::commands::get_client_guardrails_config,
            ui::commands::update_client_guardrails_config,
            // App launcher commands
            ui::commands::get_app_capabilities,
            ui::commands::try_it_out_app,
            ui::commands::configure_app_permanent,
            // Config sync commands
            ui::commands::toggle_client_sync_config,
            ui::commands::sync_client_config,
            // Firewall approval commands
            ui::commands::submit_firewall_approval,
            ui::commands::list_pending_firewall_approvals,
            ui::commands::get_firewall_approval_details,
            ui::commands::get_firewall_full_arguments,
            // Unified permission commands
            ui::commands::set_client_mcp_permission,
            ui::commands::set_client_skills_permission,
            ui::commands::set_client_model_permission,
            ui::commands::set_client_marketplace_permission,
            ui::commands::set_client_sampling_permission,
            ui::commands::set_client_elicitation_permission,
            ui::commands::clear_client_mcp_child_permissions,
            ui::commands::clear_client_skills_child_permissions,
            ui::commands::clear_client_model_child_permissions,
            ui::commands::get_mcp_server_capabilities,
            ui::commands::get_mcp_gateway_settings,
            ui::commands::set_mcp_gateway_settings,
            ui::commands::get_skill_tools,
            // OpenAPI documentation commands
            ui::commands::get_openapi_spec,
            // Internal testing commands
            ui::commands::get_internal_test_token,
            ui::commands::create_test_client_for_strategy,
            // Access logs commands
            ui::commands::get_llm_logs,
            ui::commands::get_mcp_logs,
            // Pricing override commands
            ui::commands::get_pricing_override,
            ui::commands::set_pricing_override,
            ui::commands::delete_pricing_override,
            // Tray graph settings commands
            ui::commands::get_tray_graph_settings,
            ui::commands::update_tray_graph_settings,
            // Sidebar commands
            ui::commands::get_sidebar_expanded,
            ui::commands::set_sidebar_expanded,
            // System commands
            ui::commands::get_home_dir,
            ui::commands::get_config_dir,
            ui::commands::detect_available_runtimes,
            // Update checking commands
            ui::commands::get_app_version,
            ui::commands::get_update_config,
            ui::commands::update_update_config,
            ui::commands::mark_update_check_performed,
            ui::commands::skip_update_version,
            ui::commands::set_update_notification,
            // Logging configuration commands
            ui::commands::get_logging_config,
            ui::commands::update_logging_config,
            ui::commands::open_logs_folder,
            // GuardRails commands
            ui::commands::get_guardrails_config,
            ui::commands::update_guardrails_config,
            ui::commands::rebuild_safety_engine,
            ui::commands::test_safety_check,
            // Memory commands
            ui::commands::get_memory_config,
            ui::commands::update_memory_config,
            ui::commands::open_memory_folder,
            ui::commands::memory_test_sample,
            ui::commands::memory_test_index,
            ui::commands::memory_test_search,
            ui::commands::memory_test_reset,
            ui::commands::list_memory_clients,
            ui::commands::search_client_memory,
            ui::commands::read_client_memory,
            ui::commands::clear_client_memory,
            ui::commands::open_client_memory_folder,
            ui::commands::get_client_memory_config,
            ui::commands::update_client_memory_config,
            // Memory Compaction commands
            ui::commands::get_memory_compaction_stats,
            ui::commands::force_compact_memory,
            ui::commands::recompact_memory,
            ui::commands::reindex_client_memory,
            ui::commands::read_memory_archive_file,
            // Secret Scanning commands
            ui::commands::rebuild_secret_scanner,
            ui::commands::get_secret_scanning_config,
            ui::commands::update_secret_scanning_config,
            ui::commands::test_secret_scan,
            ui::commands::get_secret_scanning_patterns,
            ui::commands_clients::get_client_secret_scanning_config,
            ui::commands_clients::update_client_secret_scanning_config,
            // Per-client JSON Repair commands
            ui::commands_clients::get_client_json_repair_config,
            ui::commands_clients::update_client_json_repair_config,
            // Prompt Compression commands
            ui::commands::get_client_compression_config,
            ui::commands::update_client_compression_config,
            ui::commands::get_compression_config,
            ui::commands::update_compression_config,
            ui::commands::get_compression_status,
            ui::commands::install_compression,
            ui::commands::rebuild_compression_engine,
            ui::commands::test_compression,
            // Embedding commands
            ui::commands::get_embedding_status,
            ui::commands::install_embedding_model,
            // JSON Repair commands
            ui::commands::get_json_repair_config,
            ui::commands::update_json_repair_config,
            ui::commands::test_json_repair,
            ui::commands::get_safety_model_status,
            ui::commands::test_safety_model,
            ui::commands::get_all_safety_categories,
            // Safety model management commands
            ui::commands::add_safety_model,
            ui::commands::remove_safety_model,
            ui::commands::toggle_safety_model,
            // Provider model pull commands
            ui::commands::pull_provider_model,
            // Connection graph commands
            ui::commands::get_active_connections,
            // Setup wizard commands
            ui::commands::get_setup_wizard_shown,
            ui::commands::set_setup_wizard_shown,
            // RouteLLM intelligent routing commands
            ui::commands_routellm::routellm_get_status,
            ui::commands_routellm::routellm_test_prediction,
            ui::commands_routellm::routellm_unload,
            ui::commands_routellm::routellm_download_models,
            ui::commands_routellm::routellm_update_settings,
            ui::commands_routellm::routellm_delete_model,
            ui::commands_routellm::open_routellm_folder,
            // Debug commands (dev only)
            ui::commands::debug_trigger_firewall_popup,
            ui::commands::debug_trigger_sampling_approval_popup,
            ui::commands::debug_trigger_elicitation_form_popup,
            ui::commands::debug_set_tray_overlay,
            ui::commands::debug_discover_providers,
            // Sampling approval commands
            ui::commands::get_sampling_approval_details,
            ui::commands::submit_sampling_approval,
            // Elicitation commands
            ui::commands::get_elicitation_details,
            ui::commands::submit_elicitation_response,
            ui::commands::cancel_elicitation,
            // File system commands
            ui::commands::open_path,
            // Skills commands
            ui::commands::list_skills,
            ui::commands::get_skill,
            ui::commands::get_context_management_config,
            ui::commands::update_context_management_config,
            ui::commands::set_gateway_indexing_permission,
            ui::commands::list_virtual_mcp_indexing_info,
            ui::commands::set_virtual_indexing_permission,
            ui::commands::get_known_client_tools,
            ui::commands::get_seen_client_tools,
            ui::commands::get_client_tools_indexing,
            ui::commands::set_client_tools_indexing,
            ui::commands::preview_catalog_compression,
            ui::commands::list_active_sessions,
            ui::commands::terminate_session,
            ui::commands::get_session_context_sources,
            ui::commands::get_session_context_stats,
            ui::commands::query_session_context_index,
            ui::commands::preview_rag_index,
            ui::commands::preview_rag_search,
            ui::commands::preview_rag_read,
            ui::commands::get_skills_config,
            ui::commands::update_skills_tool_names,
            ui::commands::add_skill_source,
            ui::commands::remove_skill_source,
            ui::commands::create_skill,
            ui::commands::is_user_created_skill,
            ui::commands::delete_user_skill,
            ui::commands::set_skill_enabled,
            ui::commands::rescan_skills,
            ui::commands::get_skill_files,
            // Marketplace commands
            ui::commands_marketplace::update_marketplace_tool_names,
            ui::commands_marketplace::marketplace_get_config,
            ui::commands_marketplace::marketplace_set_enabled,
            ui::commands_marketplace::marketplace_set_mcp_enabled,
            ui::commands_marketplace::marketplace_set_skills_enabled,
            ui::commands_marketplace::marketplace_set_registry_url,
            ui::commands_marketplace::marketplace_list_skill_sources,
            ui::commands_marketplace::marketplace_add_skill_source,
            ui::commands_marketplace::marketplace_remove_skill_source,
            ui::commands_marketplace::marketplace_add_default_skill_sources,
            ui::commands_marketplace::marketplace_reset_registry_url,
            ui::commands_marketplace::marketplace_get_cache_status,
            ui::commands_marketplace::marketplace_refresh_cache,
            ui::commands_marketplace::marketplace_clear_mcp_cache,
            ui::commands_marketplace::marketplace_clear_skills_cache,
            ui::commands_marketplace::marketplace_search_mcp_servers,
            ui::commands_marketplace::marketplace_search_skills,
            ui::commands_marketplace::marketplace_install_mcp_server_direct,
            ui::commands_marketplace::marketplace_install_skill_direct,
            ui::commands_marketplace::marketplace_delete_skill,
            ui::commands_marketplace::marketplace_is_skill_from_marketplace,
            ui::commands_marketplace::marketplace_get_pending_install,
            ui::commands_marketplace::marketplace_list_pending_installs,
            ui::commands_marketplace::marketplace_install_respond,
            ui::commands_marketplace::set_client_marketplace_enabled,
            ui::commands_marketplace::get_client_marketplace_enabled,
            ui::commands_marketplace::get_marketplace_tool_definitions,
            // Free tier commands
            ui::commands_free_tier::get_free_tier_status,
            ui::commands_free_tier::set_provider_free_tier,
            ui::commands_free_tier::reset_provider_free_tier_usage,
            ui::commands_free_tier::set_provider_free_tier_usage,
            ui::commands_free_tier::get_default_free_tier,
            // Coding agents commands
            ui::commands_coding_agents::list_coding_agents,
            ui::commands_coding_agents::list_coding_sessions,
            ui::commands_coding_agents::get_coding_session_detail,
            ui::commands_coding_agents::get_coding_agent_version,
            ui::commands_coding_agents::end_coding_session,
            ui::commands_coding_agents::get_max_coding_sessions,
            ui::commands_coding_agents::set_max_coding_sessions,
            ui::commands_coding_agents::set_client_coding_agent_permission,
            ui::commands_coding_agents::set_client_coding_agent_type,
            ui::commands_coding_agents::get_coding_agent_tool_definitions,
            ui::commands_coding_agents::get_context_mode_tool_definitions,
            ui::commands_coding_agents::get_coding_agent_approval_mode,
            ui::commands_coding_agents::set_coding_agent_approval_mode,
            ui::commands_coding_agents::get_coding_agent_tool_prefix,
            ui::commands_coding_agents::set_coding_agent_tool_prefix,
            ui::commands_coding_agents::submit_coding_agent_approval,
            // Clipboard commands
            ui::commands::copy_image_to_clipboard,
            ui::commands::copy_text_to_clipboard,
            // Monitor commands
            ui::commands_monitor::get_monitor_events,
            ui::commands_monitor::get_monitor_event_detail,
            ui::commands_monitor::clear_monitor_events,
            ui::commands_monitor::get_monitor_stats,
            ui::commands_monitor::set_monitor_max_capacity,
            ui::commands_monitor::set_monitor_intercept_rule,
            ui::commands_monitor::get_monitor_intercept_rule,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent the window from closing
                api.prevent_close();

                // Hide the window instead
                if let Err(e) = window.hide() {
                    tracing::error!("Failed to hide window: {}", e);
                }

                tracing::info!("Window close intercepted - app minimized to system tray");
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
