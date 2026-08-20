//! Tauri commands for reverse-proxy clients.
//!
//! A reverse-proxy client wraps a local LLM provider: LocalRouter takes over
//! the port the provider used to own, and the provider moves aside. That
//! involves three pieces of state that must agree —
//!
//! 1. the **provider instance** in LocalRouter's own config (its `base_url`
//!    must point at the relocated address, or the gateway breaks),
//! 2. the **provider process** on the machine (actually listening there), and
//! 3. the **listener** LocalRouter binds on the original port.
//!
//! These commands move all three together and report which of them is out of
//! step, because a half-applied relocation is the failure mode users will hit.

use std::collections::HashMap;
use std::sync::Arc;

use lr_config::{ClientReverseProxy, ConfigManager};
use lr_providers::registry::ProviderRegistry;
use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::launcher::reverse_proxy::{ReverseListenerState, ReverseProxyService};
use crate::launcher::reverse_setup;
use crate::ui::commands_clients::LaunchResult;

/// Everything the reverse-proxy setup UI needs for one client.
#[derive(Serialize)]
pub struct ReverseProxySetupInfo {
    /// Display name of the wrapped provider ("Ollama").
    pub provider_label: String,
    /// Provider key (`ollama`, `lmstudio`, …), when the template maps to one.
    pub provider_key: Option<String>,
    /// Provider instance in LocalRouter's config that this client wraps.
    pub provider_instance: Option<String>,
    /// Address apps keep using.
    pub listen_host: String,
    pub listen_port: u16,
    /// Where the provider is expected to listen after relocation.
    pub upstream_url: String,
    /// State of LocalRouter's listener on the original port.
    pub listener: ReverseListenerState,
    /// Whether something is actually answering at the upstream address.
    pub upstream_reachable: bool,
    /// Whether the user chooses the ports (the generic template) or they are
    /// fixed by the provider we know about.
    pub ports_editable: bool,
    /// Whether LocalRouter can relocate this provider itself.
    pub supports_auto: bool,
    pub supports_undo: bool,
    /// Exactly what automatic relocation would run, so the user can see it
    /// before agreeing to it.
    pub auto_commands: Vec<String>,
    /// A command the user can run to start the provider on the new port.
    pub oneoff_command: Option<String>,
    /// GUI steps for providers that can't be relocated programmatically.
    pub manual_steps: Vec<String>,
    pub notes: Vec<String>,
    pub restart_hint: Option<String>,
}

/// Look up a client and its reverse-proxy binding, or explain what's missing.
fn client_binding(
    config_manager: &ConfigManager,
    client_id: &str,
) -> Result<(lr_config::Client, ClientReverseProxy), String> {
    let client = config_manager
        .get()
        .clients
        .into_iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| "Client not found".to_string())?;
    let binding = client
        .reverse_proxy
        .clone()
        .ok_or_else(|| "Client has no reverse-proxy configuration".to_string())?;
    Ok((client, binding))
}

/// Build the relocation plan for a client's binding.
fn plan_for_client(
    client: &lr_config::Client,
    binding: &ClientReverseProxy,
) -> reverse_setup::ReversePlan {
    let key = client
        .template_id
        .as_deref()
        .and_then(reverse_setup::provider_key_for_template)
        .unwrap_or("unknown");
    let upstream_port = upstream_port_of(binding);
    reverse_setup::plan_for(key, binding.listen_port, upstream_port)
}

/// Host and port of the binding's upstream, parsed the same way the data path
/// parses it — so a URL carrying a path (`…:1235/v1`) or an unsupported scheme
/// is read identically here and by the forwarder, instead of the two
/// disagreeing about whether the upstream is usable.
fn upstream_parts(binding: &ClientReverseProxy) -> Option<(String, u16)> {
    lr_proxy::reverse::parse_http_upstream(&binding.upstream_url).ok()
}

/// The port component of the binding's upstream URL (0 when unparseable).
fn upstream_port_of(binding: &ClientReverseProxy) -> u16 {
    upstream_parts(binding).map_or(0, |(_, port)| port)
}

