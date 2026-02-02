use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpResource};
use lr_types::{AppError, AppResult};

use super::merger::merge_resources;
use super::router::{broadcast_request, separate_results};
use super::session::GatewaySession;
use super::types::*;

use super::gateway::McpGateway;

impl McpGateway {
    /// Handle resources/list request
    pub(crate) async fn handle_resources_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let request_id = request.id.clone();
        let session_read = session.read().await;

        // Check for deferred loading (only if client supports resources.listChanged)
        if let Some(deferred) = &session_read.deferred_loading {
            if deferred.enabled && deferred.resources_deferred {
                // Return only activated resources
                let resources: Vec<serde_json::Value> = deferred
                    .full_resource_catalog
                    .iter()
                    .filter(|r| deferred.activated_resources.contains(&r.name))
                    .map(|r| serde_json::to_value(r).unwrap_or_default())
                    .collect();

                drop(session_read);

                tracing::debug!(
                    "resources/list returning {} deferred resources (request_id={:?})",
                    resources.len(),
                    request_id
                );

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"resources": resources}),
                ));
            }
        }

        // Check cache
        if let Some(cached) = &session_read.cached_resources {
            if cached.is_valid() {
                let resources = cached.data.clone();
                drop(session_read);

                tracing::debug!(
                    "resources/list returning {} cached resources (request_id={:?})",
                    resources.len(),
                    request_id
                );

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"resources": resources}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        tracing::info!(
            "resources/list fetching from {} servers (request_id={:?})",
            allowed_servers.len(),
            request_id
        );

        // Fetch from servers
        let (resources, failures) = self
            .fetch_and_merge_resources(&allowed_servers, request.clone())
            .await?;

        tracing::info!(
            "resources/list fetched {} resources with {} failures (request_id={:?})",
            resources.len(),
            failures.len(),
            request_id
        );

        // Update session mappings, cache, failures, and mark as fetched
        {
            let mut session_write = session.write().await;
            session_write.update_resource_mappings(&resources);
            session_write.last_broadcast_failures = failures.clone();
            session_write.resources_list_fetched = true;

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_resources = Some(CachedList::new(resources.clone(), cache_ttl));
        }

        let mut result = json!({"resources": resources});
        if !failures.is_empty() {
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

    /// Fetch and merge resources from servers
    pub(crate) async fn fetch_and_merge_resources(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedResource>, Vec<ServerFailure>)> {
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

        // Parse resources from results
        let server_resources: Vec<(String, Vec<McpResource>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("resources")
                    .and_then(|r| serde_json::from_value::<Vec<McpResource>>(r.clone()).ok())
                    .map(|resources| (server_id, resources))
            })
            .collect();

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_resources(server_resources, &failures, Some(&name_mapping)),
            failures,
        ))
    }

    /// Handle resources/read request
    pub(crate) async fn handle_resources_read(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract resource URI or name from params
        let params = match request.params.as_ref() {
            Some(p) => p,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        // Try to get resource name first (preferred for namespaced routing)
        let resource_name = params.get("name").and_then(|n| n.as_str());

        let (server_id, original_name) = if let Some(name) = resource_name {
            // Look up resource in session mapping to get server_id (UUID) and original_name
            let session_read = session.read().await;
            match session_read.resource_mapping.get(name) {
                Some((id, orig)) => {
                    let result = (id.clone(), orig.clone());
                    drop(session_read);
                    result
                }
                None => {
                    drop(session_read);
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::resource_not_found(format!("Resource not found: {}", name)),
                    ));
                }
            }
        } else {
            // Fallback: route by URI
            let uri = match params.get("uri").and_then(|u| u.as_str()) {
                Some(u) => u,
                None => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::invalid_params("Missing resource name or URI"),
                    ));
                }
            };

            // Look up URI in session mapping
            let session_read = session.read().await;
            let mapping = session_read.resource_uri_mapping.get(uri).cloned();
            let resources_list_fetched = session_read.resources_list_fetched;
            let allowed_servers = session_read.allowed_servers.clone();
            drop(session_read);

            // If URI not found and we haven't tried fetching resources/list yet, do so
            if mapping.is_none() && !resources_list_fetched {
                tracing::info!(
                    "Resource URI not in mapping and resources/list not yet fetched, fetching now"
                );

                // Fetch resources/list to populate the URI mapping (only once per session)
                let (resources, _failures) = self
                    .fetch_and_merge_resources(
                        &allowed_servers,
                        JsonRpcRequest::new(
                            Some(serde_json::json!("auto")),
                            "resources/list".to_string(),
                            None,
                        ),
                    )
                    .await?;

                // Update session mappings and mark as fetched
                let mut session_write = session.write().await;
                session_write.update_resource_mappings(&resources);
                session_write.resources_list_fetched = true;
                let new_mapping = session_write.resource_uri_mapping.get(uri).cloned();
                drop(session_write);

                // Try again with populated mapping
                match new_mapping {
                    Some(m) => m,
                    None => {
                        return Ok(JsonRpcResponse::error(
                            request.id.unwrap_or(Value::Null),
                            JsonRpcError::resource_not_found(format!(
                                "Resource URI not found after fetching resources/list: {}",
                                uri
                            )),
                        ));
                    }
                }
            } else {
                match mapping {
                    Some(m) => m,
                    None => {
                        return Ok(JsonRpcResponse::error(
                            request.id.unwrap_or(Value::Null),
                            JsonRpcError::resource_not_found(format!(
                                "Resource URI not found: {}",
                                uri
                            )),
                        ));
                    }
                }
            }
        };

        // Transform request based on routing method
        let mut transformed_request = request.clone();
        if resource_name.is_some() {
            // Routed by namespaced name - strip namespace from name parameter
            if let Some(params) = transformed_request.params.as_mut() {
                if let Some(obj) = params.as_object_mut() {
                    obj.insert("name".to_string(), json!(original_name));
                }
            }
        }
        // If routed by URI, leave parameters unchanged - backend will handle its own URIs

        // Route to server
        self.server_manager
            .send_request(&server_id, transformed_request)
            .await
    }

    /// Handle resources/subscribe request
    ///
    /// Subscribes to change notifications for a specific resource.
    /// When the resource changes, the backend server sends notifications/resources/updated.
    pub(crate) async fn handle_resources_subscribe(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract URI from params
        let uri = match request.params.as_ref().and_then(|p| p.get("uri")) {
            Some(Value::String(uri)) => uri.clone(),
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing or invalid 'uri' parameter".to_string()),
                ));
            }
        };

        // Look up the server that owns this resource
        let server_id = {
            let session_read = session.read().await;

            // First try resource_uri_mapping
            if let Some((server_id, _)) = session_read.resource_uri_mapping.get(&uri) {
                server_id.clone()
            } else {
                // If not found, we need to determine which server owns this resource
                // Try to match by URI prefix or pattern
                // For now, return an error if not found in mapping
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32602,
                        format!("Resource URI not found: {}. Call resources/list first to populate mappings.", uri),
                        None,
                    ),
                ));
            }
        };

        // Check if server supports subscriptions
        {
            let session_read = session.read().await;
            if let Some(caps) = &session_read.merged_capabilities {
                let supports_subscribe = caps
                    .capabilities
                    .resources
                    .as_ref()
                    .and_then(|r| r.subscribe)
                    .unwrap_or(false);

                if !supports_subscribe {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32601,
                            "Resource subscriptions not supported by backend servers".to_string(),
                            Some(json!({
                                "workaround": "Use notifications/resources/list_changed for general updates"
                            })),
                        ),
                    ));
                }
            }
        }

        // Forward subscription request to the backend server
        let backend_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            method: "resources/subscribe".to_string(),
            params: Some(json!({ "uri": uri })),
        };

        match self
            .server_manager
            .send_request(&server_id, backend_request)
            .await
        {
            Ok(response) => {
                // If successful, track the subscription in the session
                if response.error.is_none() {
                    let mut session_write = session.write().await;
                    session_write.subscribe_resource(uri.clone(), server_id.clone());
                    tracing::info!("Subscribed to resource {} on server {}", uri, server_id);
                }
                Ok(response)
            }
            Err(e) => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::custom(-32603, format!("Subscription failed: {}", e), None),
            )),
        }
    }

    /// Handle resources/unsubscribe request
    ///
    /// Unsubscribes from change notifications for a specific resource.
    pub(crate) async fn handle_resources_unsubscribe(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract URI from params
        let uri = match request.params.as_ref().and_then(|p| p.get("uri")) {
            Some(Value::String(uri)) => uri.clone(),
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing or invalid 'uri' parameter".to_string()),
                ));
            }
        };

        // Check if we're subscribed and get the server_id
        let server_id = {
            let session_read = session.read().await;
            match session_read.subscribed_resources.get(&uri) {
                Some(server_id) => server_id.clone(),
                None => {
                    // Not subscribed - return success anyway (idempotent)
                    return Ok(JsonRpcResponse::success(
                        request.id.unwrap_or(Value::Null),
                        json!({}),
                    ));
                }
            }
        };

        // Forward unsubscribe request to the backend server
        let backend_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            method: "resources/unsubscribe".to_string(),
            params: Some(json!({ "uri": uri })),
        };

        match self
            .server_manager
            .send_request(&server_id, backend_request)
            .await
        {
            Ok(response) => {
                // Remove the subscription from session tracking
                let mut session_write = session.write().await;
                session_write.unsubscribe_resource(&uri);
                tracing::info!("Unsubscribed from resource {} on server {}", uri, server_id);
                Ok(response)
            }
            Err(e) => {
                // Even on error, remove from local tracking
                let mut session_write = session.write().await;
                session_write.unsubscribe_resource(&uri);
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(-32603, format!("Unsubscribe failed: {}", e), None),
                ))
            }
        }
    }
}
