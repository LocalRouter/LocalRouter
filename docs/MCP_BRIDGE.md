# MCP Bridge Mode

LocalRouter can run in **bridge mode**, acting as a lightweight STDIO ↔ HTTP proxy that allows external MCP clients (Claude Desktop, Cursor, VS Code, etc.) to connect to LocalRouter's unified MCP gateway.

## Overview

### Architecture

```
External Client (e.g., Claude Desktop)
    ↓ STDIO (via command invocation)
LocalRouter STDIO Bridge Process (--mcp-bridge)
    ↓ HTTP Client (localhost:3625/mcp)
    Authorization: Bearer <client_secret>
    ↓
LocalRouter GUI Instance (ALREADY RUNNING)
    ↓ HTTP Server (Axum on :3625)
    ↓ MCP Gateway (aggregates multiple MCP servers)
    ↓
Multiple MCP Servers (filesystem, web, github, etc.)
```

### How It Works

1. **GUI Mode** (default): Full desktop application with HTTP server, managers, and Tauri window
2. **Bridge Mode** (`--mcp-bridge`): Lightweight STDIO proxy (no GUI, no managers, just HTTP client)

The bridge reads JSON-RPC requests from stdin, forwards them to the running LocalRouter HTTP server on `localhost:3625`, and writes responses back to stdout. This allows any MCP client that supports STDIO transport to access LocalRouter's unified MCP gateway.

## Setup

### Step 1: Configure Client in LocalRouter

Edit your LocalRouter configuration file (`~/.localrouter/config.yaml`):

```yaml
clients:
  - id: claude_desktop
    name: Claude Desktop
    enabled: true
    allowed_mcp_servers:
      - filesystem
      - web
      - github
    mcp_deferred_loading: true  # Optional: enable deferred loading
```

### Step 2: Start LocalRouter GUI

The bridge requires the LocalRouter GUI to be running (to provide the HTTP server). Start it normally:

```bash
# macOS
open /Applications/LocalRouter\ AI.app

# Or via command line
localrouter
```

### Step 3: Configure External MCP Client

Configure your MCP client to invoke LocalRouter in bridge mode. See client-specific instructions below.

## Client Authentication

The bridge supports three authentication methods:

### 1. Environment Variable (Recommended)

Set `LOCALROUTER_CLIENT_SECRET` to your client secret:

```bash
export LOCALROUTER_CLIENT_SECRET=lr_your_secret_here
```

Then invoke bridge mode:

```bash
localrouter --mcp-bridge --client-id claude_desktop
```

### 2. Auto-Detection (Simplest)

If you've run the LocalRouter GUI at least once, client secrets are stored in your OS keychain. Simply invoke:

```bash
localrouter --mcp-bridge --client-id claude_desktop
```

The bridge will automatically load the secret from the keychain.

### 3. First Enabled Client

If you don't specify `--client-id`, the bridge auto-detects the first enabled client with MCP servers:

```bash
localrouter --mcp-bridge
```

## Usage

### Command Line Options

```bash
# Auto-detect first enabled client with MCP servers
localrouter --mcp-bridge

# Specify client ID explicitly
localrouter --mcp-bridge --client-id claude_desktop

# With client secret via environment variable
LOCALROUTER_CLIENT_SECRET=lr_secret localrouter --mcp-bridge --client-id claude_desktop
```

### Environment Variables

- `LOCALROUTER_CLIENT_SECRET`: Client secret for authentication (overrides keychain)
- `LOCALROUTER_KEYCHAIN`: Force keychain type (`file` or `system`)
- `RUST_LOG`: Logging level (e.g., `localrouter=debug`)

## Integration Examples

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "localrouter": {
      "command": "/Applications/LocalRouter.app/Contents/MacOS/localrouter",
      "args": ["--mcp-bridge", "--client-id", "claude_desktop"],
      "env": {
        "LOCALROUTER_CLIENT_SECRET": "lr_your_secret_here"
      }
    }
  }
}
```

**Note**: Replace the `command` path with the actual path to your LocalRouter binary:

- macOS app: `/Applications/LocalRouter.app/Contents/MacOS/localrouter`
- macOS Homebrew: `/usr/local/bin/localrouter`
- Linux: `/usr/bin/localrouter` or `~/.local/bin/localrouter`
- Windows: `C:\Program Files\LocalRouter\localrouter.exe`

Get your client secret from the LocalRouter GUI (Clients tab → Create/View Client).

### Cursor

Edit `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "localrouter": {
      "command": "/path/to/localrouter",
      "args": ["--mcp-bridge", "--client-id", "cursor"],
      "env": {
        "LOCALROUTER_CLIENT_SECRET": "lr_your_secret_here"
      }
    }
  }
}
```

### VS Code (with MCP Extension)

Edit `.vscode/mcp.json` in your workspace:

```json
{
  "servers": {
    "localrouter": {
      "command": "/path/to/localrouter",
      "args": ["--mcp-bridge", "--client-id", "vscode"],
      "env": {
        "LOCALROUTER_CLIENT_SECRET": "lr_your_secret_here"
      }
    }
  }
}
```

## Deferred Loading

Enable deferred loading in your client configuration to reduce token consumption:

```yaml
clients:
  - id: claude_desktop
    name: Claude Desktop
    enabled: true
    allowed_mcp_servers:
      - filesystem
      - web
      - github
    mcp_deferred_loading: true  # Enable deferred loading
