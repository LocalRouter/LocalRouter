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
            }),
        }
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
