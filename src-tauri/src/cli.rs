//! CLI argument parsing for LocalRouter AI
//!
//! Supports two modes:
//! - GUI mode (default): Full desktop application
//! - Bridge mode (--mcp-bridge): STDIO ↔ HTTP proxy for MCP clients

use clap::Parser;

/// LocalRouter AI - Intelligent AI model routing with OpenAI-compatible API
#[derive(Parser, Debug)]
#[command(name = "localrouter-ai")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Run in MCP bridge mode (STDIO ↔ HTTP proxy)
    ///
    /// In bridge mode, LocalRouter acts as a lightweight proxy that:
    /// - Reads JSON-RPC requests from stdin
    /// - Forwards them to the running LocalRouter HTTP server
    /// - Writes responses back to stdout
    ///
    /// This allows external MCP clients (Claude Desktop, Cursor, VS Code)
    /// to connect to LocalRouter's unified MCP gateway.
    ///
    /// Example: localrouter-ai --mcp-bridge --client-id claude_desktop
    #[arg(long)]
    pub mcp_bridge: bool,

    /// Client ID for bridge mode
    ///
    /// Specifies which client configuration to use from config.yaml.
    /// If not provided, auto-detects the first enabled client with MCP servers.
    ///
    /// The client's secret is loaded from:
    /// 1. LOCALROUTER_CLIENT_SECRET environment variable (preferred)
    /// 2. Keychain (requires LocalRouter GUI to have been run once)
    ///
    /// Example: --client-id claude_desktop
    #[arg(long, requires = "mcp_bridge")]
    pub client_id: Option<String>,
}

impl Cli {
    /// Parse CLI arguments from environment
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_help() {
        // Verify CLI can be parsed
        let cli = Cli::try_parse_from(["localrouter-ai", "--help"]);
        assert!(cli.is_err()); // --help exits with error (clap behavior)
    }

    #[test]
    fn test_cli_default_mode() {
        let cli = Cli::try_parse_from(["localrouter-ai"]).unwrap();
        assert!(!cli.mcp_bridge);
        assert!(cli.client_id.is_none());
    }

    #[test]
    fn test_cli_bridge_mode() {
        let cli = Cli::try_parse_from(["localrouter-ai", "--mcp-bridge"]).unwrap();
        assert!(cli.mcp_bridge);
        assert!(cli.client_id.is_none());
    }

    #[test]
    fn test_cli_bridge_mode_with_client_id() {
        let cli = Cli::try_parse_from([
            "localrouter-ai",
            "--mcp-bridge",
            "--client-id",
            "test_client",
        ])
        .unwrap();
        assert!(cli.mcp_bridge);
        assert_eq!(cli.client_id, Some("test_client".to_string()));
    }

    #[test]
    fn test_cli_client_id_requires_bridge_mode() {
        // --client-id requires --mcp-bridge
        let cli = Cli::try_parse_from(["localrouter-ai", "--client-id", "test_client"]);
        assert!(cli.is_err());
    }
}