/// Whether anything is listening at the upstream address.
async fn upstream_reachable(binding: &ClientReverseProxy) -> bool {
    let Some((host, port)) = upstream_parts(binding) else {
        return false;
    };
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Connection details, listener state, and the relocation plan for a client.
#[tauri::command]
pub async fn get_client_reverse_proxy_setup(
    client_id: String,
    app: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
) -> Result<ReverseProxySetupInfo, String> {
    let (client, binding) = client_binding(&config_manager, &client_id)?;
    let plan = plan_for_client(&client, &binding);
    let listener = app
        .try_state::<Arc<ReverseProxyService>>()
        .map(|s| s.state_for(&client_id))
        .unwrap_or_default();

    Ok(ReverseProxySetupInfo {
        provider_label: plan.provider_label.clone(),
        provider_key: client
            .template_id
            .as_deref()
            .and_then(reverse_setup::provider_key_for_template)
            .map(str::to_string),
        provider_instance: binding.provider_instance.clone(),
        listen_host: binding.listen_host.clone(),
        listen_port: binding.listen_port,
        upstream_url: binding.upstream_base().to_string(),
        upstream_reachable: upstream_reachable(&binding).await,
        ports_editable: reverse_setup::ports_are_editable(
            client
                .template_id
                .as_deref()
                .and_then(reverse_setup::provider_key_for_template),
        ),
        listener,
        supports_auto: plan.supports_auto(),
        supports_undo: plan.supports_undo(),
        auto_commands: plan.auto_commands(),
        oneoff_command: plan.oneoff_command.clone(),
        manual_steps: plan.manual_steps.clone(),
        notes: plan.notes.clone(),
        restart_hint: plan.restart_hint.clone(),
    })
}

/// Point a provider instance's `base_url` at a new port, preserving its scheme,
/// host and path. Ollama's base URL has no `/v1` suffix while the others do, so
/// only the authority is rewritten — never the whole URL.
pub(crate) fn retarget_base_url(base_url: &str, new_port: u16) -> String {
    let (scheme, rest) = match base_url.split_once("://") {
        Some((s, r)) => (s, r),
        None => ("http", base_url),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    // A provider entry with no base_url at all would otherwise produce
    // `http://:11435`, which nothing can connect to.
    let host = if host.is_empty() { "127.0.0.1" } else { host };
    format!("{scheme}://{host}:{new_port}{path}")
}

/// Move the wrapped provider instance onto the relocated port, in both the
/// running registry and the config file. Without this the gateway would keep
/// calling the old port — which is now LocalRouter itself, a loop.
async fn retarget_provider(
    registry: &ProviderRegistry,
    config_manager: &ConfigManager,
    instance_name: &str,
    new_port: u16,
) -> Result<String, String> {
    let config = config_manager.get();
    let provider = config
        .providers
        .iter()
        .find(|p| p.name == instance_name)
        .ok_or_else(|| format!("Provider instance '{instance_name}' not found"))?;

    let mut values: HashMap<String, String> = provider
        .provider_config
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            obj.into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (k, s)
                })
                .collect()
        })
        .unwrap_or_default();

    let current = values.get("base_url").cloned().unwrap_or_default();
    let new_base = retarget_base_url(&current, new_port);
    values.insert("base_url".to_string(), new_base.clone());

    // Keep the in-memory provider's API key: the registry rebuilds the instance
    // from this map, and the key lives in the keychain, not in the config file.
    if !values.contains_key("api_key") {
        if let Ok(Some(key)) = lr_providers::key_storage::get_provider_key(instance_name) {
            values.insert("api_key".to_string(), key);
        }
    }

    let provider_type = serde_json::to_value(provider.provider_type)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| "Unsupported provider type".to_string())?;

    registry
        .update_provider(instance_name.to_string(), provider_type, values)
        .await
        .map_err(|e| e.to_string())?;

    // Persist without secrets — the keychain already holds them.
    let name = instance_name.to_string();
    config_manager
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.name == name) {
                if let Some(serde_json::Value::Object(obj)) = p.provider_config.as_mut() {
                    obj.insert(
                        "base_url".to_string(),
                        serde_json::Value::String(new_base.clone()),
                    );
                } else {
                    p.provider_config = Some(serde_json::json!({ "base_url": new_base.clone() }));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(new_base)
}

/// Relocate the provider, retarget its instance config, and start the listener.
///
/// Every step is reported even when a later one fails, so the UI can say
/// exactly how far the relocation got.
#[tauri::command]
pub async fn configure_client_reverse_proxy(
    client_id: String,
    app: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<LaunchResult, String> {
    let (client, binding) = client_binding(&config_manager, &client_id)?;
    let plan = plan_for_client(&client, &binding);
    let upstream_port = upstream_port_of(&binding);
    let mut steps: Vec<String> = Vec::new();

    // 0. Release our own listener first. Relocation decides whether the
    //    provider has moved by watching the original port — and if LocalRouter
    //    is already bound to it (a re-run, or a listener restored at startup),
    //    that port never frees and the wait times out blaming the provider.
    if let Some(service) = app.try_state::<Arc<ReverseProxyService>>() {
        service.stop_client(&client_id);
    }

    // 1. Point LocalRouter's own provider instance at the new address first.
    //    Doing this before the move means that if the user aborts, the gateway
    //    is pointing somewhere harmless rather than at our own listener.
    if let Some(instance) = binding.provider_instance.as_deref() {
        match retarget_provider(&registry, &config_manager, instance, upstream_port).await {
            Ok(url) => steps.push(format!("Provider '{instance}' now points at {url}")),
            Err(e) => steps.push(format!("Could not retarget provider '{instance}': {e}")),
        }
    }

    // 2. Relocate the provider process itself, when we know how. This verifies
    //    the outcome (old port released, new port answering) rather than
    //    trusting exit codes — providers do refuse scripted quits.
    if plan.supports_auto() {
        match reverse_setup::relocate(&plan, binding.listen_port, upstream_port, &plan.configure)
            .await
        {
            Ok(result) => steps.push(result.message),
            Err(e) => {
                return Ok(LaunchResult {
                    success: false,
                    message: format!(
                        "{}\nCould not relocate {}: {e}",
                        steps.join("\n"),
                        plan.provider_label
                    ),
                    modified_files: vec![],
                    backup_files: vec![],
                    terminal_command: plan.oneoff_command.clone(),
                })
            }
        }
    }

    // 3. Bind the original port. This retries briefly: a just-restarted
    //    provider can still be holding the old socket.
    let listener_msg = match app.try_state::<Arc<ReverseProxyService>>() {
        Some(service) => match service.start_client(&client).await {
            Ok(port) => format!("Listening on {}:{port}", binding.listen_host),
            Err(e) => {
                let hint = if plan.supports_auto() {
                    "the provider may still be starting — try Start listener again in a moment"
                } else {
                    "relocate the provider first, then click Start listener"
                };
                return Ok(LaunchResult {
                    success: false,
                    message: format!("{}\nListener not started: {e} — {hint}", steps.join("\n")),
                    modified_files: vec![],
                    backup_files: vec![],
                    terminal_command: plan.oneoff_command.clone(),
                });
            }
        },
        None => "Reverse proxy service unavailable".to_string(),
    };
    steps.push(listener_msg);

    let _ = app.emit("clients-changed", ());
    let _ = app.emit("providers-changed", ());
    Ok(LaunchResult {
        success: true,
        message: steps.join("\n"),
        modified_files: vec![],
        backup_files: vec![],
        terminal_command: None,
    })
}

/// Put everything back: stop the listener, move the provider home, and point
/// its instance config back at the original port.
#[tauri::command]
pub async fn unconfigure_client_reverse_proxy(
    client_id: String,
    app: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<LaunchResult, String> {
    let (client, binding) = client_binding(&config_manager, &client_id)?;
    let plan = plan_for_client(&client, &binding);
    let mut steps: Vec<String> = Vec::new();

    // Free the port before the provider tries to take it back.
    if let Some(service) = app.try_state::<Arc<ReverseProxyService>>() {
        service.stop_client(&client_id);
        steps.push(format!("Listener on port {} stopped", binding.listen_port));
    }

    let key = client
        .template_id
        .as_deref()
        .and_then(reverse_setup::provider_key_for_template);
    // LM Studio's undo needs the original port, which the forward plan doesn't
    // carry (it only knows where the provider was going).
    let undo_plan = if key == Some("lmstudio") {
        reverse_setup::lmstudio_undo_plan(binding.listen_port)
    } else {
        plan.clone().into_undo()
    };

    if undo_plan.supports_auto() {
        // Moving home reverses the ports: the relocated port is freed and the
        // original one must answer again.
        match reverse_setup::relocate(
            &undo_plan,
            upstream_port_of(&binding),
            binding.listen_port,
            &undo_plan.configure,
        )
        .await
        {
            Ok(result) => steps.push(result.message),
            Err(e) => steps.push(format!("Could not move {} back: {e}", plan.provider_label)),
        }
    } else if !plan.manual_steps.is_empty() {
        steps.push(format!(
            "Set {} back to port {} yourself — LocalRouter can't do it for this provider.",
            plan.provider_label, binding.listen_port
        ));
    }

    if let Some(instance) = binding.provider_instance.as_deref() {
        match retarget_provider(&registry, &config_manager, instance, binding.listen_port).await {
            Ok(url) => steps.push(format!("Provider '{instance}' points back at {url}")),
            Err(e) => steps.push(format!("Could not retarget provider '{instance}': {e}")),
        }
    }

    let _ = app.emit("clients-changed", ());
    let _ = app.emit("providers-changed", ());
    Ok(LaunchResult {
        success: true,
        message: steps.join("\n"),
        modified_files: vec![],
        backup_files: vec![],
        terminal_command: None,
    })
}

/// Start (or retry) just the listener, without touching the provider. This is
/// the button for providers we can't relocate automatically: the user moves the
/// provider themselves, then binds the port.
#[tauri::command]
pub async fn start_client_reverse_proxy(
    client_id: String,
    app: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
) -> Result<ReverseListenerState, String> {
    let (client, _binding) = client_binding(&config_manager, &client_id)?;
    let service = app
        .try_state::<Arc<ReverseProxyService>>()
        .ok_or_else(|| "Reverse proxy service unavailable".to_string())?;
    if let Err(e) = service.start_client(&client).await {
        return Err(e.to_string());
    }
    let _ = app.emit("clients-changed", ());
    Ok(service.state_for(&client_id))
}

/// Stop a client's listener, freeing the port.
#[tauri::command]
pub async fn stop_client_reverse_proxy(
    client_id: String,
    app: tauri::AppHandle,
) -> Result<ReverseListenerState, String> {
    let service = app
        .try_state::<Arc<ReverseProxyService>>()
        .ok_or_else(|| "Reverse proxy service unavailable".to_string())?;
    service.stop_client(&client_id);
    let _ = app.emit("clients-changed", ());
    Ok(service.state_for(&client_id))
}

/// Change a client's ports/upstream, restarting its listener if it was running.
#[tauri::command]
pub async fn set_client_reverse_proxy_config(
    client_id: String,
    listen_port: u16,
    upstream_url: String,
    provider_instance: Option<String>,
    app: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let cid = client_id.clone();
    let url = upstream_url.trim().to_string();
    config_manager
        .update(|cfg| {
            if let Some(c) = cfg.clients.iter_mut().find(|c| c.id == cid) {
                let existing = c.reverse_proxy.clone();
                c.reverse_proxy = Some(ClientReverseProxy {
                    listen_host: existing
                        .as_ref()
                        .map(|r| r.listen_host.clone())
                        .unwrap_or_else(|| "127.0.0.1".to_string()),
                    listen_port,
                    upstream_url: url.clone(),
                    provider_instance: provider_instance
                        .clone()
                        .or_else(|| existing.and_then(|r| r.provider_instance)),
                });
            }
        })
        .map_err(|e| e.to_string())?;
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Re-bind if this client is live.
    if let Some(service) = app.try_state::<Arc<ReverseProxyService>>() {
        service.sync().await;
    }
    let _ = app.emit("clients-changed", ());
    Ok(())
}

/// Default binding for a reverse-proxy template, used by the creation wizard.
#[derive(Serialize)]
pub struct ReverseProxyDefaults {
    pub provider_key: String,
    pub provider_label: String,
    pub listen_port: u16,
    pub upstream_url: String,
    /// Matching provider instance already configured in LocalRouter, if any.
    pub provider_instance: Option<String>,
    /// Whether that provider currently answers on the port we'd take over.
    pub provider_detected: bool,
}

/// Suggested reverse-proxy settings for a template, resolved against the
/// providers this LocalRouter already knows about.
#[tauri::command]
pub async fn get_reverse_proxy_defaults(
    template_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<ReverseProxyDefaults, String> {
    let key = reverse_setup::provider_key_for_template(&template_id)
        .ok_or_else(|| format!("'{template_id}' is not a reverse-proxy template"))?;
    let (listen_port, upstream_port) =
        reverse_setup::default_ports(key).ok_or_else(|| "Unknown provider".to_string())?;
    let plan = reverse_setup::plan_for(key, listen_port, upstream_port);

    // Match an existing provider instance by type, so the new client wraps the
    // provider the user already configured rather than a guess.
    let type_name = reverse_setup::provider_type_name(key);
    let config = config_manager.get();
    let instance = config
        .providers
        .iter()
        .find(|p| {
            serde_json::to_value(p.provider_type)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .is_some_and(|t| t == type_name)
        })
        .map(|p| p.name.clone());

    // Is the provider actually on the port we'd take over right now? If so,
    // relocation is genuinely needed (rather than the port being free already).
    let detected = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", listen_port)),
    )
    .await
    .is_ok_and(|r| r.is_ok());

    Ok(ReverseProxyDefaults {
        provider_key: key.to_string(),
        provider_label: plan.provider_label,
        listen_port,
        upstream_url: format!("http://127.0.0.1:{upstream_port}"),
        provider_instance: instance,
        provider_detected: detected,
    })
}

/// Reconcile every listener with config (startup, and after client changes).
pub async fn sync_reverse_proxies(app: &tauri::AppHandle) {
    if let Some(service) = app.try_state::<Arc<ReverseProxyService>>() {
        service.sync().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retargets_only_the_port() {
        // Ollama: no path suffix.
        assert_eq!(
            retarget_base_url("http://localhost:11434", 11435),
            "http://localhost:11435"
        );
        // LM Studio and friends: `/v1` suffix must survive.
        assert_eq!(
            retarget_base_url("http://localhost:1234/v1", 1235),
            "http://localhost:1235/v1"
        );
        // A non-default host is preserved.
        assert_eq!(
            retarget_base_url("http://192.168.1.5:11434", 11435),
            "http://192.168.1.5:11435"
        );
        // Missing port.
        assert_eq!(
            retarget_base_url("http://localhost/v1", 8081),
            "http://localhost:8081/v1"
        );
        // Missing scheme falls back to http rather than producing garbage.
        assert_eq!(
            retarget_base_url("localhost:11434", 11435),
            "http://localhost:11435"
        );
        // Empty input still yields something connectable rather than
        // `http://:11435`, which no client could use.
        assert_eq!(retarget_base_url("", 11435), "http://127.0.0.1:11435");
    }

    #[test]
    fn reads_the_upstream_port_out_of_a_binding() {
        let binding = ClientReverseProxy {
            listen_host: "127.0.0.1".into(),
            listen_port: 11434,
            upstream_url: "http://127.0.0.1:11435/".into(),
            provider_instance: None,
        };
        assert_eq!(upstream_port_of(&binding), 11435);

        // A URL carrying a path is read the same way the forwarder reads it.
        let with_path = ClientReverseProxy {
            upstream_url: "http://127.0.0.1:1235/v1".into(),
            ..binding.clone()
        };
        assert_eq!(upstream_port_of(&with_path), 1235);
        assert_eq!(
            upstream_parts(&with_path),
            Some(("127.0.0.1".to_string(), 1235))
        );

        // A portless URL defaults to 80 rather than being treated as broken.
        let portless = ClientReverseProxy {
            upstream_url: "http://127.0.0.1".into(),
            ..binding.clone()
        };
        assert_eq!(upstream_port_of(&portless), 80);

        // The data path only speaks plain http, so status must not claim an
        // https upstream is usable.
        let https = ClientReverseProxy {
            upstream_url: "https://127.0.0.1:11435".into(),
            ..binding
        };
        assert_eq!(upstream_port_of(&https), 0);
    }
}
