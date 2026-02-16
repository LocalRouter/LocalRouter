use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTool,
};
use lr_types::{AppError, AppResult};

use super::deferred::create_search_tool;
use super::deferred::{search_prompts, search_resources, search_tools, SearchMode};
use super::merger::merge_tools;
use super::router::{broadcast_request, separate_results};
use super::session::GatewaySession;
use super::types::*;

use super::access_control::{self, AccessDecision};
use super::firewall::{self, FirewallApprovalAction};
use super::gateway::McpGateway;

/// Result of a firewall access decision check
pub(crate) enum FirewallDecisionResult {
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
            session_read.cached_tools.as_ref().map_or(false, |c| c.is_valid()),
        );

        if let Some(deferred) = &session_read.deferred_loading {
            if deferred.enabled {
                // Return only search tool + activated tools + skill tools
                let mut tools: Vec<serde_json::Value> = vec![serde_json::to_value(
                    create_search_tool(deferred.resources_deferred, deferred.prompts_deferred),
                )
                .unwrap_or_default()];

                for tool_name in &deferred.activated_tools {
                    if let Some(tool) = deferred.full_catalog.iter().find(|t| t.name == *tool_name)
                    {
                        tools.push(serde_json::to_value(tool).unwrap_or_default());
                    }
                }

                let skills_permissions = session_read.skills_permissions.clone();
                let info_loaded = session_read.skills_info_loaded.clone();
                let async_enabled = session_read.skills_async_enabled;
                let marketplace_permission = session_read.marketplace_permission.clone();
                drop(session_read);

                self.append_skill_tools(
                    &mut tools,
                    &skills_permissions,
                    &info_loaded,
                    async_enabled,
                    true,
                );
                self.append_marketplace_tools(&mut tools, &marketplace_permission);

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

                let skills_permissions = session_read.skills_permissions.clone();
                let info_loaded = session_read.skills_info_loaded.clone();
                let async_enabled = session_read.skills_async_enabled;
                let marketplace_permission = session_read.marketplace_permission.clone();
                drop(session_read);

                // Skills always use their own deferred loading (get_info unlocks run/read)
                self.append_skill_tools(
                    &mut tools,
                    &skills_permissions,
                    &info_loaded,
                    async_enabled,
                    true,
                );
                self.append_marketplace_tools(&mut tools, &marketplace_permission);

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
        let skills_permissions = session_read.skills_permissions.clone();
        let info_loaded = session_read.skills_info_loaded.clone();
        let async_enabled = session_read.skills_async_enabled;
        let marketplace_permission = session_read.marketplace_permission.clone();
        drop(session_read);

        let mut all_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or_default())
            .collect();

        // Skills always use their own deferred loading (get_info unlocks run/read)
        self.append_skill_tools(
            &mut all_tools,
            &skills_permissions,
            &info_loaded,
            async_enabled,
            true,
        );
        self.append_marketplace_tools(&mut all_tools, &marketplace_permission);

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

        // Check if it's a marketplace tool
        if lr_marketplace::is_marketplace_tool(&tool_name) {
            return self
                .handle_marketplace_tool_call(session, &tool_name, request)
                .await;
        }

        // Check if it's a skill tool
        if self.is_skill_tool(&tool_name) {
            // Firewall check for skill tools
            let skill_firewall_result = self
                .check_firewall_skill_tool(&session, &tool_name, &request)
                .await?;

            match skill_firewall_result {
                FirewallDecisionResult::Blocked(resp) => return Ok(resp),
                FirewallDecisionResult::ProceedWithEdits {
                    edited_arguments: Some(new_args),
                } => {
                    // Apply edited arguments to the request before handling
                    let mut edited_request = request.clone();
                    if let Some(params) = edited_request.params.as_mut() {
                        if let Some(obj) = params.as_object_mut() {
                            obj.insert("arguments".to_string(), new_args);
                        }
                    }
                    return self
                        .handle_skill_tool_call(session, &tool_name, edited_request)
                        .await;
                }
                _ => {}
            }

            return self
                .handle_skill_tool_call(session, &tool_name, request)
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

        match &firewall_result {
            FirewallDecisionResult::Blocked(resp) => return Ok(resp.clone()),
            _ => {}
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
        // Use server UUID + original tool name for permission resolution
        // (key format: "UUID__original_name", matching what the UI stores)
        let decision = access_control::check_mcp_tool_access(
            &session_read.mcp_permissions,
            server_id,
            original_tool_name,
        );

        // Session tracking uses the namespaced name (what the client sends in requests)
        let already_approved = session_read.firewall_session_approvals.contains(tool_name);
        let already_denied = session_read.firewall_session_denials.contains(tool_name);

        let client_id = session_read.client_id.clone();
        drop(session_read);

        self.apply_access_decision(
            session,
            &decision,
            already_approved,
            already_denied,
            &client_id,
            tool_name,
            server_id,
            request,
        )
        .await
    }

    /// Check access control for a skill tool call.
    /// Returns a `FirewallDecisionResult` indicating whether to proceed, apply edits, or block.
    async fn check_firewall_skill_tool(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        request: &JsonRpcRequest,
    ) -> AppResult<FirewallDecisionResult> {
        // Extract skill name from tool name using simple heuristic:
        // skill tools follow pattern `skill_{name}_{action}` where action is
        // get_info, run_{file}, run_async_{file}, read_{file}
        let skill_name = extract_skill_name_from_tool(tool_name);

        // Global utility tools (e.g. skill_get_async_status) have no skill name.
        // These don't execute skill code, so skip permission checks.
        if skill_name.is_empty() {
            return Ok(FirewallDecisionResult::Proceed);
        }

        let session_read = session.read().await;
        let decision = access_control::check_skill_tool_access(
            &session_read.skills_permissions,
            &skill_name,
            tool_name,
        );

        let already_approved = session_read.firewall_session_approvals.contains(tool_name);
        let already_denied = session_read.firewall_session_denials.contains(tool_name);

        let client_id = session_read.client_id.clone();
        drop(session_read);

        self.apply_access_decision(
            session,
            &decision,
            already_approved,
            already_denied,
            &client_id,
            tool_name,
            &skill_name,
            request,
        )
        .await
    }

    /// Apply access decision, returning a FirewallDecisionResult.
    #[allow(clippy::too_many_arguments)]
    async fn apply_access_decision(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        decision: &AccessDecision,
        already_approved: bool,
        already_denied: bool,
        client_id: &str,
        tool_name: &str,
        server_or_skill_name: &str,
        request: &JsonRpcRequest,
    ) -> AppResult<FirewallDecisionResult> {
        match decision {
            AccessDecision::Allow => Ok(FirewallDecisionResult::Proceed),
            AccessDecision::Deny => {
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
            AccessDecision::Ask => {
                // Check if already denied for this session
                if already_denied {
                    tracing::debug!(
                        "Firewall: tool {} already denied for session (client={})",
                        tool_name,
                        client_id
                    );
                    return Ok(FirewallDecisionResult::Blocked(JsonRpcResponse::error(
                        request.id.clone().unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32600,
                            format!("Tool call '{}' denied by user", tool_name),
                            None,
                        ),
                    )));
                }

                // Check if already approved for this session
                if already_approved {
                    tracing::debug!(
                        "Firewall: tool {} already approved for session (client={})",
                        tool_name,
                        client_id
                    );
                    return Ok(FirewallDecisionResult::Proceed);
                }

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
                        // Add to session approvals
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
                    FirewallApprovalAction::Allow1Hour => {
                        tracing::info!(
                            "Firewall: tool {} allowed for 1 hour (client={})",
                            tool_name,
                            client_id
                        );
                        if edited_arguments.is_some() {
                            Ok(FirewallDecisionResult::ProceedWithEdits { edited_arguments })
                        } else {
                            Ok(FirewallDecisionResult::Proceed)
                        }
                    }
                    FirewallApprovalAction::AllowPermanent => {
                        tracing::info!(
                            "Firewall: tool {} allowed permanently (client={})",
                            tool_name,
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
                    | FirewallApprovalAction::BlockCategories => {
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

        let mode = SearchMode::from_str(
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

    /// Append skill tools to a tools list if the client has skills access
    pub(crate) fn append_skill_tools(
        &self,
        tools: &mut Vec<serde_json::Value>,
        permissions: &lr_config::SkillsPermissions,
        info_loaded: &std::collections::HashSet<String>,
        async_enabled: bool,
        deferred_loading: bool,
    ) {
        let has_any_access = permissions.global.is_enabled() || !permissions.skills.is_empty();
        if has_any_access {
            if let Some(sm) = self.skill_manager.get() {
                let skill_tools = lr_skills::mcp_tools::build_skill_tools(
                    sm,
                    permissions,
                    info_loaded,
                    async_enabled,
                    deferred_loading,
                );
                for st in skill_tools {
                    tools.push(serde_json::to_value(&st).unwrap_or_default());
                }
            }
        }
    }

    /// Check if a tool name matches a skill tool pattern
    pub(crate) fn is_skill_tool(&self, tool_name: &str) -> bool {
        lr_skills::mcp_tools::is_skill_tool(tool_name)
    }

    /// Handle a skill tool call
    pub(crate) async fn handle_skill_tool_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let (skill_manager, script_executor) =
            match (self.skill_manager.get(), self.script_executor.get()) {
                (Some(sm), Some(se)) => (sm, se),
                _ => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32601,
                            "Skills support is not configured".to_string(),
                            None,
                        ),
                    ));
                }
            };

        // Get skills permissions and info_loaded from session
        let session_read = session.read().await;
        let skills_permissions = session_read.skills_permissions.clone();
        let info_loaded = session_read.skills_info_loaded.clone();
        let async_enabled = session_read.skills_async_enabled;
        drop(session_read);

        // Extract arguments from params
        let arguments = request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(json!({}));

        match lr_skills::mcp_tools::handle_skill_tool_call(
            tool_name,
            &arguments,
            skill_manager,
            script_executor,
            &skills_permissions,
            &info_loaded,
            async_enabled,
        )
        .await
        {
            Ok(Some(result)) => {
                use lr_skills::mcp_tools::SkillToolResult;
                match result {
                    SkillToolResult::Response(response) => Ok(JsonRpcResponse::success(
                        request.id.unwrap_or(Value::Null),
                        response,
                    )),
                    SkillToolResult::InfoLoaded {
                        skill_name,
                        response,
                    } => {
                        // Mark skill as info-loaded and invalidate tools cache
                        {
                            let mut session_write = session.write().await;
                            session_write.mark_skill_info_loaded(&skill_name);
                            session_write.invalidate_tools_cache();
                        }

                        // Send tools/list_changed notification if broadcast channel exists
                        if let Some(broadcast) = &self.notification_broadcast {
                            let notification = JsonRpcNotification {
                                jsonrpc: "2.0".to_string(),
                                method: "notifications/tools/list_changed".to_string(),
                                params: None,
                            };
                            let _ = broadcast.send(("_skills".to_string(), notification));
                        }

                        Ok(JsonRpcResponse::success(
                            request.id.unwrap_or(Value::Null),
                            response,
                        ))
                    }
                }
            }
            Ok(None) => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::tool_not_found(tool_name),
            )),
            Err(e) => Ok(JsonRpcResponse::success(
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

    /// Append marketplace tools to a tools list if the client has marketplace access
    pub(crate) fn append_marketplace_tools(
        &self,
        tools: &mut Vec<serde_json::Value>,
        marketplace_permission: &lr_config::PermissionState,
    ) {
        if !marketplace_permission.is_enabled() {
            return;
        }
        if let Some(service) = self.marketplace_service.get() {
            if service.is_enabled() {
                let marketplace_tools = service.list_tools();
                tools.extend(marketplace_tools);
            }
        }
    }

    /// Handle a marketplace tool call
    pub(crate) async fn handle_marketplace_tool_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let marketplace_service = match self.marketplace_service.get() {
            Some(service) => service,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(-32601, "Marketplace is not configured".to_string(), None),
                ));
            }
        };

        // Check marketplace access control
        let session_read = session.read().await;
        let decision =
            access_control::check_marketplace_access(&session_read.marketplace_permission);
        let already_approved = session_read.firewall_session_approvals.contains(tool_name);
        let already_denied = session_read.firewall_session_denials.contains(tool_name);
        let client_id = session_read.client_id.clone();
        let client_name = session_read.client_name.clone();
        drop(session_read);

        let marketplace_firewall = self
            .apply_access_decision(
                &session,
                &decision,
                already_approved,
                already_denied,
                &client_id,
                tool_name,
                "marketplace",
                &request,
            )
            .await?;

        if let FirewallDecisionResult::Blocked(resp) = marketplace_firewall {
            return Ok(resp);
        }

        // Extract arguments from params
        let arguments = request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(json!({}));

        match marketplace_service
            .handle_tool_call(tool_name, arguments, &client_id, &client_name)
            .await
        {
            Ok(result) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                    }]
                }),
            )),
            Err(e) => Ok(JsonRpcResponse::success(
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

/// Extract skill name from a skill tool name.
///
/// Skill tools follow the pattern `skill_{sanitized_name}_{action}` where action is
/// one of: `get_info`, `run_{file}`, `run_async_{file}`, `read_{file}`, `get_async_status`.
///
/// This is a best-effort extraction for firewall rule matching. It strips the `skill_` prefix
/// and tries to identify the skill name portion before the action suffix.
fn extract_skill_name_from_tool(tool_name: &str) -> String {
    let rest = tool_name.strip_prefix("skill_").unwrap_or(tool_name);

    // Try to find known action suffixes and extract the name before them
    // Order matters: check longer patterns first
    for suffix in &[
        "_get_async_status",
        "_get_info",
        "_run_async_",
        "_run_",
        "_read_",
    ] {
        if let Some(pos) = rest.find(suffix) {
            if pos > 0 {
                return rest[..pos].to_string();
            }
        }
    }

    // If the tool name is exactly `skill_get_async_status` (global), return empty
    if rest == "get_async_status" {
        return String::new();
    }

    // Fallback: return the rest as the skill name
    rest.to_string()
}
