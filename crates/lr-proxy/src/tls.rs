//! rustls configuration for the MITM data-path.
//!
//! - **Server side**: per-host `ServerConfig` presenting a forged leaf (from the
//!   [`CertAuthority`]), offering only ALPN `http/1.1` so the client speaks
//!   HTTP/1.1 and we avoid HTTP/2 framing.
//! - **Client side**: one `ClientConfig` validating the *genuine* upstream cert
//!   against the OS trust store — we still verify the real server.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rustls::{ClientConfig, RootCertStore, ServerConfig};

use crate::cert::CertAuthority;
use crate::error::ProxyError;

/// Install the ring crypto provider as the process default. Idempotent; safe to
/// call from every entry point (manager startup, tests).
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Produces (and caches) rustls configs for intercepted connections.
pub struct TlsFactory {
    ca: Arc<CertAuthority>,
    upstream: Arc<ClientConfig>,
    server_cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
}

impl TlsFactory {
    /// Build a factory that validates upstreams against the OS trust store.
    pub fn new(ca: Arc<CertAuthority>) -> Result<Self, ProxyError> {
        Self::with_upstream(ca, native_root_store()?)
    }

    /// Build a factory validating upstreams against a caller-supplied root store
    /// (used by tests to trust a local upstream).
    pub fn with_upstream(ca: Arc<CertAuthority>, roots: RootCertStore) -> Result<Self, ProxyError> {
        let mut upstream = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        upstream.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            ca,
            upstream: Arc::new(upstream),
            server_cache: Mutex::new(HashMap::new()),
        })
    }

    /// rustls `ServerConfig` presenting a forged leaf for `host` (cached).
    pub fn server_config_for(&self, host: &str) -> Result<Arc<ServerConfig>, ProxyError> {
        if let Some(cfg) = self.server_cache.lock().get(host) {
            return Ok(cfg.clone());
        }
        let leaf = self.ca.leaf_for(host)?;
        let mut cfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![leaf.cert_der.clone()], leaf.key_der.clone_key())
            .map_err(|e| ProxyError::Tls(format!("server config for {host}: {e}")))?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let cfg = Arc::new(cfg);
        self.server_cache
            .lock()
            .insert(host.to_string(), cfg.clone());
        Ok(cfg)
    }

    /// The shared upstream client config.
    pub fn upstream(&self) -> Arc<ClientConfig> {
        self.upstream.clone()
    }
}

/// Load the OS trust store into a rustls `RootCertStore`.
fn native_root_store() -> Result<RootCertStore, ProxyError> {
    let mut roots = RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for cert in loaded.certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err(ProxyError::Tls(
            "no OS root certificates available for upstream validation".to_string(),
        ));
    }
    Ok(roots)
}
