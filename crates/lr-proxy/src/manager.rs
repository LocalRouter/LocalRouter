//! Proxy lifecycle: build the shared context and run the accept loop.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use rustls::RootCertStore;
use tokio::net::{TcpListener, TcpStream};

use crate::cert::CertAuthority;
use crate::error::ProxyError;
use crate::interceptor::ProxyInterceptor;
use crate::resolver::ClientResolver;
use crate::tls::{ensure_crypto_provider, TlsFactory};
use crate::transport::{handle_connection, ProxyContext};

/// Owns the collaborators needed to serve proxied connections.
pub struct ProxyManager {
    ctx: Arc<ProxyContext>,
}

impl ProxyManager {
    /// Build a manager that validates upstreams against the OS trust store.
    pub fn new(
        ca: Arc<CertAuthority>,
        interceptor: Arc<dyn ProxyInterceptor>,
        resolver: Arc<dyn ClientResolver>,
    ) -> Result<Self, ProxyError> {
        ensure_crypto_provider();
        let tls = Arc::new(TlsFactory::new(ca)?);
        Ok(Self::from_tls(tls, interceptor, resolver))
    }

    /// Build a manager that validates upstreams against `roots` (tests).
    pub fn with_upstream_roots(
        ca: Arc<CertAuthority>,
        interceptor: Arc<dyn ProxyInterceptor>,
        resolver: Arc<dyn ClientResolver>,
        roots: RootCertStore,
    ) -> Result<Self, ProxyError> {
        ensure_crypto_provider();
        let tls = Arc::new(TlsFactory::with_upstream(ca, roots)?);
        Ok(Self::from_tls(tls, interceptor, resolver))
    }

    fn from_tls(
        tls: Arc<TlsFactory>,
        interceptor: Arc<dyn ProxyInterceptor>,
        resolver: Arc<dyn ClientResolver>,
    ) -> Self {
        Self {
            ctx: Arc::new(ProxyContext {
                interceptor,
                resolver,
                tls,
                dedupe_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                usage: None,
                spill: Some(Arc::new(crate::spill::SpillConfig::new(
                    std::env::temp_dir().join("localrouter-proxy-spill"),
                ))),
                length_required: Arc::new(crate::framing::LengthRequiredHosts::default()),
            }),
        }
    }

    fn with_ctx(self, change: impl FnOnce(&mut ProxyContext)) -> Self {
        let mut ctx = (*self.ctx).clone();
        change(&mut ctx);
        Self { ctx: Arc::new(ctx) }
    }

    /// Share the app's live "duplicate request detection" flag with the
    /// transport (defaults to enabled).
    pub fn with_dedupe_flag(self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.with_ctx(|ctx| ctx.dedupe_enabled = flag)
    }

    /// Feed rate-limit headers and usage responses of proxied traffic to the
    /// usage tracker.
    pub fn with_usage_tracker(self, usage: Arc<lr_usage::UsageTracker>) -> Self {
        self.with_ctx(|ctx| ctx.usage = Some(usage))
    }

    /// Where held request bytes spill, encrypted, beyond RAM (`None`: hold
    /// in RAM up to the buffered request limit). Defaults to a directory
    /// under the system temp dir; the app points it at its data directory.
    pub fn with_spill(self, spill: Option<Arc<crate::spill::SpillConfig>>) -> Self {
        self.with_ctx(|ctx| ctx.spill = spill)
    }

    /// Upstreams that get transformed request bodies with a Content-Length
    /// (configured, and learned from refusals).
    pub fn with_length_required(self, hosts: Arc<crate::framing::LengthRequiredHosts>) -> Self {
        self.with_ctx(|ctx| ctx.length_required = hosts)
    }

    /// Bind a TCP listener for the proxy on `host:port`.
    pub async fn bind(host: &str, port: u16) -> Result<TcpListener, ProxyError> {
        Ok(TcpListener::bind((host, port)).await?)
    }

    /// Run the accept loop until `shutdown` resolves. Each connection is handled
    /// on its own task; a slow or stuck tunnel never blocks new connections.
    pub async fn serve(&self, listener: TcpListener, shutdown: impl Future<Output = ()> + Send) {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!("proxy accept loop shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => self.spawn_conn(stream, peer),
                        Err(e) => {
                            tracing::warn!("proxy accept error: {e}");
                        }
                    }
                }
            }
        }
    }

    fn spawn_conn(&self, stream: TcpStream, _peer: SocketAddr) {
        let ctx = self.ctx.clone();
        tokio::spawn(handle_connection(stream, ctx));
    }
}
