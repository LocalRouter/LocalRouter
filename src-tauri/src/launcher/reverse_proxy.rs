//! App-side wiring for reverse-proxy listeners.
//!
//! One listener per client in [`LlmMode::ReverseProxy`], each bound to the port
//! a local provider used to own and forwarding to wherever that provider was
//! relocated. Listeners are reconciled from config: [`ReverseProxyService::sync`]
//! starts what should be running, stops what shouldn't, and restarts anything
//! whose binding changed.
//!
//! Bind failures are recorded, not fatal. The overwhelmingly common one is
//! "the provider is still sitting on that port" (relocation not done yet), and
//! that has to surface in the setup UI as an explainable state rather than
//! taking the app down.

use std::collections::HashMap;
use std::sync::Arc;

use lr_config::{Client, ConfigManager};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::reverse::{ReverseClient, ReverseProxy, ReverseRecorder};
use lr_types::{AppError, AppResult};
use parking_lot::Mutex;

use crate::launcher::proxy::{AppClientNames, CatalogPricing};

/// How long to keep retrying a bind, for the window where a relocated provider
/// is still releasing its old port.
const BIND_RETRIES: u32 = 10;
const BIND_RETRY_DELAY_MS: u64 = 300;

/// A live listener owned by one client.
struct RunningListener {
    port: u16,
    /// The upstream exactly as configured. Compared against config on sync, so
    /// it must be the configured string — comparing against the *normalized*
    /// form would differ whenever the URL carries a path (`…:1235/v1`) and
    /// restart the listener on every sync.
    configured_upstream: String,
    /// Where traffic actually goes (normalized), for display.
    upstream: String,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

/// Current state of one client's reverse-proxy listener, for the UI.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReverseListenerState {
    pub running: bool,
    pub port: Option<u16>,
    pub upstream: Option<String>,
    /// Why the listener isn't running (usually: the port is still occupied).
    pub error: Option<String>,
}

/// Owns every reverse-proxy listener and reconciles them against config.
pub struct ReverseProxyService {
    recorder: Arc<PassiveInterceptor>,
    config_manager: ConfigManager,
    running: Mutex<HashMap<String, RunningListener>>,
    /// Last bind/start failure per client, cleared on success.
    errors: Mutex<HashMap<String, String>>,
}

impl ReverseProxyService {
    pub fn new(
        monitor_store: Arc<lr_monitor::MonitorEventStore>,
        metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
        client_manager: Arc<lr_clients::ClientManager>,
        config_manager: ConfigManager,
    ) -> Self {
        // Same recorder the MITM proxy uses, so reverse-proxied calls land in
        // the Monitor and the dashboards identically (tagged ReverseProxy).
        let recorder = PassiveInterceptor::new(monitor_store)
            .with_metrics(metrics_collector)
            .with_pricing(Arc::new(CatalogPricing))
            .with_client_names(Arc::new(AppClientNames { client_manager }));
        Self {
            recorder: Arc::new(recorder),
            config_manager,
            running: Mutex::new(HashMap::new()),
            errors: Mutex::new(HashMap::new()),
        }
    }

    /// State of one client's listener.
    pub fn state_for(&self, client_id: &str) -> ReverseListenerState {
        let running = self.running.lock();
        match running.get(client_id) {
            Some(l) => ReverseListenerState {
                running: true,
                port: Some(l.port),
                upstream: Some(l.upstream.clone()),
                error: None,
            },
            None => ReverseListenerState {
                running: false,
                port: None,
                upstream: None,
                error: self.errors.lock().get(client_id).cloned(),
            },
        }
    }