```

When enabled:

- Only a `search` tool is initially visible
- Tools are activated on-demand through search queries
- Dramatically reduces token consumption for large MCP server catalogs
- Example: Search for "file" to activate filesystem tools

## Troubleshooting

### Bridge Won't Start

**Error**: `Could not connect to LocalRouter at localhost:3625. Is the app running?`

**Solution**: Start the LocalRouter GUI first. The bridge requires the HTTP server to be running.

---

**Error**: `Client 'xyz' not found in config.yaml`

**Solution**: Add the client to your `config.yaml` file (see Setup section).

---

**Error**: `Client 'xyz' is disabled`

**Solution**: Set `enabled: true` for the client in `config.yaml`.

---

**Error**: `Client secret not found`

**Solution**:

1. Run LocalRouter GUI at least once to create credentials, OR
2. Set `LOCALROUTER_CLIENT_SECRET` environment variable, OR
3. Generate a client secret in the GUI (Clients tab → Create Client)

### Connection Errors During Use

**Error**: `Invalid client credentials`

**Solution**: Your client secret is incorrect. Regenerate it in the LocalRouter GUI.

---

**Error**: `Client 'xyz' is not allowed to access MCP servers`

**Solution**: Add MCP servers to `allowed_mcp_servers` in the client configuration.

---

**Error**: `HTTP 404 error`

**Solution**: Update to the latest version of LocalRouter. The MCP endpoint may be missing.

### No Tools Visible

**Issue**: External client shows no tools from LocalRouter

**Solutions**:

1. Check that `allowed_mcp_servers` includes the servers you want to use
2. Verify MCP servers are enabled in LocalRouter GUI (MCP tab)
3. Check bridge logs (stderr) for error messages
4. Restart both LocalRouter GUI and the external client

### Performance Issues

**Issue**: Bridge is slow or unresponsive

**Solutions**:

1. Enable `mcp_deferred_loading: true` to reduce token overhead
2. Limit `allowed_mcp_servers` to only the servers you need
3. Check LocalRouter GUI logs for errors
4. Ensure LocalRouter GUI has sufficient system resources

## Logging

Bridge mode logs to **stderr only** (stdout is reserved for JSON-RPC responses).

### View Logs

When running from terminal:

```bash
localrouter --mcp-bridge 2>&1 | tee bridge.log
```

When configured in external client, check the client's log files:

- Claude Desktop: `~/Library/Logs/Claude/mcp*.log` (macOS)
- Cursor: `~/.cursor/logs/`
- VS Code: Check Output panel → MCP extension

### Adjust Log Level

```bash
RUST_LOG=localrouter=debug localrouter --mcp-bridge
```

Log levels: `error`, `warn`, `info`, `debug`, `trace`

## Security Considerations

### Client Secrets

- **Never commit** client secrets to version control
- Store secrets in environment variables or OS keychain
- Rotate secrets regularly in the LocalRouter GUI
- Each external client should have its own client ID and secret

### Network Security

- Bridge connects to `localhost:3625` only (no external network access)
- HTTP server is localhost-only by default
- Use firewall rules if you need to restrict access further

### Access Control

- `allowed_mcp_servers` controls which MCP servers the client can access
- Each client can have different permissions
- Disabled clients cannot authenticate (even with valid secrets)

## Advanced Usage

### Multiple Clients

Run multiple bridge instances for different clients:

```bash
# Terminal 1: Claude Desktop
LOCALROUTER_CLIENT_SECRET=lr_secret1 localrouter --mcp-bridge --client-id claude_desktop

# Terminal 2: Cursor
LOCALROUTER_CLIENT_SECRET=lr_secret2 localrouter --mcp-bridge --client-id cursor
```

Each bridge instance is isolated and uses its own client configuration.

### Custom Server URL

By default, the bridge connects to `http://localhost:3625/mcp`. To use a different URL, modify the source code in `src-tauri/src/mcp/bridge/stdio_bridge.rs`:

