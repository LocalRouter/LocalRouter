//! Integration tests for the MCP via LLM agentic orchestrator.
//!
//! These tests exercise the full `run_agentic_loop` and `resume_after_mixed`
//! codepaths with mocked LLM provider and MCP virtual servers wired through
//! real `Router` and `McpGateway` instances.

#[cfg(test)]
mod helpers {
    use std::any::Any;
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::{json, Value};

    use lr_config::{AppConfig, Client, ClientMode, ConfigManager, McpViaLlmConfig};
    use lr_mcp::gateway::types::GatewayConfig;
    use lr_mcp::gateway::virtual_server::{
        VirtualFirewallResult, VirtualInstructions, VirtualMcpServer, VirtualSessionState,
        VirtualToolCallResult,
    };
    use lr_mcp::gateway::FirewallDecisionResult;
    use lr_mcp::McpGateway;
    use lr_mcp::McpServerManager;
    use lr_providers::factory::{ProviderCategory, ProviderFactory, SetupParameter};
    use lr_providers::registry::ProviderRegistry;
    use lr_providers::{
        Capability, ChatMessage, ChatMessageContent, CompletionChoice, CompletionRequest,
        CompletionResponse, FunctionCall, HealthStatus, ModelInfo, ModelProvider, ProviderHealth,
        TokenUsage, ToolCall,
    };
    use lr_router::free_tier::FreeTierManager;
    use lr_router::rate_limit::RateLimiterManager;
    use lr_router::Router;
    use lr_types::McpTool;

    // ── Mock LLM Provider ──────────────────────────────────────────────────

    pub struct MockLlmProvider {
        pub responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
        pub requests_received: Arc<Mutex<Vec<CompletionRequest>>>,
    }

    impl MockLlmProvider {
        pub fn new(responses: Vec<CompletionResponse>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(VecDeque::from(responses))),
                requests_received: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl ModelProvider for MockLlmProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn health_check(&self) -> ProviderHealth {
            ProviderHealth {
                status: HealthStatus::Healthy,
                latency_ms: None,
                last_checked: chrono::Utc::now(),
                error_message: None,
            }
        }

        async fn list_models(&self) -> lr_types::AppResult<Vec<ModelInfo>> {
            Ok(vec![ModelInfo {
                id: "test-model".to_string(),
                name: "Test Model".to_string(),
                provider: "mock".to_string(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: false,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            }])
        }

        async fn get_pricing(
            &self,
            _model: &str,
        ) -> lr_types::AppResult<lr_providers::PricingInfo> {
            Err(lr_types::AppError::Provider("not implemented".into()))
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> lr_types::AppResult<CompletionResponse> {
            self.requests_received.lock().push(request);
            let response =
                self.responses.lock().pop_front().ok_or_else(|| {
                    lr_types::AppError::Provider("no more scripted responses".into())
                })?;
            Ok(response)
        }

        async fn stream_complete(
            &self,
            _request: CompletionRequest,
        ) -> lr_types::AppResult<
            std::pin::Pin<
                Box<
                    dyn futures::Stream<Item = lr_types::AppResult<lr_providers::CompletionChunk>>
                        + Send,
                >,
            >,
        > {
            Err(lr_types::AppError::Provider(
                "streaming not supported in mock".into(),
            ))
        }
    }

    // ── Mock Provider Factory ──────────────────────────────────────────────

    pub struct MockProviderFactory {
        provider: Arc<dyn ModelProvider>,
    }

    impl MockProviderFactory {
        pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
            Self { provider }
        }
    }

    #[async_trait]
    impl ProviderFactory for MockProviderFactory {
        fn provider_type(&self) -> &str {
            "mock"
        }

        fn display_name(&self) -> &str {
            "Mock Provider"
        }

        fn category(&self) -> ProviderCategory {
            ProviderCategory::Local
        }

        fn description(&self) -> &str {
            "Mock provider for testing"
        }

        fn setup_parameters(&self) -> Vec<SetupParameter> {
            vec![]
        }

        fn create(
            &self,
            _instance_name: String,
            _config: HashMap<String, String>,
        ) -> lr_types::AppResult<Arc<dyn ModelProvider>> {
            Ok(self.provider.clone())
        }

