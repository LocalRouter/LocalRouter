# STDIO Bridge Mode for LocalRouter AI - Implementation Plan

## Overview

Add STDIO bridge mode to LocalRouter AI, allowing external MCP clients (Claude Desktop, Cursor, VS Code, etc.) to connect to LocalRouter's unified MCP gateway via standard input/output. This transforms LocalRouter from a purely HTTP-based gateway to a STDIO-compatible MCP server.

## Current Architecture

### How It Works Now
```
External Client (e.g., Claude Desktop)
    ↓ HTTP POST /mcp
    Authorization: Bearer <client_secret>
    ↓
LocalRouter HTTP Server (Axum on :3625)
    ↓
MCP Gateway (aggregates multiple MCP servers)
    ↓
Multiple MCP Servers (filesystem, web, github, etc.)
```

### Desired Architecture
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

**Key insight:** The bridge is a **STDIO ↔ HTTP proxy**. It reads JSON-RPC from stdin, forwards to the running LocalRouter HTTP server, and writes responses back to stdout.

## Key Design Decisions

### 1. Binary Invocation
**Runtime flag detection** - Use `--mcp-bridge` CLI argument to switch modes

**Invocation examples:**
```bash
# Auto-detect client (first enabled client with MCP servers)
localrouter --mcp-bridge

# Specify client ID explicitly
localrouter --mcp-bridge --client-id claude_desktop

# With optional secret verification via environment variable
LOCALROUTER_CLIENT_SECRET=lr_... localrouter --mcp-bridge --client-id claude_desktop
```

### 2. Authentication Strategy
**Pass-through authentication** to running LocalRouter HTTP server

**Flow:**
1. Bridge starts with `--client-id <id>` (or auto-detects from config)
2. Load client secret from keychain or env var `LOCALROUTER_CLIENT_SECRET`
3. For each JSON-RPC request from stdin:
   - Add HTTP `Authorization: Bearer <client_secret>` header
   - POST to `http://localhost:3625/mcp`
   - Return HTTP response body as JSON-RPC to stdout

**Simple pass-through** - No session management in bridge:
- All session management handled by existing HTTP server
- Bridge is stateless proxy
- Multiple bridges can use same or different clients

### 3. Client Configuration
**Reuse existing Client system** - No new config structures needed

**Example config.yaml:**
```yaml
clients:
  - id: claude_desktop
    name: Claude Desktop
    enabled: true
    allowed_mcp_servers:
      - filesystem
      - web
      - github
    mcp_deferred_loading: true
```

### 4. Process Lifecycle
**Lightweight proxy process** - Requires running LocalRouter instance

- **GUI mode** (default): Starts HTTP server + Tauri window + MCP servers
- **Bridge mode**: Lightweight STDIO ↔ HTTP proxy only
  - No managers initialization
  - No MCP servers started
  - No GUI
  - Just reads stdin, POSTs to localhost:3625, writes stdout
- Can run multiple bridge instances for different clients
- Must have LocalRouter GUI running (for HTTP server)

### 5. Error Handling
**stdout = JSON-RPC only, stderr = logs**

- **Initialization errors** (exit immediately with code 1):
  - Config not found
  - Client not found/disabled
  - No MCP servers allowed

- **Runtime errors** (JSON-RPC error responses):
  - Gateway errors
  - Server unavailable
  - Invalid requests

## Implementation Phases

### Phase 1: CLI Argument Parsing

**New files:**
- `src-tauri/src/cli.rs` - CLI argument parsing with clap

**Modified files:**
- `src-tauri/Cargo.toml` - Add clap dependency
- `src-tauri/src/lib.rs` - Export cli module
- `src-tauri/src/main.rs` - Add CLI parsing before Tauri init

**CLI structure:**
```rust
#[derive(Parser, Debug)]
pub struct Cli {
    /// Run in MCP bridge mode (STDIO)
    #[arg(long)]
    pub mcp_bridge: bool,

    /// Client ID for bridge mode
    #[arg(long, requires = "mcp_bridge")]
    pub client_id: Option<String>,
}
```

**Acceptance:**
- [ ] `--help` shows options
- [ ] `--mcp-bridge` flag detected
- [ ] `--client-id` requires `--mcp-bridge`

### Phase 2: Bridge Core Module

**New files:**
- `src-tauri/src/mcp/bridge/mod.rs` - Public API
- `src-tauri/src/mcp/bridge/stdio_bridge.rs` - STDIO ↔ HTTP proxy

**Module structure:**
```
src-tauri/src/mcp/bridge/
├── mod.rs                  # Public API
└── stdio_bridge.rs         # STDIO I/O + HTTP client
```

