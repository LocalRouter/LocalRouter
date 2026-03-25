use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTool,
};
use lr_types::{AppError, AppResult};

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

        // Check cache
        if let Some(cached) = &session_read.cached_tools {
            if cached.is_valid() {
                let filtered = apply_catalog_compression_tools(
                    &cached.data,
                    session_read.catalog_compression.as_ref(),
                    session_read.activated_tools(),
                );
                let mut tools: Vec<serde_json::Value> = filtered
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
        let filtered = apply_catalog_compression_tools(
            &tools,
            session_read.catalog_compression.as_ref(),
            session_read.activated_tools(),
        );
        let mut all_tools: Vec<serde_json::Value> = filtered
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

        // Parse tools from results, exhausting pagination for each server
        let mut server_tools: Vec<(String, Vec<McpTool>)> = Vec::new();

        for (server_id, value) in successes {
            let mut all_items: Vec<McpTool> = value
                .get("tools")
                .and_then(|t| serde_json::from_value::<Vec<McpTool>>(t.clone()).ok())
                .unwrap_or_default();

            // Exhaust pagination if the server returned a nextCursor
            let mut next_cursor = value
                .get("nextCursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let mut page = 1u32;
            const MAX_PAGES: u32 = 100;

            while let Some(cursor) = next_cursor.take() {
                if page >= MAX_PAGES {
                    tracing::warn!(
                        "tools/list pagination: hit max page limit ({}) for server {}",
                        MAX_PAGES,
                        server_id
                    );
                    break;
                }

                let page_request = JsonRpcRequest::new(
                    Some(json!(format!("page-{}", page))),
                    "tools/list".to_string(),
                    Some(json!({ "cursor": cursor })),
                );

                match tokio::time::timeout(
                    timeout,
                    self.server_manager.send_request(&server_id, page_request),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        if let Some(result) = &response.result {
                            if let Some(tools) = result.get("tools").and_then(|t| {
                                serde_json::from_value::<Vec<McpTool>>(t.clone()).ok()
                            }) {
                                all_items.extend(tools);
                            }
                            next_cursor = result
                                .get("nextCursor")
                                .and_then(|c| c.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "tools/list pagination error for server {} (page {}): {}",
                            server_id,
                            page,
                            e
                        );
                        break;
                    }
                    Err(_) => {
                        tracing::warn!(
                            "tools/list pagination timeout for server {} (page {})",
                            server_id,
                            page
                        );
                        break;
                    }
                }

                page += 1;
            }

            if !all_items.is_empty() {
                if page > 1 {
                    tracing::info!(
                        "tools/list: fetched {} tools from server {} across {} pages",
                        all_items.len(),
                        server_id,
                        page
                    );
                }
                server_tools.push((server_id, all_items));
            }
        }

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

        // Check virtual servers
        let matching_vs = {
            let virtual_servers = self.virtual_servers.read();
            virtual_servers
                .iter()
                .find(|vs| vs.owns_tool(&tool_name))
                .cloned()
        };
        if let Some(vs) = matching_vs {
            tracing::debug!(
                "Gateway tool call dispatched to virtual server: tool={}, vs={}",
                tool_name,
                vs.display_name()
            );
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

        // Capture firewall action for monitor event before destructuring
        let mon_firewall_action = match &firewall_result {
            FirewallDecisionResult::Proceed => None,
            FirewallDecisionResult::ProceedWithEdits { .. } => {
                Some("proceed_with_edits".to_string())
            }
            FirewallDecisionResult::Blocked(_) => Some("blocked".to_string()),
        };

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

        // Emit pending monitor event for MCP tool call
        let (mon_client_id, mon_client_name, mon_session_id) = {
            let s = session.read().await;
            (
                Some(s.client_id.clone()),
                Some(s.client_name.clone()),
                s.monitor_session_id.clone(),
            )
        };
        let arguments = transformed_request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(Value::Null);
        let mon_event_id = self.emit_monitor_event(
            lr_monitor::MonitorEventType::McpToolCall,
            mon_client_id,
            mon_client_name,
            mon_session_id,
            lr_monitor::MonitorEventData::McpToolCall {
                tool_name: tool_name.clone(),
                server_id: server_id.clone(),
                server_name: None,
                arguments,
                firewall_action: mon_firewall_action,
                latency_ms: None,
                success: None,
                response_preview: None,
                error: None,
            },
            lr_monitor::EventStatus::Pending,
            None,
        );

        let tool_call_start = std::time::Instant::now();

        // Track pending request for cancellation forwarding
        let request_id_str = transformed_request
            .id
            .as_ref()
            .map(|id| {
                id.as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| id.to_string())
            })
            .unwrap_or_default();
        if !request_id_str.is_empty() {
            let mut session_write = session.write().await;
            session_write
                .pending_requests
                .insert(request_id_str.clone(), server_id.clone());
        }

        // Route to server
        let result = self
            .server_manager
            .send_request(&server_id, transformed_request)
            .await;

        // Remove from pending requests
        if !request_id_str.is_empty() {
            let mut session_write = session.write().await;
            session_write.pending_requests.remove(&request_id_str);
        }

        let tool_call_latency = tool_call_start.elapsed().as_millis() as u64;

        // Update the monitor event with response data
        match &result {
            Ok(response) => {
                tracing::info!(
                    "Gateway received response from server {}: response_id={:?}, has_error={}",
                    server_id,
                    response.id,
                    response.error.is_some()
                );

                let response_preview = response
                    .result
                    .as_ref()
                    .map(|r| serde_json::to_string_pretty(r).unwrap_or_default())
                    .unwrap_or_default();
                let error_msg = response.error.as_ref().map(|e| e.message.clone());
                let success = response.error.is_none();
                let status = if success {
                    lr_monitor::EventStatus::Complete
                } else {
                    lr_monitor::EventStatus::Error
                };
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = status;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms: ref mut lm,
                            success: ref mut s,
                            response_preview: ref mut rp,
                            error: ref mut e,
                            ..
                        } = &mut event.data
                        {
                            *lm = Some(tool_call_latency);
                            *s = Some(success);
                            *rp = Some(response_preview);
                            *e = error_msg;
                        }
                    }),
                );
            }
            Err(e) => {
                tracing::error!(
                    "Gateway failed to get response from server {}: {}",
                    server_id,
                    e
                );

                let err_msg = e.to_string();
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = lr_monitor::EventStatus::Error;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms: ref mut lm,
                            success: ref mut s,
                            response_preview: ref mut rp,
                            error: ref mut er,
                            ..
                        } = &mut event.data
                        {
                            *lm = Some(tool_call_latency);
                            *s = Some(false);
                            *rp = Some(String::new());
                            *er = Some(err_msg);
                        }
                    }),
                );
            }
        }

        // Context management: compress large responses
        if let Ok(response) = result {
            let compressed = self
                .maybe_compress_response(&session, &tool_name, response)
                .await;
            Ok(compressed)
        } else {
            result
        }
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

        // Bypass firewall for internal test client (Try It Out UI).
        // The test client is for testing tool behavior, not permission enforcement.
        if session_read.client_id == "internal-test" {
            drop(session_read);
            return Ok(FirewallDecisionResult::Proceed);
        }

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

        self.apply_firewall_result(
            session,
            result,
            &client_id,
            tool_name,
            server_id,
            request,
            firewall::InterceptCategory::Mcp,
        )
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
        intercept_category: firewall::InterceptCategory,
    ) -> AppResult<FirewallDecisionResult> {
        // Monitor intercept: override Allow → Ask if intercept rule matches
        let result = if result == FirewallCheckResult::Allow
            && self
                .firewall_manager
                .should_intercept(client_id, intercept_category)
        {
            tracing::info!(
                "Monitor intercept: overriding Allow → Ask for tool {} (client={})",
                tool_name,
                client_id
            );
            FirewallCheckResult::Ask
        } else {
            result
        };

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

    /// Append tools from all registered virtual servers to the tools list.
    fn append_virtual_server_tools(
        &self,
        tools: &mut Vec<serde_json::Value>,
        session: &GatewaySession,
    ) {
        let virtual_servers = self.virtual_servers.read();

        // Collect deferred virtual-server slugs from the compression plan
        let deferred_virtual: std::collections::HashSet<&str> = session
            .catalog_compression
            .as_ref()
            .map(|plan| {
                plan.deferred_servers
                    .iter()
                    .map(|d| d.server_slug.as_str())
                    .collect()
            })
            .unwrap_or_default();

        for vs in virtual_servers.iter() {
            if let Some(state) = session.virtual_server_state.get(vs.id()) {
                let virtual_tools = vs.list_tools(state.as_ref());
                let server_deferred = deferred_virtual.contains(vs.id());

                if server_deferred {
                    // Only defer tools that the virtual server explicitly allows
                    let deferrable: std::collections::HashSet<String> =
                        vs.deferrable_tools(state.as_ref()).into_iter().collect();
                    let activated = session.activated_tools();

                    for tool in virtual_tools {
                        let is_deferrable = deferrable.contains(&tool.name);
                        let is_activated =
                            activated.map(|a| a.contains(&tool.name)).unwrap_or(false);

                        // Include if: not deferrable, or activated via ctx_search
                        if !is_deferrable || is_activated {
                            tools.push(serde_json::to_value(&tool).unwrap_or_default());
                        }
                    }
                } else {
                    for tool in virtual_tools {
                        tools.push(serde_json::to_value(&tool).unwrap_or_default());
                    }
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

            // Bypass firewall for internal test client (Try It Out UI).
            // The test client is for testing tool behavior, not permission enforcement.
            let is_internal_test = session_read.client_id == "internal-test";

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

            let result = if is_internal_test {
                VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
            } else {
                let approved = session_read.firewall_session_approvals.contains(tool_name);
                let denied = session_read.firewall_session_denials.contains(tool_name);
                let arguments = request.params.as_ref().and_then(|p| p.get("arguments"));
                vs.check_permissions(state.as_ref(), tool_name, arguments, approved, denied)
            };

            (
                result,
                session_read.client_id.clone(),
                session_read.client_name.clone(),
            )
        };

        // 2. Apply firewall
        let vs_intercept_category = match vs.id() {
            "_skills" => firewall::InterceptCategory::Skill,
            "_marketplace" => firewall::InterceptCategory::Marketplace,
            "_coding_agents" => firewall::InterceptCategory::CodingAgent,
            _ => firewall::InterceptCategory::Mcp,
        };
        let decision = match firewall_result {
            VirtualFirewallResult::Standard(check) => {
                self.apply_firewall_result(
                    &session,
                    check,
                    &client_id,
                    tool_name,
                    vs.id(),
                    &request,
                    vs_intercept_category,
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

        // 4. Prepare client name and emit pending monitor event
        let display_client_name = if client_name.is_empty() {
            client_id.clone()
        } else {
            client_name
        };
        let mon_session_id = {
            let s = session.read().await;
            s.monitor_session_id.clone()
        };
        let mon_arguments = request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(Value::Null);
        let mon_event_id = self.emit_monitor_event(
            lr_monitor::MonitorEventType::McpToolCall,
            Some(client_id.clone()),
            Some(display_client_name.clone()),
            mon_session_id,
            lr_monitor::MonitorEventData::McpToolCall {
                tool_name: tool_name.to_string(),
                server_id: vs.id().to_string(),
                server_name: Some(vs.display_name().to_string()),
                arguments: mon_arguments,
                firewall_action: None,
                latency_ms: None,
                success: None,
                response_preview: None,
                error: None,
            },
            lr_monitor::EventStatus::Pending,
            None,
        );

        let tool_call_start = std::time::Instant::now();

        // 5. Call handler
        let result = vs
            .handle_tool_call(
                state,
                tool_name,
                arguments,
                &client_id,
                &display_client_name,
            )
            .await;

        let tool_call_latency = tool_call_start.elapsed().as_millis() as u64;

        // 6. Apply result and update monitor event
        match result {
            VirtualToolCallResult::Success(response) => {
                let preview = serde_json::to_string_pretty(&response).unwrap_or_default();
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = lr_monitor::EventStatus::Complete;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms,
                            success,
                            response_preview,
                            ..
                        } = &mut event.data
                        {
                            *latency_ms = Some(tool_call_latency);
                            *success = Some(true);
                            *response_preview = Some(preview);
                        }
                    }),
                );
                Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    response,
                ))
            }
            VirtualToolCallResult::SuccessWithSideEffects {
                response,
                invalidate_cache,
                send_list_changed,
                state_update,
                add_allowed_servers,
            } => {
                let preview = serde_json::to_string_pretty(&response).unwrap_or_default();
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = lr_monitor::EventStatus::Complete;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms,
                            success,
                            response_preview,
                            ..
                        } = &mut event.data
                        {
                            *latency_ms = Some(tool_call_latency);
                            *success = Some(true);
                            *response_preview = Some(preview);
                        }
                    }),
                );
                if state_update.is_some() || invalidate_cache || add_allowed_servers.is_some() {
                    let mut sw = session.write().await;
                    if let Some(updater) = state_update {
                        if let Some(state) = sw.virtual_server_state.get_mut(vs.id()) {
                            updater(state.as_mut());
                        }
                    }
                    if invalidate_cache {
                        sw.invalidate_tools_cache();
                    }
                    if let Some(new_servers) = add_allowed_servers {
                        for server_id in new_servers {
                            if !sw.allowed_servers.contains(&server_id) {
                                sw.allowed_servers.push(server_id);
                            }
                        }
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
            VirtualToolCallResult::NotHandled => {
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = lr_monitor::EventStatus::Error;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms,
                            success,
                            error,
                            ..
                        } = &mut event.data
                        {
                            *latency_ms = Some(tool_call_latency);
                            *success = Some(false);
                            *error = Some("Tool not handled".to_string());
                        }
                    }),
                );
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::tool_not_found(tool_name),
                ))
            }
            VirtualToolCallResult::ToolError(e) => {
                let err_msg = e.to_string();
                self.update_monitor_event(
                    &mon_event_id,
                    Box::new(move |event| {
                        event.status = lr_monitor::EventStatus::Error;
                        event.duration_ms = Some(tool_call_latency);
                        if let lr_monitor::MonitorEventData::McpToolCall {
                            latency_ms,
                            success,
                            error,
                            ..
                        } = &mut event.data
                        {
                            *latency_ms = Some(tool_call_latency);
                            *success = Some(false);
                            *error = Some(err_msg);
                        }
                    }),
                );
                Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {}", e)
                        }],
                        "isError": true
                    }),
                ))
            }
        }
    }

    /// Compress a tool call response if context management is enabled and it exceeds the threshold.
    ///
    /// Indexes the full response into context-mode FTS5 and replaces it with a truncated
    /// preview + search hint. Responses from ctx_* tools are never compressed.
    async fn maybe_compress_response(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        response: JsonRpcResponse,
    ) -> JsonRpcResponse {
        // Never compress error responses
        if response.error.is_some() {
            return response;
        }

        let result = match &response.result {
            Some(r) => r,
            None => return response,
        };

        // Extract text content and measure size
        let full_text = extract_text_from_result(result);
        if full_text.is_empty() {
            return response;
        }

        // Check context management state and extract store in a single lock acquisition
        let (threshold, run_id, store, search_tool_name) = {
            let mut session_write = session.write().await;
            if let Some(state) = session_write.virtual_server_state.get_mut("_context_mode") {
                if let Some(cm_state) = state
                    .as_any_mut()
                    .downcast_mut::<super::context_mode::ContextModeSessionState>(
                ) {
                    if !cm_state.enabled || full_text.len() <= cm_state.response_threshold_bytes {
                        return response;
                    }
                    // Skip compression for our own search/read tools
                    if tool_name == cm_state.search_tool_name
                        || tool_name == cm_state.read_tool_name
                    {
                        return response;
                    }
                    // Check gateway indexing eligibility
                    let (server_slug, original_name) = match tool_name.split_once("__") {
                        Some((s, t)) => (s, t),
                        None => (tool_name, tool_name),
                    };
                    if !cm_state
                        .gateway_indexing
                        .is_tool_eligible(server_slug, original_name)
                    {
                        return response;
                    }
                    let run_id = cm_state.next_run_id(tool_name);
                    (
                        cm_state.response_threshold_bytes,
                        run_id,
                        cm_state.store.clone(),
                        cm_state.search_tool_name.clone(),
                    )
                } else {
                    return response;
                }
            } else {
                return response;
            }
        };

        let source = format!("{}:{}", tool_name, run_id);
        let byte_size = full_text.len();

        // Index full content into native ContentStore.
        // Use spawn_blocking since ContentStore uses parking_lot::Mutex.
        let source_idx = source.clone();
        let full_text_for_index = full_text.clone();
        let index_result = tokio::task::spawn_blocking(move || {
            store
                .index(&source_idx, &full_text_for_index)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|e| Err(format!("Index task panicked: {}", e)));

        if let Err(e) = index_result {
            tracing::warn!(
                "Failed to index response for {} ({}): {}",
                tool_name,
                source,
                e
            );
            return response;
        }

        // Build compressed response with preview
        let preview_bytes = (threshold / 8).clamp(200, 500);
        let preview = truncate_to_char_boundary(&full_text, preview_bytes);
        let compressed_text = format!(
            "[Response compressed — {} bytes indexed as {}]\n\n{}\n\nFull output indexed. \
             Use {}(queries=[\"your search terms\"], source=\"{}\") to retrieve specific sections.",
            byte_size, source, preview, search_tool_name, source
        );

        // Build new response with compressed content
        let mut new_result = result.clone();
        if let Some(content) = new_result.get_mut("content").and_then(|c| c.as_array_mut()) {
            content.clear();
            content.push(json!({
                "type": "text",
                "text": compressed_text,
            }));
        }

        let saved = byte_size.saturating_sub(compressed_text.len()) as u64;
        if let Some(cb) = self.on_context_saved.read().as_ref() {
            cb(saved);
        }

        tracing::info!(
            "Compressed response for {} ({} bytes → {} bytes, source={})",
            tool_name,
            byte_size,
            compressed_text.len(),
            source
        );

        JsonRpcResponse {
            result: Some(new_result),
            ..response
        }
    }
}

