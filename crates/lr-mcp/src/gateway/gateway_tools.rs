use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTool,
};
use lr_types::{AppError, AppResult};

use super::deferred::create_search_tool;
use super::deferred::create_server_info_tool;
use super::deferred::{search_prompts, search_resources, search_tools, SearchMode};
use super::merger::merge_tools;
use super::router::{broadcast_request, separate_results};
use super::session::GatewaySession;
use super::types::*;

use super::access_control::{self, FirewallCheckContext, FirewallCheckResult};
use super::firewall::{self, FirewallApprovalAction};
use super::gateway::McpGateway;

/// Result of a firewall access decision check
pub enum FirewallDecisionResult {
    /// Proceed with the original request unchanged
    Proceed,
    /// Proceed but apply edits from the user
    ProceedWithEdits {
        edited_arguments: Option<serde_json::Value>,
    },
    /// Block the request with this error response
    Blocked(JsonRpcResponse),
}

impl McpGateway {
    /// Handle tools/list request
    pub(crate) async fn handle_tools_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check for deferred loading
        tracing::info!(
            "handle_tools_list: deferred_loading={:?}, deferred_loading_requested={}, cached_tools_valid={}",
            session_read.deferred_loading.as_ref().map(|d| d.enabled),
            session_read.deferred_loading_requested,
            session_read.cached_tools.as_ref().is_some_and(|c| c.is_valid()),
        );