        fn validate_config(&self, _config: &HashMap<String, String>) -> lr_types::AppResult<()> {
            Ok(())
        }
    }

    // ── Mock Virtual Session State ─────────────────────────────────────────

    #[derive(Clone)]
    pub struct MockSessionState;

    impl VirtualSessionState for MockSessionState {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn VirtualSessionState> {
            Box::new(self.clone())
        }
    }

    // ── Mock MCP Virtual Server ────────────────────────────────────────────

    pub struct MockMcpVirtualServer {
        pub server_id: String,
        pub tools: Vec<McpTool>,
        pub tool_results: Arc<Mutex<HashMap<String, Value>>>,
        pub calls_received: Arc<Mutex<Vec<(String, Value)>>>,
        pub error_results: Arc<Mutex<HashMap<String, String>>>,
    }

    impl MockMcpVirtualServer {
        pub fn new(
            server_id: &str,
            tools: Vec<McpTool>,
            tool_results: HashMap<String, Value>,
        ) -> Self {
            Self {
                server_id: server_id.to_string(),
                tools,
                tool_results: Arc::new(Mutex::new(tool_results)),
                calls_received: Arc::new(Mutex::new(Vec::new())),
                error_results: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        pub fn with_errors(mut self, errors: HashMap<String, String>) -> Self {
            self.error_results = Arc::new(Mutex::new(errors));
            self
        }
    }

    #[async_trait]
    impl VirtualMcpServer for MockMcpVirtualServer {
        fn id(&self) -> &str {
            &self.server_id
        }

        fn display_name(&self) -> &str {
            &self.server_id
        }

        fn owns_tool(&self, tool_name: &str) -> bool {
            self.tools.iter().any(|t| t.name == tool_name)
        }

        fn is_enabled(&self, _client: &Client) -> bool {
            true
        }

        fn list_tools(&self, _state: &dyn VirtualSessionState) -> Vec<McpTool> {
            self.tools.clone()
        }

        fn check_permissions(
            &self,
            _state: &dyn VirtualSessionState,
            _tool_name: &str,
            _arguments: Option<&Value>,
            _session_approved: bool,
            _session_denied: bool,
        ) -> VirtualFirewallResult {
            VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
        }

        async fn handle_tool_call(
            &self,
            _state: Box<dyn VirtualSessionState>,
            tool_name: &str,
            arguments: Value,
            _client_id: &str,
            _client_name: &str,
        ) -> VirtualToolCallResult {
            self.calls_received
                .lock()
                .push((tool_name.to_string(), arguments.clone()));

            // Check for error results first
            if let Some(error) = self.error_results.lock().get(tool_name) {
                return VirtualToolCallResult::ToolError(error.clone());
            }

            if let Some(result) = self.tool_results.lock().get(tool_name) {
                VirtualToolCallResult::Success(result.clone())
            } else {
                VirtualToolCallResult::Success(json!({
                    "content": [{"type": "text", "text": format!("result for {}", tool_name)}]
                }))
            }
        }

        fn build_instructions(
            &self,
            _state: &dyn VirtualSessionState,
        ) -> Option<VirtualInstructions> {
            None
        }

        fn create_session_state(&self, _client: &Client) -> Box<dyn VirtualSessionState> {
            Box::new(MockSessionState)
        }

        fn update_session_state(&self, _state: &mut dyn VirtualSessionState, _client: &Client) {}

        fn all_tool_names(&self) -> Vec<String> {
            self.tools.iter().map(|t| t.name.clone()).collect()
        }
    }

    // ── Test Environment ───────────────────────────────────────────────────

    pub struct TestEnv {
        pub gateway: Arc<McpGateway>,
        pub router: Arc<Router>,
        pub client: Client,
        pub mock_provider: Arc<MockLlmProvider>,
        pub mock_servers: Vec<Arc<MockMcpVirtualServer>>,
    }

    pub async fn setup_test_env(
        llm_responses: Vec<CompletionResponse>,
        mcp_tools: Vec<McpTool>,
        tool_results: HashMap<String, Value>,
    ) -> TestEnv {
        setup_test_env_multi(
            llm_responses,
            vec![("_test_server", mcp_tools, tool_results)],
        )
        .await
    }

    pub async fn setup_test_env_multi(
        llm_responses: Vec<CompletionResponse>,
        servers: Vec<(&str, Vec<McpTool>, HashMap<String, Value>)>,
    ) -> TestEnv {
        setup_test_env_multi_with_mock_servers(llm_responses, servers, vec![]).await
    }

    pub async fn setup_test_env_multi_with_mock_servers(
        llm_responses: Vec<CompletionResponse>,
        servers: Vec<(&str, Vec<McpTool>, HashMap<String, Value>)>,
        extra_mock_servers: Vec<Arc<MockMcpVirtualServer>>,
    ) -> TestEnv {
        // 1. Config manager
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            PathBuf::from("/tmp/lr-integration-test.yaml"),
        ));

