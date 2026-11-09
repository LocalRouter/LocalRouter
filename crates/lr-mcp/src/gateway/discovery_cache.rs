//! Last-known discovery, isolated by client, working directory and server config.
//! Deliberately memory-only: schemas may contain private upstream information.
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{gateway::McpGateway, session::GatewaySession, types::ServerFailure};

#[derive(Clone)]
pub(super) struct DiscoverySnapshot {
    pub updated: Instant,
    pub items: Vec<Value>,
    pub bytes: usize,
}

impl McpGateway {
    fn discovery_key(&self, state: &GatewaySession, id: &str, family: &str) -> Option<String> {
        let config = self.server_manager.get_config(id).filter(|c| c.enabled)?;
        // Hash configuration rather than keeping credentials in cache keys.
        let mut config = serde_json::to_value(config).ok()?;
        config.as_object_mut()?.remove("capability_snapshot");
        let bytes =
            serde_json::to_vec(&(state.client_id.as_str(), &state.client_cwd, config, family))
                .ok()?;
        Some(format!("{:x}", Sha256::digest(bytes)))
    }

    pub(super) fn cached_discovery<T: DeserializeOwned>(
        &self,
        state: &GatewaySession,
        id: &str,
        family: &str,
    ) -> Vec<T> {
        let Some(key) = self.discovery_key(state, id, family) else {
            return Vec::new();
        };
        self.discovery_cache
            .get(&key)
            .filter(|entry| entry.updated.elapsed() < Duration::from_secs(86400))
            .map(|entry| {
                entry
                    .items
                    .iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Only successful families replace snapshots (an empty success removes old
    /// items). A failed fetch keeps last-known discovery, never operation results.
    pub(super) fn remember_discovery<T: Serialize + DeserializeOwned + Clone>(
        &self,
        state: &GatewaySession,
        ids: &[String],
        family: &str,
        items: &mut Vec<T>,
        failures: &[ServerFailure],
        identity: impl Fn(&T) -> (&str, &str),
    ) {
        for id in &state.requested_servers {
            if failures.iter().any(|f| &f.server_id == id)
                || (!ids.contains(id)
                    && !matches!(
                        state.server_init_status.get(id),
                        Some(super::types::InitStatus::Completed(_))
                    ))
            {
                items.extend(self.cached_discovery::<T>(state, id, family));
                continue;
            }
            // A configuration edit during an in-flight fetch must not publish
            // the old endpoint's catalog under the new endpoint's identity.
            if state.transports.as_ref().is_some_and(|transports| {
                self.server_manager
                    .get_config(id)
                    .is_none_or(|config| !transports.configuration_matches(id, &config))
            }) {
                continue;
            }
            let Some(key) = self.discovery_key(state, id, family) else {
                continue;
            };
            let values: Vec<Value> = items
                .iter()
                .filter(|item| identity(item).0 == id)
                .filter_map(|item| {
                    let mut value = serde_json::to_value(item).ok()?;
                    // Routing fields are intentionally omitted from wire responses,
                    // but must survive snapshot deserialization.
                    value["server_id"] = Value::from(id.clone());
                    value["original_name"] = Value::from(identity(item).1);
                    Some(value)
                })
                .collect();
            let bytes = values.iter().map(|v| v.to_string().len()).sum::<usize>();
            if bytes > 4 * 1024 * 1024 {
                self.discovery_cache.remove(&key);
                continue;
            }
            while self.discovery_cache.len() >= 256
                || self
                    .discovery_cache
                    .iter()
                    .map(|entry| entry.bytes)
                    .sum::<usize>()
                    + bytes
                    > 32 * 1024 * 1024
            {
                let oldest = self
                    .discovery_cache
                    .iter()
                    .min_by_key(|e| e.updated)
                    .map(|e| e.key().clone());
                if let Some(oldest) = oldest {
                    self.discovery_cache.remove(&oldest);
                } else {
                    break;
                }
            }
            self.discovery_cache.insert(
                key,
                DiscoverySnapshot {
                    updated: Instant::now(),
                    items: values,
                    bytes,
                },
            );
        }
    }

    pub(super) fn visible_cached_templates(
        &self,
        state: &GatewaySession,
        items: Vec<Value>,
    ) -> Vec<Value> {
        items
            .into_iter()
            .filter(|item| {
                state
                    .mcp_permissions
                    .resolve_resource(
                        item["server_id"].as_str().unwrap_or(""),
                        item["uriTemplate"].as_str().unwrap_or(""),
                    )
                    .is_enabled()
            })
            .map(|mut item| {
                if let Some(obj) = item.as_object_mut() {
                    obj.remove("server_id");
                    obj.remove("original_name");
                }
                item
            })
            .collect()
    }

    /// Called by the server's maintenance timer, independent of client traffic.
    pub async fn recover_idle_backends(&self) {
        let sessions: Vec<_> = self.sessions.iter().map(|s| s.value().clone()).collect();
        for session in sessions {
            if session.read().await.is_expired() {
                continue;
            }
            self.recover_backends_for_request(
                &session,
                &crate::protocol::JsonRpcRequest::with_id(0, "tools/list".into(), None),
            )
            .await;
        }
    }
}