**Key types:**
```rust
pub struct StdioBridge {
    /// Client secret for Authorization header
    client_secret: String,

    /// LocalRouter HTTP endpoint
    server_url: String,  // Default: "http://localhost:3625/mcp"

    /// HTTP client
    http_client: reqwest::Client,

    /// STDIO handles
    stdin: BufReader<tokio::io::Stdin>,
    stdout: tokio::io::Stdout,
}

impl StdioBridge {
    pub async fn new(
        client_id: Option<String>,
        config_path: Option<PathBuf>,
    ) -> AppResult<Self> {
        // Load config to get client secret
        // Or use LOCALROUTER_CLIENT_SECRET env var
        // Create HTTP client
    }

    pub async fn run(mut self) -> AppResult<()> {
        // Loop: read stdin → HTTP POST → write stdout
    }

    async fn handle_request(&mut self, request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        // POST to http://localhost:3625/mcp
        // With Authorization: Bearer header
    }
}
```

**Client secret resolution:**
```rust
async fn resolve_client_secret(
    client_id: Option<String>,
    config: &AppConfig,
) -> AppResult<(String, String)> {
    // 1. Try LOCALROUTER_CLIENT_SECRET env var
    if let Ok(secret) = std::env::var("LOCALROUTER_CLIENT_SECRET") {
        return Ok((client_id.unwrap_or("env".to_string()), secret));
    }

    // 2. Load from config + keychain
    let client = find_client(client_id, config)?;
    let keychain = CachedKeychain::auto()?;
    let secret = keychain.get("LocalRouter-Clients", &client.id)?
        .ok_or_else(|| AppError::Config("Client secret not found".into()))?;

    Ok((client.id, secret))
}
```

**Acceptance:**
- [ ] Reads JSON-RPC from stdin line-by-line
- [ ] POSTs to localhost:3625/mcp with bearer token
- [ ] Writes HTTP response to stdout as JSON-RPC
- [ ] All logging goes to stderr
- [ ] Handles malformed JSON gracefully
- [ ] Handles connection errors (server not running)

### Phase 3: Main Entry Point Modification

**Modified files:**
- `src-tauri/src/main.rs` - Add mode branching

**Implementation:**
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::parse_args();
    init_logging(); // Always to stderr

    if cli.mcp_bridge {
        run_bridge_mode(cli.client_id).await
    } else {
        run_gui_mode().await
    }
}

