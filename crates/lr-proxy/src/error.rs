//! Error type for the HTTPS inspection proxy.

use thiserror::Error;

/// Errors produced while running or configuring the inspection proxy.
#[derive(Debug, Error)]
pub enum ProxyError {
    /// I/O failure (socket, filesystem).
    #[error("proxy I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Certificate generation / loading failure.
    #[error("proxy certificate error: {0}")]
    Cert(String),

    /// TLS setup or handshake failure.
    #[error("proxy TLS error: {0}")]
    Tls(String),

    /// Malformed CONNECT request or proxy protocol violation.
    #[error("proxy protocol error: {0}")]
    Protocol(String),

    /// Invalid proxy configuration (e.g. an unusable upstream URL).
    #[error("proxy configuration error: {0}")]
    Config(String),
}