    /// Bind and serve a listener for `client`. Replaces any existing listener
    /// for that client. Returns the bound port.
    pub async fn start_client(&self, client: &Client) -> AppResult<u16> {
        let rp = client
            .active_reverse_proxy()
            .ok_or_else(|| AppError::Config("client has no reverse-proxy configuration".into()))?
            .clone();

        // Drop any previous listener first so the port is free to rebind.
        self.stop_client(&client.id);

        let proxy = Arc::new(
            ReverseProxy::new(
                &rp.upstream_url,
                ReverseClient {
                    client_id: client.id.clone(),
                    strategy_id: client.strategy_id.clone(),
                    provider_instance: rp.provider_instance.clone(),
                },
                self.recorder.clone() as Arc<dyn ReverseRecorder>,
            )
            .map_err(|e| AppError::Config(e.to_string()))?,
        );

        let listener = self
            .bind_with_retry(&rp.listen_host, rp.listen_port)
            .await?;
        let bound = listener
            .local_addr()
            .map_err(|e| AppError::Internal(format!("reverse proxy local_addr: {e}")))?
            .port();

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let upstream = proxy.upstream().to_string();
        tokio::spawn(async move {
            proxy
                .serve(listener, async {
                    let _ = rx.await;
                })
                .await;
        });

        self.running.lock().insert(
            client.id.clone(),
            RunningListener {
                port: bound,
                configured_upstream: rp.upstream_base().to_string(),
                upstream: upstream.clone(),
                shutdown: tx,
            },
        );
        self.errors.lock().remove(&client.id);
        tracing::info!(
            client = %client.name,
            "reverse proxy listening on {}:{bound} → {upstream}",
            rp.listen_host
        );
        Ok(bound)
    }

    /// Bind, retrying briefly while a just-relocated provider releases the port.
    async fn bind_with_retry(&self, host: &str, port: u16) -> AppResult<tokio::net::TcpListener> {
        let mut last: Option<String> = None;
        for attempt in 0..BIND_RETRIES {
            match ReverseProxy::bind(host, port).await {
                Ok(l) => return Ok(l),
                Err(e) => {
                    last = Some(e.to_string());
                    if attempt + 1 < BIND_RETRIES {
                        tokio::time::sleep(std::time::Duration::from_millis(BIND_RETRY_DELAY_MS))
                            .await;
                    }
                }
            }
        }
        Err(AppError::Internal(format!(
            "could not bind {host}:{port} — is the wrapped provider still listening there? ({})",
            last.unwrap_or_default()
        )))
    }

    /// Stop a client's listener if one is running.
    pub fn stop_client(&self, client_id: &str) {
        if let Some(l) = self.running.lock().remove(client_id) {
            let _ = l.shutdown.send(());
            tracing::info!(
                client_id,
                "reverse proxy listener stopped (port {})",
                l.port
            );
        }
    }

    /// Record why a client's listener isn't running, for the setup UI.
    fn note_error(&self, client_id: &str, msg: String) {
        self.errors.lock().insert(client_id.to_string(), msg);
    }

    /// Reconcile every listener against the current config: start listeners for
    /// enabled reverse-proxy clients, stop the rest, and restart any whose
    /// binding changed.
    pub async fn sync(&self) {
        let config = self.config_manager.get();

        let wanted: Vec<Client> = config
            .clients
            .iter()
            .filter(|c| c.enabled && c.active_reverse_proxy().is_some())
            .cloned()
            .collect();
        let wanted_ids: Vec<String> = wanted.iter().map(|c| c.id.clone()).collect();

        // Stop listeners that should no longer exist.
        let stale: Vec<String> = self
            .running
            .lock()
            .keys()
            .filter(|id| !wanted_ids.contains(id))
            .cloned()
            .collect();
        for id in stale {
            self.stop_client(&id);
            self.errors.lock().remove(&id);
        }

        for client in wanted {
            let rp = client
                .active_reverse_proxy()
                .expect("filtered above")
                .clone();
            // Already serving this exact binding? Leave it alone — restarting
            // would drop in-flight requests for no reason.
            let unchanged = self.running.lock().get(&client.id).is_some_and(|l| {
                l.port == rp.listen_port && l.configured_upstream == rp.upstream_base()
            });
            if unchanged {
                continue;
            }
            if let Err(e) = self.start_client(&client).await {
                tracing::warn!(client = %client.name, "reverse proxy not started: {e}");
                self.note_error(&client.id, e.to_string());
            }
        }
    }

    /// Stop every listener (app teardown).
    pub fn stop_all(&self) {
        let ids: Vec<String> = self.running.lock().keys().cloned().collect();
        for id in ids {
            self.stop_client(&id);
        }
    }
}