async fn run_bridge_mode(client_id: Option<String>) -> anyhow::Result<()> {
    // Lightweight STDIO ↔ HTTP proxy
    // No managers, no MCP servers, no GUI

    eprintln!("Starting LocalRouter MCP Bridge");
    eprintln!("Connecting to LocalRouter server at http://localhost:3625");

    // Create and run bridge (loads config for client secret only)
    let bridge = mcp::bridge::StdioBridge::new(
        client_id,
        None, // Use default config path
    ).await?;

    eprintln!("Bridge ready, forwarding JSON-RPC requests...");
    bridge.run().await?;

    Ok(())
}
```

**Acceptance:**
- [ ] `--mcp-bridge` skips Tauri initialization
- [ ] Bridge mode loads minimal config (client secret only)
- [ ] Both modes can coexist (different processes)
- [ ] Bridge exits gracefully if server not running

### Phase 4: HTTP Client Integration

**Modified files:**
- `src-tauri/src/mcp/bridge/stdio_bridge.rs` - HTTP client implementation

**Implementation:**
```rust
async fn handle_request(&mut self, request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
    // Serialize JSON-RPC request
    let body = serde_json::to_string(&request)?;

    // POST to LocalRouter HTTP server
    let response = self.http_client
        .post(&self.server_url)
        .header("Authorization", format!("Bearer {}", self.client_secret))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| AppError::Mcp(format!("HTTP request failed: {}", e)))?;

    // Check status
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await?;
        return Err(AppError::Mcp(format!(
            "HTTP {} error: {}", status, text
        )));
    }

    // Parse JSON-RPC response
    let json_rpc_response: JsonRpcResponse = response.json().await?;
    Ok(json_rpc_response)
}
```

**Error handling:**
- Connection refused → "LocalRouter not running? Start the app first."
- 401 Unauthorized → "Invalid client credentials"
- 403 Forbidden → "Client not allowed to access MCP servers"
- Other HTTP errors → Pass through error message

**Acceptance:**
- [ ] `initialize` request works
- [ ] `tools/list` returns namespaced tools
- [ ] `tools/call` routes to correct server
- [ ] Connection errors are handled gracefully
- [ ] Helpful error messages for common issues

### Phase 5: Error Handling & Logging

**Modified files:**
- `src-tauri/src/mcp/bridge/stdio_bridge.rs` - Error handling
- `src-tauri/src/main.rs` - Logging configuration

**Logging setup:**
```rust
fn init_logging() {
    // Simple stderr logging (no fancy formatting for bridge mode)
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr) // CRITICAL: stderr only
        )
        .init();
}
```

**Error handling:**

1. **Startup errors** (exit with code 1):
   - Config not found: "Config file not found at ~/.localrouter/config.yaml"
   - Client not found: "Client 'xyz' not found in config"
   - Client disabled: "Client 'xyz' is disabled"
   - Secret not found: "Client secret not found. Set LOCALROUTER_CLIENT_SECRET or run GUI once."

2. **Connection errors** (exit with code 1):
   - Connection refused: "Could not connect to LocalRouter at localhost:3625. Is the app running?"
   - 401 Unauthorized: "Invalid client credentials"
   - 403 Forbidden: "Client not allowed to access MCP servers"

3. **Runtime errors** (return JSON-RPC error):
   - Malformed JSON from stdin
   - HTTP timeout
   - Other gateway errors

**Acceptance:**
- [ ] All logs to stderr only
- [ ] Startup errors have helpful messages
- [ ] Connection errors suggest fixes
- [ ] Runtime errors return JSON-RPC errors

### Phase 6: Documentation

**New files:**
- `docs/MCP_BRIDGE.md` - User guide

**Sections:**
1. Overview - What is bridge mode
2. Setup - Client configuration
3. Usage - Command examples
4. Claude Desktop integration - Example config
5. Cursor integration - Example config
6. Troubleshooting - Common issues

**Claude Desktop config example:**
```json
{
  "mcpServers": {
    "localrouter": {
      "command": "/Applications/LocalRouter AI.app/Contents/MacOS/localrouter-ai",
      "args": ["--mcp-bridge", "--client-id", "claude_desktop"],
      "env": {
        "LOCALROUTER_CLIENT_SECRET": "lr_..."
      }
    }
  }
}
```

### Phase 7: Testing

**New files:**
- `src-tauri/tests/mcp_bridge_tests.rs` - Integration tests

**Test scenarios:**

1. **Client resolution:**
   - Auto-detect first enabled client
   - Explicit client ID
   - Client not found
   - Client disabled
   - Client with empty `allowed_mcp_servers`

2. **STDIO communication:**
   - Read valid JSON-RPC
   - Write JSON-RPC response
   - Handle malformed JSON
   - Handle EOF

3. **Gateway integration:**
   - Initialize request
   - Tools list
   - Tools call
   - Multiple requests (session persistence)

4. **Authentication:**
   - Valid client secret
   - Invalid client secret
   - No secret when optional

5. **Error handling:**
   - Invalid method
   - Server unavailable
   - Timeout

**Manual testing:**
```bash
# Basic test
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | \
  cargo run -- --mcp-bridge --client-id test_client

# Test with Claude Desktop
# Edit ~/.config/Claude/claude_desktop_config.json
# Add localrouter MCP server config
# Test in Claude Desktop UI
```

## Critical Files

**To create:**
- `src-tauri/src/cli.rs` - CLI parsing (~50 lines)
- `src-tauri/src/mcp/bridge/mod.rs` - Public API (~20 lines)
- `src-tauri/src/mcp/bridge/stdio_bridge.rs` - STDIO ↔ HTTP proxy (~150 lines)
- `docs/MCP_BRIDGE.md` - User documentation
- `src-tauri/tests/mcp_bridge_tests.rs` - Integration tests

**To modify:**
- `src-tauri/Cargo.toml` - Add clap + reqwest dependencies
- `src-tauri/src/lib.rs` - Export cli module
- `src-tauri/src/main.rs` - Add mode branching (~50 lines added)

**Total new code:** ~300 lines
**Modified code:** ~80 lines
**Reused code:** Existing HTTP server + gateway (no changes needed!)

## Dependencies

**New:**
- `clap = { version = "4.5", features = ["derive"] }` - CLI parsing
- `reqwest = { version = "0.11", features = ["json"] }` - HTTP client (likely already a dependency)

**Existing (reused):**
- tokio - Async runtime, STDIO
- serde_json - JSON-RPC
- tracing - Logging
- Existing HTTP server + gateway (no changes!)

## Configuration

**No config changes needed** - Reuses existing Client system:

```yaml
clients:
  - id: claude_desktop
    name: Claude Desktop
    enabled: true
    allowed_mcp_servers:  # Controls MCP access
      - filesystem
      - web
    mcp_deferred_loading: true  # Enables search tool
