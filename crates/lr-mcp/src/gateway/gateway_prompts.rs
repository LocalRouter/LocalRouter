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
use super::gateway_tools::apply_catalog_compression_prompts;

impl McpGateway {
    /// Handle prompts/list request
    pub(crate) async fn handle_prompts_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check cache
        if let Some(cached) = &session_read.cached_prompts {
            if cached.is_valid() {
                let prompts = apply_catalog_compression_prompts(
                    &cached.data,
                    session_read.catalog_compression.as_ref(),
                    session_read.activated_prompts(),
                );
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

        // Apply catalog compression (filter deferred prompts)
        let prompts = {
            let session_read = session.read().await;
            let filtered = apply_catalog_compression_prompts(
                &prompts,
                session_read.catalog_compression.as_ref(),
                session_read.activated_prompts(),
            );
            filtered
        };

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

        // Parse prompts from results, exhausting pagination for each server
        let mut server_prompts: Vec<(String, Vec<McpPrompt>)> = Vec::new();

        for (server_id, value) in successes {
            let mut all_items: Vec<McpPrompt> = value
                .get("prompts")
                .and_then(|p| serde_json::from_value::<Vec<McpPrompt>>(p.clone()).ok())
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
                        "prompts/list pagination: hit max page limit ({}) for server {}",
                        MAX_PAGES,
                        server_id
                    );
                    break;
                }

                let page_request = JsonRpcRequest::new(
                    Some(json!(format!("page-{}", page))),
                    "prompts/list".to_string(),
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
                            if let Some(prompts) = result.get("prompts").and_then(|p| {
                                serde_json::from_value::<Vec<McpPrompt>>(p.clone()).ok()
                            }) {
                                all_items.extend(prompts);
                            }
                            next_cursor = result
                                .get("nextCursor")
                                .and_then(|c| c.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "prompts/list pagination error for server {} (page {}): {}",
                            server_id,
                            page,
                            e
                        );
                        break;
                    }
                    Err(_) => {
                        tracing::warn!(
                            "prompts/list pagination timeout for server {} (page {})",
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
                        "prompts/list: fetched {} prompts from server {} across {} pages",
                        all_items.len(),
                        server_id,
                        page
                    );
                }
                server_prompts.push((server_id, all_items));
            }
        }

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

        // Emit pending monitor event for prompt get
        let (mon_client_id, mon_client_name, mon_session_id) = {
            let s = session.read().await;
            (
                Some(s.client_id.clone()),
                Some(s.client_name.clone()),
                s.monitor_session_id.clone(),
            )
        };
        let mon_args = transformed_request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let mon_event_id = self.emit_monitor_event(
            lr_monitor::MonitorEventType::McpPromptGet,
            mon_client_id,
            mon_client_name,
            mon_session_id,
            lr_monitor::MonitorEventData::McpPromptGet {
                prompt_name: prompt_name.to_string(),
                server_id: server_id.clone(),
                server_name: None,
                arguments: mon_args,
                latency_ms: None,
                success: None,
                content_preview: None,
                error: None,
            },
            lr_monitor::EventStatus::Pending,
            None,
        );

        let start = std::time::Instant::now();

        // Route to server
        let result = self
            .server_manager
            .send_request(&server_id, transformed_request)
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        // Update monitor event with response data
        let (success, response_preview, error_msg) = match &result {
            Ok(resp) => {
                let preview = resp
                    .result
                    .as_ref()
                    .map(|r| {
                        let s = serde_json::to_string(r).unwrap_or_default();
                        if s.len() > 2000 {
                            format!("{}...", &s[..2000])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();
                let err = resp.error.as_ref().map(|e| e.message.clone());
                (resp.error.is_none(), preview, err)
            }
            Err(e) => (false, String::new(), Some(e.to_string())),
        };
        let status = if success {
            lr_monitor::EventStatus::Complete
        } else {
            lr_monitor::EventStatus::Error
        };
        self.update_monitor_event(
            &mon_event_id,
            Box::new(move |event| {
                event.status = status;
                event.duration_ms = Some(latency_ms);
                if let lr_monitor::MonitorEventData::McpPromptGet {
                    latency_ms: ref mut lm,
                    success: ref mut s,
                    content_preview: ref mut cp,
                    error: ref mut e,
                    ..
                } = &mut event.data
                {
                    *lm = Some(latency_ms);
                    *s = Some(success);
                    *cp = Some(response_preview);
                    *e = error_msg;
                }
            }),
        );

        result
    }
}