```rust
server_url: "http://custom-host:custom-port/mcp".to_string(),
```

### File-Based Keychain (Development)

For development, use file-based keychain to avoid macOS keychain prompts:

```bash
LOCALROUTER_KEYCHAIN=file localrouter --mcp-bridge
```

**WARNING**: File-based keychain stores secrets in **plain text**. Only use for development with test credentials.

## Comparison with HTTP MCP Gateway

| Feature                | Bridge Mode (STDIO)      | HTTP Gateway              |
| ---------------------- | ------------------------ | ------------------------- |
| **Transport**          | STDIO (stdin/stdout)     | HTTP POST /mcp            |
| **Client Support**     | Claude Desktop, Cursor, VS Code | Any HTTP client           |
| **Authentication**     | Client secret (Bearer)   | Client secret (Bearer)    |
| **Process Model**      | Separate process per client | Single HTTP server        |
| **Resource Usage**     | ~10MB per bridge         | Shared HTTP server        |
| **Startup Time**       | < 100ms                  | N/A (always running)      |
| **Use Case**           | External MCP clients     | OpenAI-compatible clients |

## Performance

- **Startup time**: < 100ms (minimal initialization)
- **Request overhead**: < 10ms (STDIO parsing + HTTP POST + localhost latency)
- **Memory usage**: < 10MB per bridge instance (very lightweight)
- **Latency**: Dominated by MCP server calls (10-100ms), HTTP overhead negligible

## Limitations

1. **Requires Running GUI**: Bridge mode requires the LocalRouter GUI to be running (for the HTTP server)
2. **No Standalone Mode**: Cannot run bridge without GUI (for now)
3. **Localhost Only**: Bridge connects to localhost:3625 only (no remote servers)
4. **No WebSocket**: Currently only supports STDIO transport (no WebSocket support)

## Roadmap

Future enhancements planned:

- [ ] Standalone bridge mode (no GUI required)
- [ ] WebSocket transport support
- [ ] Remote server support (connect to LocalRouter on another machine)
- [ ] Multi-client bridge (single process for multiple clients)
- [ ] Bridge metrics and monitoring
- [ ] Hot config reload (no restart needed)

## FAQ

### Q: Do I need to run the LocalRouter GUI to use bridge mode?

**A**: Yes, the bridge requires the LocalRouter GUI to be running. The bridge is a lightweight proxy that forwards requests to the HTTP server running in the GUI.

### Q: Can I use bridge mode without installing the GUI?

**A**: Not currently. Future versions may support standalone bridge mode.

### Q: How do I get a client secret?

**A**: Client secrets are generated automatically when you create a client in the LocalRouter GUI (Clients tab → Create Client). You can also view existing secrets by clicking on a client.

### Q: Can I use the same client for both HTTP and STDIO?

**A**: Yes! The same client configuration works for both HTTP requests (OpenAI-compatible API) and STDIO bridge mode (MCP).

### Q: What's the difference between a client and an MCP server?

**A**:

- **Client**: External application that *uses* LocalRouter (e.g., Claude Desktop, Cursor, your custom app)
- **MCP Server**: Tool provider that LocalRouter *connects to* (e.g., filesystem, web, GitHub)

Clients consume tools from MCP servers via LocalRouter's unified gateway.

### Q: How do I enable deferred loading?

**A**: Set `mcp_deferred_loading: true` in your client configuration. This reduces token consumption by only loading tools on-demand through search queries.

### Q: Can I use bridge mode with custom MCP servers?

**A**: Yes! Configure your custom MCP servers in LocalRouter (MCP tab), then add them to `allowed_mcp_servers` for your client.

### Q: Does bridge mode support all MCP features?

**A**: Yes, bridge mode supports all MCP features including:

- Tools (list, call)
- Resources (list, read, subscribe)
- Prompts (list, get)
- Notifications
- Deferred loading (search tool)

### Q: How do I debug bridge mode?

**A**:

1. Check stderr output (all logs go there)
2. Set `RUST_LOG=localrouter=debug` for verbose logs
3. Check LocalRouter GUI logs (Help → View Logs)
4. Check external client logs (see Logging section)

## Support

For issues, questions, or feature requests:

- GitHub Issues: https://github.com/yourusername/localrouterai/issues
- Documentation: https://localrouter.ai/docs
- Discord: https://discord.gg/localrouter

## License

LocalRouter is licensed under the AGPL-3.0-or-later license. See LICENSE file for details.