```

## Security Considerations

### Client Secret Handling
Three approaches supported:

1. **Environment variable** (Recommended):
   ```bash
   LOCALROUTER_CLIENT_SECRET=lr_... localrouter --mcp-bridge
   ```

2. **Config file only** (Simpler, less secure):
   ```bash
   localrouter --mcp-bridge --client-id my_client
   # No secret verification
   ```

3. **Keychain verification** (Most secure):
   ```bash
   localrouter --mcp-bridge --client-id my_client
   # Verifies against keychain (may prompt on macOS)
   ```

### Process Isolation
- Each bridge instance = separate process
- Isolated session in gateway
- No cross-client data leakage
- Resource limits via OS

### Access Control
- `allowed_mcp_servers` enforced at gateway level
- Same security model as HTTP gateway
- No way to bypass access restrictions

## Performance Expectations

- **Startup time:** < 100ms (minimal initialization, just HTTP client setup)
- **Request overhead:** < 10ms (STDIO parsing + HTTP POST + localhost latency)
- **Memory usage:** < 10MB per bridge instance (very lightweight)
- **Latency:** Dominated by MCP server calls (10-100ms), HTTP overhead negligible

## User Workflow

### Setup (one-time)

1. Configure client in LocalRouter:
   ```yaml
   # ~/.localrouter/config.yaml
   clients:
     - id: claude_desktop
       name: Claude Desktop
       enabled: true
       allowed_mcp_servers:
         - filesystem
         - web
   ```

2. Configure external client (Claude Desktop):
   ```json
   {
     "mcpServers": {
       "localrouter": {
         "command": "/path/to/localrouter",
         "args": ["--mcp-bridge", "--client-id", "claude_desktop"]
       }
     }
   }
   ```

### Usage (daily)

- External client (Claude Desktop) starts bridge automatically
- Bridge connects to unified MCP gateway
- All configured MCP servers available
- Namespaced tools: `filesystem__read_file`, `web__search`, etc.

## Alternative Approaches Considered

### 1. Separate Binary
**Rejected** - Duplicate code, harder maintenance, distribution complexity

### 2. HTTP Proxy Mode
**Rejected** - Most MCP clients only support STDIO, added network complexity

### 3. Named Pipe Transport
**Rejected** - Not cross-platform, more complex than STDIO

### 4. WebSocket Transport
**Future enhancement** - Not needed for MVP, STDIO covers 99% of use cases

## Success Criteria

### MVP (Must Have)
- [ ] Bridge mode starts without GUI
- [ ] Client resolution works (explicit + auto)
- [ ] JSON-RPC communication via STDIO
- [ ] Gateway integration (initialize, tools/list, tools/call)
- [ ] Namespaced tools/resources work
- [ ] Error handling (startup + runtime)
- [ ] Claude Desktop integration tested
- [ ] Documentation complete
- [ ] Integration tests pass

### Post-MVP (Should Have)
- [ ] Client secret verification via env var
- [ ] Deferred loading support
- [ ] Cursor integration tested
- [ ] VS Code MCP extension tested
- [ ] Comprehensive tests

### Future (Nice to Have)
- [ ] Metrics for bridge mode
- [ ] Hot config reload
- [ ] WebSocket bridge mode
- [ ] Multi-client bridge (single process)

## Rollout Timeline

- **Phase 1-5** (Core implementation): 3-5 days
- **Phase 6** (Documentation): 1 day
- **Phase 7** (Testing): 2 days
- **Total**: 1 week (much simpler than original plan!)

## Risks & Mitigations

### High Risk
1. **STDIO protocol mismatch** - External clients expect different format
   - *Mitigation:* Test with real clients (Claude Desktop, Cursor) early

2. **Process lifecycle issues** - Bridge not cleaned up properly
   - *Mitigation:* Proper signal handling, process cleanup

### Medium Risk
1. **User confusion** - Complex client setup
   - *Mitigation:* Clear documentation, helpful error messages

2. **Configuration complexity** - Multiple config locations
   - *Mitigation:* Single config file, auto-detect when possible

### Low Risk
1. **Performance overhead** - Bridge adds latency
   - *Mitigation:* Minimal async overhead, tested

2. **Compatibility** - Different MCP client expectations
   - *Mitigation:* Follow MCP spec, test multiple clients

## Verification

After implementation, verify:

1. **STDIO Communication:**
   ```bash
   echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | \
     cargo run -- --mcp-bridge
   ```

2. **Client Resolution:**
   ```bash
   cargo run -- --mcp-bridge  # Auto-detect
   cargo run -- --mcp-bridge --client-id test  # Explicit
   ```

3. **Gateway Integration:**
   - Initialize request succeeds
   - Tools list returns namespaced tools
   - Tools call routes correctly
   - Session persists across requests

4. **Claude Desktop:**
   - Configure claude_desktop_config.json
   - Verify tools appear in Claude Desktop
   - Test tool invocation
   - Check logs for errors

5. **Error Handling:**
   - Invalid client ID exits cleanly
   - Malformed JSON returns error
   - Server errors propagate correctly

---

## Next Steps

1. Review and approve this plan
2. Create todo list for implementation phases
3. Start with Phase 1 (CLI parsing)
4. Test incrementally after each phase
5. Document as we build
6. Test with real MCP clients early (Phase 4)