        // 2. Provider registry with mock factory
        let mock_provider = Arc::new(MockLlmProvider::new(llm_responses));
        let mock_factory = Arc::new(MockProviderFactory::new(mock_provider.clone()));

        let registry = Arc::new(ProviderRegistry::new());
        registry.register_factory(mock_factory);
        registry
            .create_provider("mock".to_string(), "mock".to_string(), HashMap::new())
            .await
            .expect("Failed to create mock provider");

        // 3. Rate limiter and metrics
        let rate_limiter = Arc::new(RateLimiterManager::new(None));
        let temp_dir =
            std::env::temp_dir().join(format!("lr-test-metrics-{}", uuid::Uuid::new_v4()));
        let metrics_db = Arc::new(
            lr_monitoring::storage::MetricsDatabase::new(temp_dir)
                .expect("Failed to create metrics DB"),
        );
        let metrics = Arc::new(lr_monitoring::metrics::MetricsCollector::new(metrics_db));

        // 4. Router
        let free_tier = Arc::new(FreeTierManager::new(None));
        let router = Arc::new(Router::new(
            config_manager,
            registry,
            rate_limiter,
            metrics,
            free_tier,
        ));

        // 5. MCP gateway with virtual servers
        let manager = Arc::new(McpServerManager::new_for_test());
        let gateway = Arc::new(McpGateway::new(
            manager,
            GatewayConfig::default(),
            router.clone(),
        ));

        let mut mock_servers = Vec::new();
        for (server_id, tools, results) in servers {
            let mock_server = Arc::new(MockMcpVirtualServer::new(server_id, tools, results));
            gateway.register_virtual_server(mock_server.clone());
            mock_servers.push(mock_server);
        }
        for server in &extra_mock_servers {
            gateway.register_virtual_server(server.clone());
            mock_servers.push(server.clone());
        }

        // 6. Client
        let mut client =
            Client::new_with_strategy("test-client".to_string(), "default".to_string());
        client.id = "internal-test".to_string();
        client.client_mode = ClientMode::McpViaLlm;

        TestEnv {
            gateway,
            router,
            client,
            mock_provider,
            mock_servers,
        }
    }

    // ── Helper functions ───────────────────────────────────────────────────

    pub fn make_response(
        text: Option<&str>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> CompletionResponse {
        CompletionResponse {
            id: uuid::Uuid::new_v4().to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "mock/test-model".to_string(),
            provider: "mock".to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatMessageContent::Text(text.unwrap_or("").to_string()),
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some(if text.is_some() {
                    "stop".to_string()
                } else {
                    "tool_calls".to_string()
                }),
                logprobs: None,
            }],
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        }
    }