        if let Some(deferred) = &session_read.deferred_loading {
            if deferred.enabled {
                // Return search tool + server_info tool + activated tools + virtual server tools
                let mut tools: Vec<serde_json::Value> = vec![
                    serde_json::to_value(create_search_tool(
                        deferred.resources_deferred,
                        deferred.prompts_deferred,
                    ))
                    .unwrap_or_default(),
                    serde_json::to_value(create_server_info_tool()).unwrap_or_default(),
                ];

                for tool_name in &deferred.activated_tools {
                    if let Some(tool) = deferred.full_catalog.iter().find(|t| t.name == *tool_name)
                    {
                        tools.push(serde_json::to_value(tool).unwrap_or_default());
                    }
                }

                // Append tools from virtual servers
                self.append_virtual_server_tools(&mut tools, &session_read);
                drop(session_read);

                let tool_names: Vec<String> = tools
                    .iter()
                    .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect();
                tracing::info!(
                    "handle_tools_list DEFERRED: returning {} tools: {:?}",
                    tools.len(),
                    tool_names,
                );

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"tools": tools}),
                ));
            }
        }

        // Check cache
        if let Some(cached) = &session_read.cached_tools {
            if cached.is_valid() {
                let mut tools: Vec<serde_json::Value> = cached
                    .data
                    .iter()
                    .map(|t| serde_json::to_value(t).unwrap_or_default())
                    .collect();

                // Append tools from virtual servers
                self.append_virtual_server_tools(&mut tools, &session_read);
                drop(session_read);

                tracing::info!("handle_tools_list CACHED: returning {} tools", tools.len(),);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"tools": tools}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Fetch from servers
        let (tools, failures) = self
            .fetch_and_merge_tools(&allowed_servers, request.clone())
            .await?;

        // Update session mappings, cache, and failures
        {
            let mut session_write = session.write().await;
            session_write.update_tool_mappings(&tools);
            session_write.last_broadcast_failures = failures;

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_tools = Some(CachedList::new(tools.clone(), cache_ttl));
        }

        // Check if there were any failures during fetch
        let session_read = session.read().await;
        let has_failures = !session_read.last_broadcast_failures.is_empty();
        let failures = if has_failures {
            Some(session_read.last_broadcast_failures.clone())
        } else {
            None
        };
        let mut all_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or_default())
            .collect();

        // Append tools from virtual servers
        self.append_virtual_server_tools(&mut all_tools, &session_read);
        drop(session_read);

        let mut result = json!({"tools": all_tools});
        if let Some(failures) = failures {
            result["_meta"] = json!({
                "partial_failure": true,
                "failures": failures
            });
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Fetch and merge tools from servers
    pub(crate) async fn fetch_and_merge_tools(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedTool>, Vec<ServerFailure>)> {
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            server_ids,
            request,
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        let (successes, failures) = separate_results(results);

        // If all servers failed, return error
        if successes.is_empty() && !failures.is_empty() {
            let error_summary = failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All servers failed to respond: {}",
                error_summary
            )));
        }

        // Parse tools from results
        let server_tools: Vec<(String, Vec<McpTool>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("tools")
                    .and_then(|tools| serde_json::from_value::<Vec<McpTool>>(tools.clone()).ok())
                    .map(|tools| (server_id, tools))
            })
            .collect();

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_tools(server_tools, &failures, Some(&name_mapping)),
            failures,
        ))
    }

    /// Handle tools/call request
    pub(crate) async fn handle_tools_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract tool name from params
        let tool_name = match request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(name) => name.to_string(),
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing tool name in params"),
                ));
            }
        };

        // Check if it's the virtual search tool
        if tool_name == "search" {
            return self.handle_search_tool(session, request).await;
        }

        // Check if it's the virtual server_info tool
        if tool_name == "server_info" {
            return self.handle_server_info_tool(session, request).await;
        }

        // Check virtual servers
        let matching_vs = {
            let virtual_servers = self.virtual_servers.read();
            virtual_servers
                .iter()
                .find(|vs| vs.owns_tool(&tool_name))
                .cloned()
        };
        if let Some(vs) = matching_vs {
            return self
                .dispatch_virtual_tool_call(vs, session, &tool_name, request)
                .await;
        }

        // Look up tool in session mapping to get server_id (UUID) and original_name
        // The mapping stores: namespaced_name -> (server_id, original_name)
        // where namespaced_name uses human-readable server name but server_id is the UUID for routing
        let session_read = session.read().await;
        let (server_id, original_name) = match session_read.tool_mapping.get(&tool_name) {
            Some((id, name)) => (id.clone(), name.clone()),
            None => {
                drop(session_read);
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::tool_not_found(&tool_name),
                ));
            }
        };
        drop(session_read);

        // Firewall check for MCP tools
        // Pass both namespaced name (for session tracking/display) and original name (for permission lookup)
        let firewall_result = self
            .check_firewall_mcp_tool(&session, &tool_name, &original_name, &server_id, &request)
            .await?;

        if let FirewallDecisionResult::Blocked(resp) = &firewall_result {
            return Ok(resp.clone());
        }

        // Transform request: Strip namespace
        let mut transformed_request = request.clone();
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        // Apply edited arguments from firewall edit mode
        if let FirewallDecisionResult::ProceedWithEdits {
            edited_arguments: Some(new_args),
        } = firewall_result
        {
            if let Some(params) = transformed_request.params.as_mut() {
                if let Some(obj) = params.as_object_mut() {
                    obj.insert("arguments".to_string(), new_args);
                }
            }
        }

        tracing::info!(
            "Gateway routing tools/call to server {}: tool={}, request_id={:?}",
            server_id,
            original_name,
            transformed_request.id
        );

        // Route to server
        let result = self
            .server_manager
            .send_request(&server_id, transformed_request)
            .await;

        match &result {
            Ok(response) => {
                tracing::info!(
                    "Gateway received response from server {}: response_id={:?}, has_error={}",
                    server_id,
                    response.id,
                    response.error.is_some()
                );
            }
            Err(e) => {
                tracing::error!(
                    "Gateway failed to get response from server {}: {}",
                    server_id,
                    e
                );
            }
        }

        result
    }

    /// Check access control for an MCP tool call.
    /// Returns a `FirewallDecisionResult` indicating whether to proceed, apply edits, or block.
    ///
    /// `tool_name` is the namespaced name (e.g. `filesystem__write_file`) used for session tracking.
    /// `original_tool_name` is the raw name (e.g. `write_file`) used for permission lookup.
    /// `server_id` is the UUID used for permission lookup and routing.
    async fn check_firewall_mcp_tool(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        original_tool_name: &str,
        server_id: &str,
        request: &JsonRpcRequest,
    ) -> AppResult<FirewallDecisionResult> {
        let session_read = session.read().await;
        let ctx = FirewallCheckContext::McpTool {
            permissions: &session_read.mcp_permissions,
            server_id,
            original_tool_name,
            session_approved: session_read.firewall_session_approvals.contains(tool_name),
            session_denied: session_read.firewall_session_denials.contains(tool_name),
        };
        let result = access_control::check_needs_approval(&ctx);
        let client_id = session_read.client_id.clone();
        drop(session_read);

        self.apply_firewall_result(session, result, &client_id, tool_name, server_id, request)
            .await
    }

    /// Apply a unified FirewallCheckResult, returning a FirewallDecisionResult.
    ///
    /// For Allow/Deny, returns immediately. For Ask, requests user approval via popup.
    #[allow(clippy::too_many_arguments)]
    async fn apply_firewall_result(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        result: FirewallCheckResult,
        client_id: &str,
        tool_name: &str,
        server_or_skill_name: &str,
        request: &JsonRpcRequest,
    ) -> AppResult<FirewallDecisionResult> {
        match result {
            FirewallCheckResult::Allow => Ok(FirewallDecisionResult::Proceed),
            FirewallCheckResult::Deny => {
                tracing::info!(
                    "Firewall denied tool call: client={}, tool={}",
                    client_id,
                    tool_name
                );
                Ok(FirewallDecisionResult::Blocked(JsonRpcResponse::error(
                    request.id.clone().unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32600,
                        format!("Tool call '{}' denied by firewall policy", tool_name),
                        None,
                    ),
                )))
            }
            FirewallCheckResult::Ask => {
                // Get client name for display
                let client_name = {
                    let session_read = session.read().await;
                    let name = session_read.client_name.clone();
                    if name.is_empty() {
                        session_read.client_id.clone()
                    } else {
                        name
                    }
                };

                // Extract arguments preview (truncated) and full arguments
                let full_arguments = request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("arguments"))
                    .cloned();

                let arguments_preview = full_arguments
                    .as_ref()
                    .map(|args| firewall::truncate_arguments_preview(args, 200))
                    .unwrap_or_else(|| "{}".to_string());

                tracing::info!(
                    "Firewall requesting approval: client={}, tool={}, server/skill={}",
                    client_id,
                    tool_name,
                    server_or_skill_name
                );

                // Request approval (blocks until user responds or timeout)
                let response = self
                    .firewall_manager
                    .request_approval(
                        client_id.to_string(),
                        client_name,
                        tool_name.to_string(),
                        server_or_skill_name.to_string(),
                        arguments_preview,
                        None,
                        full_arguments,
                    )
                    .await?;

                let edited_arguments = response.edited_arguments;

                match response.action {
                    FirewallApprovalAction::AllowOnce => {
                        tracing::info!(
                            "Firewall: tool {} allowed once (client={})",
                            tool_name,
                            client_id
                        );
                        if edited_arguments.is_some() {
                            Ok(FirewallDecisionResult::ProceedWithEdits { edited_arguments })
                        } else {
                            Ok(FirewallDecisionResult::Proceed)
                        }
                    }
                    FirewallApprovalAction::AllowSession => {
                        tracing::info!(
                            "Firewall: tool {} allowed for session (client={})",
                            tool_name,
                            client_id
                        );
                        let mut session_write = session.write().await;
                        session_write
                            .firewall_session_approvals
                            .insert(tool_name.to_string());
                        if edited_arguments.is_some() {
                            Ok(FirewallDecisionResult::ProceedWithEdits { edited_arguments })
                        } else {
                            Ok(FirewallDecisionResult::Proceed)
                        }
                    }
                    FirewallApprovalAction::Allow1Minute
                    | FirewallApprovalAction::Allow1Hour
                    | FirewallApprovalAction::AllowPermanent
                    | FirewallApprovalAction::AllowCategories => {
                        tracing::info!(
                            "Firewall: tool {} allowed ({:?}, client={})",
                            tool_name,
                            response.action,
                            client_id
                        );
                        if edited_arguments.is_some() {
                            Ok(FirewallDecisionResult::ProceedWithEdits { edited_arguments })
                        } else {
                            Ok(FirewallDecisionResult::Proceed)
                        }
                    }
                    FirewallApprovalAction::Deny => {
                        tracing::info!(
                            "Firewall: user denied tool {} (client={})",
                            tool_name,
                            client_id
                        );
                        Ok(FirewallDecisionResult::Blocked(JsonRpcResponse::error(
                            request.id.clone().unwrap_or(Value::Null),
                            JsonRpcError::custom(
                                -32600,
                                format!("Tool call '{}' denied by user", tool_name),
                                None,
                            ),
                        )))
                    }
                    FirewallApprovalAction::DenySession => {
                        tracing::info!(
                            "Firewall: tool {} denied for session (client={})",
                            tool_name,
                            client_id
                        );
                        let mut session_write = session.write().await;
                        session_write
                            .firewall_session_denials
                            .insert(tool_name.to_string());
                        Ok(FirewallDecisionResult::Blocked(JsonRpcResponse::error(
                            request.id.clone().unwrap_or(Value::Null),
                            JsonRpcError::custom(
                                -32600,
                                format!("Tool call '{}' denied by user", tool_name),
                                None,
                            ),
                        )))
                    }
                    FirewallApprovalAction::DenyAlways
                    | FirewallApprovalAction::BlockCategories
                    | FirewallApprovalAction::Deny1Hour
                    | FirewallApprovalAction::DisableClient => {
                        tracing::info!(
                            "Firewall: tool {} denied permanently (client={})",
                            tool_name,
                            client_id
                        );
                        Ok(FirewallDecisionResult::Blocked(JsonRpcResponse::error(
                            request.id.clone().unwrap_or(Value::Null),
                            JsonRpcError::custom(
                                -32600,
                                format!("Tool call '{}' denied by user", tool_name),
                                None,
                            ),
                        )))
                    }
                }
            }
        }
    }

    /// Handle search tool call (for deferred loading)
    pub(crate) async fn handle_search_tool(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let mut session_write = session.write().await;

        let deferred = match session_write.deferred_loading.as_mut() {
            Some(d) => d,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Deferred loading not enabled"),
                ));
            }
        };

        // Extract arguments from params
        // MCP tools/call format: params.arguments contains the tool arguments
        let params = match request.params.as_ref() {
            Some(p) => p,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        let arguments = match params.get("arguments") {
            Some(a) => a,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing arguments in params"),
                ));
            }
        };

        let query = match arguments.get("query").and_then(|q| q.as_str()) {
            Some(q) => q,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing query parameter"),
                ));
            }
        };

        let search_type = arguments
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("all");

        let limit = arguments
            .get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(10) as usize;

        let mode = SearchMode::parse_str(
            arguments
                .get("mode")
                .and_then(|m| m.as_str())
                .unwrap_or("regex"),
        );

        // Search based on type
        let mut activated_names = Vec::new();
        let mut all_matches = Vec::new();

        if search_type == "tools" || search_type == "all" {
            let matches = search_tools(query, &deferred.full_catalog, limit, mode);
            for (tool, _score) in &matches {
                deferred.activated_tools.insert(tool.name.clone());
                activated_names.push(tool.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(tool, score)| {
                json!({
                    "type": "tool",
                    "name": tool.name,
                    "relevance": score,
                    "description": tool.description,
                })
            }));
        }

        if search_type == "resources" || search_type == "all" {
            let matches = search_resources(query, &deferred.full_resource_catalog, limit, mode);
            for (resource, _score) in &matches {
                deferred.activated_resources.insert(resource.name.clone());
                activated_names.push(resource.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(resource, score)| {
                json!({
                    "type": "resource",
                    "name": resource.name,
                    "relevance": score,
                    "description": resource.description,
                })
            }));
        }

        if search_type == "prompts" || search_type == "all" {
            let matches = search_prompts(query, &deferred.full_prompt_catalog, limit, mode);
            for (prompt, _score) in &matches {
                deferred.activated_prompts.insert(prompt.name.clone());
                activated_names.push(prompt.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(prompt, score)| {
                json!({
                    "type": "prompt",
                    "name": prompt.name,
                    "relevance": score,
                    "description": prompt.description,
                })
            }));
        }

        drop(session_write);

        // Return search results
        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({
                "activated": activated_names,
                "message": format!("Activated {} items. Use tools/list, resources/list, or prompts/list to see them.", activated_names.len()),
                "matches": all_matches,
            }),
        ))
    }

    /// Handle server_info tool call (for deferred loading).
    ///
    /// Returns the full tool list and instructions for a specific MCP server.
    /// Does NOT activate tools — the LLM still needs `search` to activate them.
    pub(crate) async fn handle_server_info_tool(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        let deferred = match &session_read.deferred_loading {
            Some(d) if d.enabled => d,
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Deferred loading not enabled"),
                ));
            }
        };

        let params = match request.params.as_ref() {
            Some(p) => p,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        let arguments = match params.get("arguments") {
            Some(a) => a,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing arguments in params"),
                ));
            }
        };

        let server_name = match arguments.get("server").and_then(|s| s.as_str()) {
            Some(s) => s,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing server parameter"),
                ));
            }
        };

        // Look up by slugified name
        let slug = super::types::slugify(server_name);
        let tool_list = deferred.server_tool_lists.get(&slug);
        let instructions = deferred.server_instructions.get(&slug);

        if tool_list.is_none() && instructions.is_none() {
            return Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Server '{}' not found. Check the server name in the tool listing.", server_name)
                    }],
                    "isError": true
                }),
            ));
        }

        let mut result_text = String::new();
        if let Some(tools) = tool_list {
            result_text.push_str(&format!("**{}** — {} items\n\n", server_name, tools.len()));
            for name in tools {
                result_text.push_str(&format!("- `{}`\n", name));
            }
        }
        if let Some(inst) = instructions {
            if !result_text.is_empty() {
                result_text.push('\n');
            }
            result_text.push_str(inst);
        }

        drop(session_read);

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({
                "content": [{
                    "type": "text",
                    "text": result_text
                }]
            }),
        ))
    }

    /// Append tools from all registered virtual servers to the tools list.
    fn append_virtual_server_tools(
        &self,
        tools: &mut Vec<serde_json::Value>,
        session: &GatewaySession,
    ) {
        let virtual_servers = self.virtual_servers.read();
        for vs in virtual_servers.iter() {
            if let Some(state) = session.virtual_server_state.get(vs.id()) {
                let virtual_tools = vs.list_tools(state.as_ref());
                for tool in virtual_tools {
                    tools.push(serde_json::to_value(&tool).unwrap_or_default());
                }
            }
        }
    }

    /// Dispatch a tool call to a virtual server with firewall checks.
    async fn dispatch_virtual_tool_call(
        &self,
        vs: Arc<dyn super::virtual_server::VirtualMcpServer>,
        session: Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        use super::virtual_server::*;

        // 1. Permission check (read lock, then drop)
        let (firewall_result, client_id, client_name) = {
            let session_read = session.read().await;
            let state = match session_read.virtual_server_state.get(vs.id()) {
                Some(s) => s,
                None => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32601,
                            format!(
                                "Virtual server '{}' has no session state",
                                vs.display_name()
                            ),
                            None,
                        ),
                    ));
                }
            };
            let approved = session_read.firewall_session_approvals.contains(tool_name);
            let denied = session_read.firewall_session_denials.contains(tool_name);
            let result = vs.check_permissions(state.as_ref(), tool_name, approved, denied);
            (
                result,
                session_read.client_id.clone(),
                session_read.client_name.clone(),
            )
        };

        // 2. Apply firewall
        let decision = match firewall_result {
            VirtualFirewallResult::Standard(check) => {
                self.apply_firewall_result(
                    &session,
                    check,
                    &client_id,
                    tool_name,
                    vs.id(),
                    &request,
                )
                .await?
            }
            VirtualFirewallResult::Handled(d) => d,
        };

        // Handle Blocked / ProceedWithEdits
        let arguments = match decision {
            FirewallDecisionResult::Blocked(resp) => return Ok(resp),
            FirewallDecisionResult::ProceedWithEdits {
                edited_arguments: Some(new_args),
            } => new_args,
            _ => request
                .params
                .as_ref()
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({})),
        };

        // 3. Clone state out of session (can't hold lock across await)
        let state = {
            let session_read = session.read().await;
            match session_read.virtual_server_state.get(vs.id()) {
                Some(s) => s.clone_box(),
                None => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::tool_not_found(tool_name),
                    ));
                }
            }
        };

        // 4. Call handler
        let display_client_name = if client_name.is_empty() {
            client_id.clone()
        } else {
            client_name
        };
        let result = vs
            .handle_tool_call(
                state,
                tool_name,
                arguments,
                &client_id,
                &display_client_name,
            )
            .await;

        // 5. Apply result
        match result {
            VirtualToolCallResult::Success(response) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                response,
            )),
            VirtualToolCallResult::SuccessWithSideEffects {
                response,
                invalidate_cache,
                send_list_changed,
                state_update,
            } => {
                if state_update.is_some() || invalidate_cache {
                    let mut sw = session.write().await;
                    if let Some(updater) = state_update {
                        if let Some(state) = sw.virtual_server_state.get_mut(vs.id()) {
                            updater(state.as_mut());
                        }
                    }
                    if invalidate_cache {
                        sw.invalidate_tools_cache();
                    }
                }
                if send_list_changed {
                    if let Some(broadcast) = &self.notification_broadcast {
                        let notification = JsonRpcNotification {
                            jsonrpc: "2.0".to_string(),
                            method: "notifications/tools/list_changed".to_string(),
                            params: None,
                        };
                        let _ = broadcast.send((vs.id().to_string(), notification));
                    }
                }
                Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    response,
                ))
            }
            VirtualToolCallResult::NotHandled => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::tool_not_found(tool_name),
            )),
            VirtualToolCallResult::ToolError(e) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e)
                    }],
                    "isError": true
                }),
            )),
        }
    }
}
