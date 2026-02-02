use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpPrompt};
use lr_types::{AppError, AppResult};

use super::merger::merge_prompts;
use super::router::{broadcast_request, separate_results};
use super::session::GatewaySession;
use super::types::*;

use super::gateway::McpGateway;

impl McpGateway {
    /// Handle prompts/list request
    pub(crate) async fn handle_prompts_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check for deferred loading (only if client supports prompts.listChanged)
        if let Some(deferred) = &session_read.deferred_loading {
            if deferred.enabled && deferred.prompts_deferred {
                // Return only activated prompts
                let prompts: Vec<serde_json::Value> = deferred
                    .full_prompt_catalog
                    .iter()
                    .filter(|p| deferred.activated_prompts.contains(&p.name))
                    .map(|p| serde_json::to_value(p).unwrap_or_default())
                    .collect();

                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"prompts": prompts}),
                ));
            }
        }

        // Check cache
        if let Some(cached) = &session_read.cached_prompts {
            if cached.is_valid() {
                let prompts = cached.data.clone();
                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"prompts": prompts}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Fetch from servers
        let (prompts, failures) = self
            .fetch_and_merge_prompts(&allowed_servers, request.clone())
            .await?;

        // Update session mappings, cache, and failures
        {
            let mut session_write = session.write().await;
            session_write.update_prompt_mappings(&prompts);
            session_write.last_broadcast_failures = failures.clone();

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_prompts = Some(CachedList::new(prompts.clone(), cache_ttl));
        }

        let mut result = json!({"prompts": prompts});
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

    /// Fetch and merge prompts from servers
    pub(crate) async fn fetch_and_merge_prompts(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedPrompt>, Vec<ServerFailure>)> {
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

        // Parse prompts from results
        let server_prompts: Vec<(String, Vec<McpPrompt>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("prompts")
                    .and_then(|p| serde_json::from_value::<Vec<McpPrompt>>(p.clone()).ok())
                    .map(|prompts| (server_id, prompts))
            })
            .collect();

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_prompts(server_prompts, &failures, Some(&name_mapping)),
            failures,
        ))
    }

    /// Handle prompts/get request
    pub(crate) async fn handle_prompts_get(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract prompt name from params
        let prompt_name = match request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(name) => name,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing prompt name in params"),
                ));
            }
        };

        // Look up prompt in session mapping to get server_id (UUID) and original_name
        let session_read = session.read().await;
        let (server_id, original_name) = match session_read.prompt_mapping.get(prompt_name) {
            Some((id, name)) => (id.clone(), name.clone()),
            None => {
                drop(session_read);
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::prompt_not_found(prompt_name),
                ));
            }
        };
        drop(session_read);

        // Transform request: Strip namespace
        let mut transformed_request = request.clone();
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        // Route to server
        self.server_manager
            .send_request(&server_id, transformed_request)
            .await
    }
}