/// Extract all text content from an MCP result value.
fn extract_text_from_result(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Truncate a string to at most `max_bytes` at a char boundary.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Extract the server slug (namespace prefix) from a namespaced name.
/// e.g., "everything-mcp-server__echo" -> "everything-mcp-server"
fn extract_server_slug(namespaced_name: &str) -> &str {
    namespaced_name
        .find(NAMESPACE_SEPARATOR)
        .map(|idx| &namespaced_name[..idx])
        .unwrap_or(namespaced_name)
}

/// Apply catalog compression plan to a tools list:
/// 1. Filter out tools from servers whose tools are deferred
/// 2. Re-include tools that were individually activated via ctx_search
fn apply_catalog_compression_tools(
    tools: &[NamespacedTool],
    plan: Option<&CatalogCompressionPlan>,
    activated: Option<&std::collections::HashSet<String>>,
) -> Vec<NamespacedTool> {
    let plan = match plan {
        Some(p) => p,
        None => return tools.to_vec(),
    };

    // Collect server slugs whose tools are fully deferred
    let deferred_servers: std::collections::HashSet<&str> = plan
        .deferred_servers
        .iter()
        .map(|d| d.server_slug.as_str())
        .collect();

    if deferred_servers.is_empty() {
        return tools.to_vec();
    }

    let original_count = tools.len();

    let result: Vec<NamespacedTool> = tools
        .iter()
        .filter(|tool| {
            let server_slug = extract_server_slug(&tool.name);
            // Include if not from a deferred server, or individually activated via ctx_search
            !deferred_servers.contains(server_slug)
                || activated.map(|a| a.contains(&tool.name)).unwrap_or(false)
        })
        .cloned()
        .collect();

    if result.len() < original_count {
        tracing::info!(
            "Catalog compression: deferred {}/{} tools (servers: {:?})",
            original_count - result.len(),
            original_count,
            deferred_servers,
        );
    }

    result
}

/// Apply catalog compression plan to a resources list:
/// Filter out resources from servers whose resources are deferred,
/// re-including individually activated resources.
pub(crate) fn apply_catalog_compression_resources(
    resources: &[NamespacedResource],
    plan: Option<&CatalogCompressionPlan>,
    activated: Option<&std::collections::HashSet<String>>,
) -> Vec<NamespacedResource> {
    let plan = match plan {
        Some(p) => p,
        None => return resources.to_vec(),
    };

    let deferred_servers: std::collections::HashSet<&str> = plan
        .deferred_servers
        .iter()
        .map(|d| d.server_slug.as_str())
        .collect();

    if deferred_servers.is_empty() {
        return resources.to_vec();
    }

    let original_count = resources.len();
    let result: Vec<NamespacedResource> = resources
        .iter()
        .filter(|r| {
            !deferred_servers.contains(extract_server_slug(&r.name))
                || activated.map(|a| a.contains(&r.name)).unwrap_or(false)
        })
        .cloned()
        .collect();

    if result.len() < original_count {
        tracing::info!(
            "Catalog compression: deferred {}/{} resources (servers: {:?})",
            original_count - result.len(),
            original_count,
            deferred_servers,
        );
    }

    result
}

/// Apply catalog compression plan to a prompts list:
/// Filter out prompts from servers whose prompts are deferred,
/// re-including individually activated prompts.
pub(crate) fn apply_catalog_compression_prompts(
    prompts: &[NamespacedPrompt],
    plan: Option<&CatalogCompressionPlan>,
    activated: Option<&std::collections::HashSet<String>>,
) -> Vec<NamespacedPrompt> {
    let plan = match plan {
        Some(p) => p,
        None => return prompts.to_vec(),
    };

    let deferred_servers: std::collections::HashSet<&str> = plan
        .deferred_servers
        .iter()
        .map(|d| d.server_slug.as_str())
        .collect();

    if deferred_servers.is_empty() {
        return prompts.to_vec();
    }

    let original_count = prompts.len();
    let result: Vec<NamespacedPrompt> = prompts
        .iter()
        .filter(|p| {
            !deferred_servers.contains(extract_server_slug(&p.name))
                || activated.map(|a| a.contains(&p.name)).unwrap_or(false)
        })
        .cloned()
        .collect();

    if result.len() < original_count {
        tracing::info!(
            "Catalog compression: deferred {}/{} prompts (servers: {:?})",
            original_count - result.len(),
            original_count,
            deferred_servers,
        );
    }

    result
}
