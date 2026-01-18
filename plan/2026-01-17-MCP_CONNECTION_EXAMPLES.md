# MCP Server Connection Examples

This document provides examples for connecting to MCP servers using different authentication methods and transports.

## Client Authentication Methods

LocalRouter supports two authentication methods for clients connecting to MCP servers:

### 1. Direct Bearer Token
Use your client secret directly as a Bearer token in the Authorization header.

**Example:**
```bash
curl -X POST http://localhost:8080/mcp/lr-abc123/my-server \
  -H "Authorization: Bearer your-client-secret-here" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "method": "tools/list", "id": 1}'
```

### 2. OAuth 2.0 Client Credentials Flow
Exchange your client credentials for a short-lived access token (1 hour expiry).

**Step 1: Get Access Token**
```bash
curl -X POST http://localhost:8080/oauth/token \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "client_credentials",
    "client_id": "lr-abc123",
    "client_secret": "your-client-secret-here"
  }'
```

**Response:**
```json
{
  "access_token": "eyJhbGc...",
  "token_type": "Bearer",
  "expires_in": 3600
}
```

**Step 2: Use Access Token**
```bash
curl -X POST http://localhost:8080/mcp/lr-abc123/my-server \
  -H "Authorization: Bearer eyJhbGc..." \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "method": "tools/list", "id": 1}'
```

**Or use Basic Authentication:**
```bash
curl -X POST http://localhost:8080/oauth/token \
  -u "lr-abc123:your-client-secret-here" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials"
```

## MCP Server Transport Examples

### STDIO Transport (Subprocess)

Run an MCP server as a subprocess with stdin/stdout communication.

**Example: Local Node.js MCP Server**
```json
{
  "name": "Local MCP Server",
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-everything"],
    "env": {}
  }
}
```

**Example: Python MCP Server with API Key**
```json
{
  "name": "Python MCP Server",
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "python",
    "args": ["-m", "my_mcp_server"],
    "env": {
      "API_KEY": "your-api-key-here",
      "DEBUG": "true"
    }
  }
}
```

### SSE Transport (HTTP/Server-Sent Events)

Connect to a remote MCP server via HTTP with SSE.

**Example: Remote MCP Server with Bearer Token**
```json
{
  "name": "Remote MCP Server",
  "transport": "Sse",
  "config": {
    "type": "sse",
    "url": "https://mcp.example.com/sse",
    "headers": {
      "Authorization": "Bearer your-mcp-server-api-key"
    }
  }
}
```

**Example: Remote MCP Server with Custom Headers**
```json
{
  "name": "Remote MCP with Custom Headers",
  "transport": "Sse",
  "config": {
    "type": "sse",
    "url": "https://api.example.com/mcp",
    "headers": {
      "X-API-Key": "your-api-key",
      "X-Custom-Header": "custom-value"
    }
  }
}
```

### STDIO to SSE Bridge (Using Supergateway)

Use [supergateway](https://github.com/supercorp-ai/supergateway) to bridge STDIO to an SSE endpoint. This is useful when you want to use a STDIO-based tool to connect to a remote SSE MCP server.

**Example: Supergateway Bridge**
```json
{
  "name": "SSE Server via Supergateway",
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "https://your-mcp-server.example.com/sse"
    ],
    "env": {
      "MCP_API_KEY": "your-remote-mcp-server-api-key"
    }
  }
}
```

**How it works:**
1. LocalRouter spawns `npx @uptech/supergateway` as a subprocess
2. Supergateway connects to the remote SSE endpoint at `https://your-mcp-server.example.com/sse`
3. It bridges JSON-RPC messages between stdin/stdout (STDIO) and the remote SSE server
4. The `MCP_API_KEY` environment variable is passed to the subprocess, which supergateway can use to authenticate with the remote server

**Example: Supergateway with Authorization Header**
```json
{
  "name": "Authenticated SSE via Supergateway",
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "--header",
      "Authorization: Bearer ${MCP_API_KEY}",
      "https://api.example.com/mcp/sse"
    ],
    "env": {
      "MCP_API_KEY": "your-api-key-here"
    }
  }
}
```

**Benefits:**
- Use STDIO-based tooling with remote SSE servers
- Centralized API key management (in environment variables)
- Easy to test locally before deploying
- Works with any SSE-based MCP server

## Full Client + MCP Server Example

Here's a complete example showing how to set up a client and connect to an MCP server:

**1. Create a Client in LocalRouter**
- Client ID: `lr-prod-app`
- Client Secret: `sk_live_abc123xyz789` (stored in keychain)
- Allowed MCP Servers: `["weather-server", "data-server"]`

**2. Create MCP Server in LocalRouter**
```json
{
  "id": "weather-server",
  "name": "Weather Data Server",
  "transport": "Stdio",
  "config": {
    "type": "stdio",
    "command": "npx",
    "args": [
      "-y",
      "@uptech/supergateway",
      "--header",
      "Authorization: Bearer ${WEATHER_API_KEY}",
      "https://weather-mcp.example.com/sse"
    ],
    "env": {
      "WEATHER_API_KEY": "weather_api_key_xyz"
    }
  }
}
```

**3. Client Connects to LocalRouter**
```javascript
// Get OAuth token
const tokenResponse = await fetch('http://localhost:8080/oauth/token', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    grant_type: 'client_credentials',
    client_id: 'lr-prod-app',
    client_secret: 'sk_live_abc123xyz789'
  })
});

const { access_token } = await tokenResponse.json();

// Make MCP request
const mcpResponse = await fetch('http://localhost:8080/mcp/lr-prod-app/weather-server', {
  method: 'POST',
  headers: {
    'Authorization': `Bearer ${access_token}`,
    'Content-Type': 'application/json'
  },
  body: JSON.stringify({
    jsonrpc: '2.0',
    method: 'tools/call',
    params: {
      name: 'get_weather',
      arguments: { city: 'San Francisco' }
    },
    id: 1
  })
});

const result = await mcpResponse.json();
console.log('Weather data:', result);
```

## Environment Variable Best Practices

When using environment variables for API keys:

1. **Use descriptive names**: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `MCP_SERVER_KEY`
2. **Never commit secrets**: Use `.env` files (add to `.gitignore`)
3. **Use different keys per environment**: Development vs Production
4. **Rotate keys regularly**: Update in LocalRouter config and keychain
5. **Principle of least privilege**: Only grant the minimum required permissions

## Security Notes

- **Client Secrets**: Stored in OS keychain, never in config files
- **OAuth Tokens**: 1-hour expiry, stored in-memory only
- **Bearer Tokens**: Use OAuth tokens for short-lived access, direct secrets for long-lived
- **MCP Server Credentials**: Stored in keychain, passed via environment variables to subprocesses
- **TLS/HTTPS**: Always use HTTPS for remote MCP server connections
- **Access Control**: Use `allowed_mcp_servers` to restrict client access

## Troubleshooting

### "Invalid client credentials" error
- Verify your `client_id` matches exactly (e.g., `lr-abc123`)
- Check that your `client_secret` is correct
- Ensure the client is enabled in LocalRouter

### "Client not authorized for this server" error
- Add the MCP server ID to the client's `allowed_mcp_servers` list
- Verify the server ID in the URL matches the configured server

### STDIO subprocess fails to start
- Check that the command exists (`npx`, `python`, etc.)
- Verify all required arguments are provided
- Check environment variables are set correctly
- Look at LocalRouter logs for stderr output

### SSE connection timeout
- Verify the URL is accessible
- Check firewall/network settings
- Verify authentication headers are correct
- Test the endpoint with `curl` first
