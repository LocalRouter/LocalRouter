//! Resolving a proxied connection's client identity from its proxy credentials.
//!
//! The proxy authenticates each `CONNECT` via HTTP Basic `Proxy-Authorization`,
//! carrying `client_id:client_secret`. Verifying the secret and looking up the
//! client's proxy mode lives in the app (lr-clients / config), so the transport
//! depends only on this trait — the wiring provides the concrete implementation.

use crate::interceptor::ClientCtx;

/// Resolves proxy credentials into an authenticated client context.
pub trait ClientResolver: Send + Sync {
    /// Return a [`ClientCtx`] if `client_id` + `secret` authenticate to a known
    /// client, or `None` to reject the tunnel with `407`.
    fn resolve(&self, client_id: &str, secret: &str) -> Option<ClientCtx>;
}

/// A fixed-credential resolver, primarily for tests.
pub struct StaticResolver {
    pub client_id: String,
    pub secret: String,
    pub proxy_enabled: bool,
}

impl ClientResolver for StaticResolver {
    fn resolve(&self, client_id: &str, secret: &str) -> Option<ClientCtx> {
        if client_id == self.client_id && secret == self.secret {
            Some(ClientCtx {
                client_id: self.client_id.clone(),
                proxy_enabled: self.proxy_enabled,
            })
        } else {
            None
        }
    }
}