    pub fn make_tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            tool_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
        }
    }

    pub fn make_mcp_tool(name: &str, description: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: Some(description.to_string()),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    pub fn make_request(user_message: &str) -> CompletionRequest {
        CompletionRequest {
            model: "mock/test-model".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text(user_message.to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: None,
            max_tokens: None,
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
            reasoning_effort: None,
            pre_computed_routing: None,
        }
    }

    pub fn make_config() -> McpViaLlmConfig {
        McpViaLlmConfig {
            session_ttl_seconds: 3600,
            max_concurrent_sessions: 100,
            max_loop_iterations: 10,
            max_loop_timeout_seconds: 300,
            expose_resources_as_tools: false,
            inject_prompts: false,
        }
    }

    pub fn make_session() -> Arc<parking_lot::RwLock<crate::session::McpViaLlmSession>> {
        Arc::new(parking_lot::RwLock::new(
            crate::session::McpViaLlmSession::new(
                uuid::Uuid::new_v4().to_string(),
                "internal-test".to_string(),
            ),
        ))
    }
}

// ── Agentic Loop Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod agentic_loop_tests {
    use super::helpers::*;
    use crate::orchestrator::{run_agentic_loop, OrchestratorResult};
    use std::collections::HashMap;

    #[tokio::test]
    async fn passthrough_no_mcp_tools() {
        let env = setup_test_env(
            vec![make_response(Some("Hello!"), None)],
            vec![], // No MCP tools
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Hi there");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                let text = &resp.choices[0].message.content;
                assert!(
                    matches!(text, lr_providers::ChatMessageContent::Text(t) if t == "Hello!"),
                    "Expected passthrough response"
                );
            }
            OrchestratorResult::PendingMixed { .. } => {
                panic!("Expected Complete, got PendingMixed")
            }
        }

        // LLM called exactly once
        assert_eq!(env.mock_provider.requests_received.lock().len(), 1);
    }

    #[tokio::test]
    async fn single_mcp_tool_iteration() {
        let tool = make_mcp_tool("fs__read", "Read a file");
        let mut results = HashMap::new();
        results.insert(
            "fs__read".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "file contents"}]}),
        );

        let env = setup_test_env(
            vec![
                // First LLM call: returns a tool call
                make_response(
                    None,
                    Some(vec![make_tool_call(
                        "call-1",
                        "fs__read",
                        r#"{"path":"/tmp"}"#,
                    )]),
                ),
                // Second LLM call: final text
                make_response(Some("The file contains: file contents"), None),
            ],
            vec![tool],
            results,
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read /tmp");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                let text = &resp.choices[0].message.content;
                assert!(matches!(
                    text,
                    lr_providers::ChatMessageContent::Text(t) if t.contains("file contents")
                ));
                // Usage should be aggregated across 2 iterations
                assert_eq!(resp.usage.prompt_tokens, 20); // 10 + 10
                assert_eq!(resp.usage.completion_tokens, 10); // 5 + 5
            }
            _ => panic!("Expected Complete"),
        }

        // LLM called twice
        assert_eq!(env.mock_provider.requests_received.lock().len(), 2);
    }

    #[tokio::test]
    async fn multiple_mcp_tools_in_one_turn() {
        let tools = vec![
            make_mcp_tool("fs__read", "Read a file"),
            make_mcp_tool("fs__write", "Write a file"),
            make_mcp_tool("fs__list", "List files"),
        ];

        let env = setup_test_env(
            vec![
                // First LLM call: 3 tool calls
                make_response(
                    None,
                    Some(vec![
                        make_tool_call("c1", "fs__read", "{}"),
                        make_tool_call("c2", "fs__write", "{}"),
                        make_tool_call("c3", "fs__list", "{}"),
                    ]),
                ),
                // Second LLM call: final text
                make_response(Some("Done with all three tools"), None),
            ],
            tools,
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Do file operations");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                let text = &resp.choices[0].message.content;
                assert!(matches!(
                    text,
                    lr_providers::ChatMessageContent::Text(t) if t.contains("all three")
                ));
            }
            _ => panic!("Expected Complete"),
        }

        // The second LLM call should have tool result messages for all 3 tools
        let requests = env.mock_provider.requests_received.lock();
        assert_eq!(requests.len(), 2);
        let tool_messages: Vec<_> = requests[1]
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .collect();
        assert_eq!(tool_messages.len(), 3);
    }

    #[tokio::test]
    async fn client_only_tools_returned_directly() {
        let mcp_tools = vec![make_mcp_tool("fs__read", "Read a file")];

        let env = setup_test_env(
            vec![
                // LLM returns only client tools (not in MCP set)
                make_response(
                    None,
                    Some(vec![make_tool_call("c1", "my_client_tool", "{}")]),
                ),
            ],
            mcp_tools,
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Use client tool");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                let tc = resp.choices[0].message.tool_calls.as_ref().unwrap();
                assert_eq!(tc.len(), 1);
                assert_eq!(tc[0].function.name, "my_client_tool");
            }
            _ => panic!("Expected Complete with client tools"),
        }

        // MCP server should not have received any calls
        assert!(env.mock_servers[0].calls_received.lock().is_empty());
    }

    #[tokio::test]
    async fn tool_injection_verified_in_request() {
        let tools = vec![
            make_mcp_tool("fs__read", "Read a file"),
            make_mcp_tool("db__query", "Query database"),
        ];

        let env = setup_test_env(
            vec![make_response(Some("Final answer"), None)],
            tools,
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Hello");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        // Verify the request sent to LLM includes MCP tools
        let requests = env.mock_provider.requests_received.lock();
        assert_eq!(requests.len(), 1);
        let tools = requests[0]
            .tools
            .as_ref()
            .expect("tools should be injected");
        let tool_names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert!(tool_names.contains(&"fs__read"));
        assert!(tool_names.contains(&"db__query"));
    }
}

