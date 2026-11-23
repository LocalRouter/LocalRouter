//! Which upstream owns a resource named in `resources/read` or
//! `resources/subscribe`, without requiring the client to list first.
//!
//! Order: the session's mappings; a fresh `resources/list` of the servers
//! that offer resources (the mapping may predate a `list_changed`, or no list
//! ran yet); then, for a namespaced name, its `server__` prefix; for a URI,
//! the only resource server, else the server whose resource template's
//! literal prefix matches the URI longest. A URI resolved by a template is
//! remembered in the session's URI mapping until the next list replaces it.

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::protocol::JsonRpcRequest;
use lr_types::{AppError, AppResult};

use super::gateway::McpGateway;
use super::router::{broadcast_request, separate_results};
use super::session::GatewaySession;
use super::types::{parse_namespace, slugify, InitStatus};

/// The literal text of a URI template before its first expression
/// (`file:///logs/{date}` → `file:///logs/`). `None` for an empty prefix,
/// which would match every URI.
pub(crate) fn template_prefix(template: &str) -> Option<&str> {
    let prefix = template.split('{').next().unwrap_or_default();
    (!prefix.is_empty()).then_some(prefix)
}

/// The server whose template prefix is the longest prefix of `uri`; `None`
/// when no template matches or two servers tie for the longest.
pub(crate) fn server_for_templated_uri<'a>(
    uri: &str,
    templates: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    let mut tied = false;
    for (server_id, template) in templates {
        let Some(prefix) = template_prefix(template) else {
            continue;
        };
        if !uri.starts_with(prefix) {
            continue;
        }
        match best {
            Some((len, id)) if prefix.len() == len && id != server_id => tied = true,
            Some((len, _)) if prefix.len() <= len => {}
            _ => {
                best = Some((prefix.len(), server_id));
                tied = false;
            }
        }
    }
    if tied {
        return None;
    }
    best.map(|(_, id)| id.to_string())
}

impl McpGateway {
    /// `(server id, original name)` of the resource `name` or `uri` refers
    /// to, or `None` when no allowed server owns it.
    pub(crate) async fn resolve_resource_target(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        name: Option<&str>,
        uri: Option<&str>,
    ) -> AppResult<Option<(String, String)>> {
        let lookup = |state: &GatewaySession| {
            name.and_then(|n| state.resource_mapping.get(n).cloned())
                .or_else(|| uri.and_then(|u| state.resource_uri_mapping.get(u).cloned()))
        };
        let (resource_servers, transports) = {
            let state = session.read().await;
            if let Some(hit) = lookup(&state) {
                return Ok(Some(hit));
            }
            let servers: Vec<String> = state
                .allowed_servers
                .iter()
                .filter(|id| {
                    matches!(
                        state.server_init_status.get(*id),
                        Some(InitStatus::Completed(init)) if init.capabilities.resources.is_some()
                    )
                })
                .cloned()
                .collect();
            (servers, state.transports.clone())
        };
        let transports =
            transports.ok_or_else(|| AppError::Mcp("Session not initialized".to_string()))?;
        if resource_servers.is_empty() {
            return Ok(None);
        }

        // A fresh listing of the servers that offer resources
        let (resources, failures) = self
            .fetch_and_merge_resources(
                &resource_servers,
                JsonRpcRequest::new(Some(json!("auto")), "resources/list".to_string(), None),
                &transports,
            )
            .await?;
        {
            let mut state = session.write().await;
            state.update_resource_mappings(&resources, &failures);
            state.resources_list_fetched = true;
            if let Some(hit) = lookup(&state) {
                return Ok(Some(hit));
            }
        }

        // A namespaced name of a server whose list did not carry it
        if let Some((prefix, original)) = name.and_then(parse_namespace) {
            let owner = resource_servers.iter().find(|id| {
                self.server_manager
                    .get_config(id)
                    .is_some_and(|config| slugify(&config.name) == prefix)
            });
            if let Some(owner) = owner {
                return Ok(Some((owner.clone(), original)));
            }
        }

        let Some(uri) = uri else {
            return Ok(None);
        };
        let owner = if let [only] = resource_servers.as_slice() {
            Some(only.clone())
        } else {
            self.templated_uri_owner(&resource_servers, uri, &transports)
                .await
        };
        let Some(owner) = owner else {
            return Ok(None);
        };
        session
            .write()
            .await
            .resource_uri_mapping
            .insert(uri.to_string(), (owner.clone(), uri.to_string()));
        Ok(Some((owner, uri.to_string())))
    }

    /// The server whose `resources/templates/list` has the template that
    /// matches `uri` best (first page only: this is a routing hint).
    async fn templated_uri_owner(
        &self,
        servers: &[String],
        uri: &str,
        transports: &Arc<crate::transport::SessionTransportSet>,
    ) -> Option<String> {
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let results = broadcast_request(
            servers,
            JsonRpcRequest::new(
                Some(json!("auto-templates")),
                "resources/templates/list".to_string(),
                None,
            ),
            transports,
            timeout,
            0,
        )
        .await;
        let (successes, _) = separate_results(results);
        let templates: Vec<(String, String)> = successes
            .iter()
            .flat_map(|(server_id, value)| {
                value
                    .get("resourceTemplates")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.get("uriTemplate").and_then(Value::as_str))
                    .map(|t| (server_id.clone(), t.to_string()))
            })
            .collect();
        server_for_templated_uri(
            uri,
            templates.iter().map(|(id, t)| (id.as_str(), t.as_str())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_prefix_is_the_literal_head() {
        assert_eq!(
            template_prefix("file:///logs/{date}"),
            Some("file:///logs/")
        );
        assert_eq!(template_prefix("db://t"), Some("db://t"));
        assert_eq!(template_prefix("{scheme}://x"), None);
    }

    #[test]
    fn longest_template_prefix_wins_and_ties_resolve_to_nothing() {
        let templates = [
            ("a", "file:///{path}"),
            ("b", "file:///logs/{date}"),
            ("c", "db://{table}"),
        ];
        assert_eq!(
            server_for_templated_uri("file:///logs/2026", templates).as_deref(),
            Some("b")
        );
        assert_eq!(
            server_for_templated_uri("file:///etc/hosts", templates).as_deref(),
            Some("a")
        );
        assert_eq!(server_for_templated_uri("s3://bucket", templates), None);
        let tie = [("a", "x://{p}"), ("b", "x://{q}")];
        assert_eq!(server_for_templated_uri("x://1", tie), None);
        let same_server = [("a", "x://{p}"), ("a", "x://{q}")];
        assert_eq!(
            server_for_templated_uri("x://1", same_server).as_deref(),
            Some("a")
        );
    }
}