// ── MCP Server Verification Tests ──────────────────────────────────────────

#[cfg(test)]
mod mcp_server_verification_tests {
    use super::helpers::*;
    use crate::orchestrator::run_agentic_loop;
    use std::collections::HashMap;

    #[tokio::test]
    async fn verify_mcp_server_receives_tool_call() {
        let tool = make_mcp_tool("fs__read", "Read a file");
        let mut results = HashMap::new();
        results.insert(
            "fs__read".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "file data"}]}),
        );

        let env = setup_test_env(
            vec![
                make_response(
                    None,
                    Some(vec![make_tool_call(
                        "call-1",
                        "fs__read",
                        r#"{"path":"/etc/hosts"}"#,
                    )]),
                ),
                make_response(Some("Done"), None),
            ],
            vec![tool],
            results,
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read hosts file");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        let calls = env.mock_servers[0].calls_received.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "fs__read");
        assert_eq!(calls[0].1, serde_json::json!({"path": "/etc/hosts"}));
    }

    #[tokio::test]
    async fn multiple_mcp_servers_different_tools() {
        let fs_tools = vec![make_mcp_tool("fs__read", "Read a file")];
        let db_tools = vec![make_mcp_tool("db__query", "Query database")];

        let mut fs_results = HashMap::new();
        fs_results.insert(
            "fs__read".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "config data"}]}),
        );
        let mut db_results = HashMap::new();
        db_results.insert(
            "db__query".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "query results"}]}),
        );

        let env = setup_test_env_multi(
            vec![
                make_response(
                    None,
                    Some(vec![
                        make_tool_call("c1", "fs__read", r#"{"path":"/config"}"#),
                        make_tool_call("c2", "db__query", r#"{"sql":"SELECT 1"}"#),
                    ]),
                ),
                make_response(Some("Both operations complete"), None),
            ],
            vec![
                ("_fs_server", fs_tools, fs_results),
                ("_db_server", db_tools, db_results),
            ],
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read config and query DB");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        // Each server should have received exactly one call
        let fs_calls = env.mock_servers[0].calls_received.lock();
        assert_eq!(fs_calls.len(), 1);
        assert_eq!(fs_calls[0].0, "fs__read");

        let db_calls = env.mock_servers[1].calls_received.lock();
        assert_eq!(db_calls.len(), 1);
        assert_eq!(db_calls[0].0, "db__query");
    }

    #[tokio::test]
    async fn mcp_server_call_order_matches_llm_response() {
        let tools = vec![
            make_mcp_tool("tool_a", "Tool A"),
            make_mcp_tool("tool_b", "Tool B"),
            make_mcp_tool("tool_c", "Tool C"),
        ];

        let env = setup_test_env(
            vec![
                make_response(
                    None,
                    Some(vec![
                        make_tool_call("c1", "tool_a", "{}"),
                        make_tool_call("c2", "tool_b", "{}"),
                        make_tool_call("c3", "tool_c", "{}"),
                    ]),
                ),
                make_response(Some("Done"), None),
            ],
            tools,
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Call tools");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        let calls = env.mock_servers[0].calls_received.lock();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "tool_a");
        assert_eq!(calls[1].0, "tool_b");
        assert_eq!(calls[2].0, "tool_c");
    }

    #[tokio::test]
    async fn mcp_server_receives_correct_arguments() {
        let tool = make_mcp_tool("fs__read", "Read a file");
        let env = setup_test_env(
            vec![
                make_response(
                    None,
                    Some(vec![make_tool_call(
                        "c1",
                        "fs__read",
                        r#"{"path":"/tmp/test","encoding":"utf-8"}"#,
                    )]),
                ),
                make_response(Some("Done"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read file");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        let calls = env.mock_servers[0].calls_received.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            serde_json::json!({"path": "/tmp/test", "encoding": "utf-8"})
        );
    }
}

// ── Mixed Tool Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod mixed_tool_tests {
    use super::helpers::*;
    use crate::orchestrator::{resume_after_mixed, run_agentic_loop, OrchestratorResult};
    use lr_providers::{ChatMessage, ChatMessageContent};
    use std::collections::HashMap;

    #[tokio::test]
    async fn mixed_mcp_and_client_tools() {
        let tool = make_mcp_tool("fs__read", "Read a file");
        let mut results = HashMap::new();
        results.insert(
            "fs__read".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "file data"}]}),
        );

        let env = setup_test_env(
            vec![
                // LLM returns both MCP and client tool calls
                make_response(
                    None,
                    Some(vec![
                        make_tool_call("mcp-1", "fs__read", r#"{"path":"/tmp"}"#),
                        make_tool_call("client-1", "my_tool", r#"{"key":"val"}"#),
                    ]),
                ),
            ],
            vec![tool],
            results,
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Mixed tools");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::PendingMixed {
                client_response,
                pending,
            } => {
                // Client response should only contain client tools
                let tc = client_response.choices[0]
                    .message
                    .tool_calls
                    .as_ref()
                    .unwrap();
                assert_eq!(tc.len(), 1);
                assert_eq!(tc[0].function.name, "my_tool");
                assert_eq!(tc[0].id, "client-1");

                // Pending should have the client tool call ID
                assert_eq!(pending.client_tool_call_ids, vec!["client-1"]);
            }
            _ => panic!("Expected PendingMixed"),
        }
    }

    #[tokio::test]
    async fn resume_after_mixed_execution() {
        let tool = make_mcp_tool("fs__read", "Read a file");
        let mut results = HashMap::new();
        results.insert(
            "fs__read".to_string(),
            serde_json::json!({"content": [{"type": "text", "text": "file data"}]}),
        );

        let env = setup_test_env(
            vec![
                // First call: mixed tools
                make_response(
                    None,
                    Some(vec![
                        make_tool_call("mcp-1", "fs__read", r#"{"path":"/tmp"}"#),
                        make_tool_call("client-1", "my_tool", "{}"),
                    ]),
                ),
                // After resume: no more tools available (already fetched above),
                // so we need the resumed loop to find MCP tools again.
                // The resumed loop will list tools, inject them, and call LLM again.
                make_response(Some("All done!"), None),
            ],
            vec![tool],
            results,
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Mixed tools");

        // Phase 1: get PendingMixed
        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session.clone(),
            request.clone(),
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("phase 1 should succeed");

        let pending = match result {
            OrchestratorResult::PendingMixed { pending, .. } => pending,
            _ => panic!("Expected PendingMixed"),
        };

        // Phase 2: resume with client tool results
        let client_tool_results = vec![ChatMessage {
            role: "tool".to_string(),
            content: ChatMessageContent::Text("client result".to_string()),
            tool_calls: None,
            tool_call_id: Some("client-1".to_string()),
            name: None,
        }];

        let cm_config = lr_config::ContextManagementConfig::default();
        let resume_result = resume_after_mixed(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            pending,
            request,
            client_tool_results,
            &config,
            vec![],
            &cm_config,
        )
        .await
        .expect("resume should succeed");

        match resume_result {
            OrchestratorResult::Complete(resp) => {
                let text = &resp.choices[0].message.content;
                assert!(matches!(
                    text,
                    lr_providers::ChatMessageContent::Text(t) if t == "All done!"
                ));
            }
            _ => panic!("Expected Complete after resume"),
        }

        // MCP server should have received the tool call
        let calls = env.mock_servers[0].calls_received.lock();
        assert!(calls.iter().any(|(name, _)| name == "fs__read"));
    }
}

// ── Guardrail Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod guardrail_tests {
    use super::helpers::*;
    use crate::manager::McpViaLlmError;
    use crate::orchestrator::{run_agentic_loop, OrchestratorResult};
    use std::collections::HashMap;

    #[tokio::test]
    async fn guardrail_pass() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![make_response(Some("Safe response"), None)],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Hello");

        // Guardrail gate that resolves Ok
        let gate: crate::manager::GuardrailGate = tokio::spawn(async { Ok(()) });

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            Some(gate),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                assert!(matches!(
                    &resp.choices[0].message.content,
                    lr_providers::ChatMessageContent::Text(t) if t == "Safe response"
                ));
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[tokio::test]
    async fn guardrail_deny() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                // LLM returns a tool call, but guardrail will deny before execution
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Bad request");

        // Guardrail gate that denies
        let gate: crate::manager::GuardrailGate =
            tokio::spawn(async { Err("Content policy violation".to_string()) });

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            Some(gate),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        match result {
            Err(McpViaLlmError::GuardrailDenied(msg)) => {
                assert!(msg.contains("Content policy violation"));
            }
            Ok(_) => panic!("Expected GuardrailDenied, got Ok"),
            Err(other) => panic!("Expected GuardrailDenied, got: {:?}", other),
        }

        // MCP server should not have received any calls
        assert!(env.mock_servers[0].calls_received.lock().is_empty());
    }

    #[tokio::test]
    async fn guardrail_checked_once_across_iterations() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                // First call: tool call
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                // Second call: another tool call (guardrail should already be consumed)
                make_response(None, Some(vec![make_tool_call("c2", "fs__read", "{}")])),
                // Third call: final response
                make_response(Some("Done"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Multi-iteration");

        // Guardrail resolves Ok
        let gate: crate::manager::GuardrailGate = tokio::spawn(async { Ok(()) });

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            Some(gate),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(_) => {}
            _ => panic!("Expected Complete"),
        }

        // LLM called 3 times, guardrail only consumed once (no panic from double-poll)
        assert_eq!(env.mock_provider.requests_received.lock().len(), 3);
    }
}

// ── Error Handling Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod error_handling_tests {
    use super::helpers::*;
    use crate::manager::McpViaLlmError;
    use crate::orchestrator::{run_agentic_loop, OrchestratorResult};
    use lr_config::McpViaLlmConfig;
    use std::collections::HashMap;

    #[tokio::test]
    async fn max_iterations_limit() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        // LLM always returns tool calls — should hit iteration limit
        let env = setup_test_env(
            vec![
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                make_response(None, Some(vec![make_tool_call("c2", "fs__read", "{}")])),
                make_response(None, Some(vec![make_tool_call("c3", "fs__read", "{}")])),
                // Extra response in case loop somehow continues
                make_response(Some("Should not reach"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = McpViaLlmConfig {
            max_loop_iterations: 2,
            ..make_config()
        };
        let request = make_request("Loop forever");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        match result {
            Err(McpViaLlmError::MaxIterations(limit)) => {
                assert_eq!(limit, 2);
            }
            Ok(_) => panic!("Expected MaxIterations, got Ok"),
            Err(other) => panic!("Expected MaxIterations, got: {:?}", other),
        }

        // Mock server should have received tool calls for all iterations
        let calls = env.mock_servers[0].calls_received.lock();
        assert_eq!(calls.len(), 2);
    }

    #[tokio::test]
    async fn tool_error_fed_back_to_llm() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let mock_server = std::sync::Arc::new(
            MockMcpVirtualServer::new("_test_server", vec![tool], HashMap::new()).with_errors(
                HashMap::from([("fs__read".to_string(), "disk full".to_string())]),
            ),
        );

        let env = setup_test_env_multi_with_mock_servers(
            vec![
                // First call: tool call
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                // Second call: LLM sees the error and gives a text response
                make_response(Some("Sorry, the disk is full"), None),
            ],
            vec![], // No default servers
            vec![mock_server.clone()],
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read file");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed despite tool error");

        match result {
            OrchestratorResult::Complete(resp) => {
                assert!(matches!(
                    &resp.choices[0].message.content,
                    lr_providers::ChatMessageContent::Text(t) if t.contains("disk is full")
                ));
            }
            _ => panic!("Expected Complete"),
        }

        // Check that the error was forwarded to LLM
        let requests = env.mock_provider.requests_received.lock();
        assert_eq!(requests.len(), 2);
        // Second request should contain a tool result with the error
        let tool_msgs: Vec<_> = requests[1]
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        let content = match &tool_msgs[0].content {
            lr_providers::ChatMessageContent::Text(t) => t.clone(),
            _ => panic!("Expected text content"),
        };
        assert!(
            content.contains("Error") || content.contains("error") || content.contains("disk full"),
            "Tool error should be in the message: {}",
            content
        );
    }

    #[tokio::test]
    async fn malformed_tool_arguments() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                // LLM returns tool call with invalid JSON arguments
                make_response(
                    None,
                    Some(vec![make_tool_call("c1", "fs__read", "not valid json{{{")]),
                ),
                // LLM sees the parse error and retries with valid args (or gives text)
                make_response(Some("I see the arguments were malformed"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Read file");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(_) => {}
            _ => panic!("Expected Complete"),
        }

        // MCP server should NOT have received any calls (invalid JSON never reaches it)
        assert!(env.mock_servers[0].calls_received.lock().is_empty());

        // The LLM should have received a tool result message with the parse error
        let requests = env.mock_provider.requests_received.lock();
        assert_eq!(requests.len(), 2);
        let tool_msgs: Vec<_> = requests[1]
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        let content = match &tool_msgs[0].content {
            lr_providers::ChatMessageContent::Text(t) => t.clone(),
            _ => panic!("Expected text content"),
        };
        assert!(
            content.contains("invalid JSON") || content.contains("Error"),
            "Parse error should be in the message: {}",
            content
        );
    }
}

// ── Metadata Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod metadata_tests {
    use super::helpers::*;
    use crate::orchestrator::{run_agentic_loop, OrchestratorResult};
    use std::collections::HashMap;

    #[tokio::test]
    async fn token_aggregation() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                make_response(Some("Final"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Aggregate tokens");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                // 2 iterations × 10 prompt tokens = 20
                assert_eq!(resp.usage.prompt_tokens, 20);
                // 2 iterations × 5 completion tokens = 10
                assert_eq!(resp.usage.completion_tokens, 10);
                assert_eq!(resp.usage.total_tokens, 30);
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[tokio::test]
    async fn mcp_via_llm_extension_metadata() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                make_response(Some("Final"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Metadata test");

        let result = run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session,
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        match result {
            OrchestratorResult::Complete(resp) => {
                let extensions = resp.extensions.expect("should have extensions");
                let mcp_meta = extensions
                    .get("mcp_via_llm")
                    .expect("should have mcp_via_llm");

                assert_eq!(mcp_meta["iterations"], 2);
                let tools_called = mcp_meta["mcp_tools_called"].as_array().unwrap();
                assert_eq!(tools_called.len(), 1);
                assert_eq!(tools_called[0], "fs__read");
                assert_eq!(mcp_meta["total_prompt_tokens"], 20);
                assert_eq!(mcp_meta["total_completion_tokens"], 10);
            }
            _ => panic!("Expected Complete"),
        }
    }
}

// ── Session Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod session_tests {
    use super::helpers::*;
    use crate::orchestrator::run_agentic_loop;
    use std::collections::HashMap;

    #[tokio::test]
    async fn session_history_tracking() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![
                make_response(None, Some(vec![make_tool_call("c1", "fs__read", "{}")])),
                make_response(Some("Final"), None),
            ],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Track history");

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session.clone(),
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        let s = session.read();
        // History should include: gateway instructions (system), user msg, assistant (with tool call), tool result, final assistant
        assert!(
            s.history.full_messages.len() >= 4,
            "Expected at least 4 messages in history, got {}",
            s.history.full_messages.len()
        );
        // First message should be the injected gateway instructions (system)
        assert_eq!(s.history.full_messages[0].role, "system");
        // Second message should be user
        assert_eq!(s.history.full_messages[1].role, "user");
    }

    #[tokio::test]
    async fn gateway_initialized_flag() {
        let tool = make_mcp_tool("fs__read", "Read a file");

        let env = setup_test_env(
            vec![make_response(Some("Hello"), None)],
            vec![tool],
            HashMap::new(),
        )
        .await;

        let session = make_session();
        let config = make_config();
        let request = make_request("Init test");

        // Before the loop, gateway_initialized should be false
        assert!(!session.read().gateway_initialized);

        run_agentic_loop(
            env.gateway.clone(),
            &env.router,
            &env.client,
            session.clone(),
            request,
            &config,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should succeed");

        // After the loop, gateway_initialized should be true
        assert!(session.read().gateway_initialized);
    }
}
