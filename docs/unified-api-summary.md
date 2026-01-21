# Unified API Surface - Quick Reference

## Overview

LocalRouter AI now provides a **single, unified base URL** for both OpenAI-compatible and MCP endpoints at `http://localhost:3625` (or your configured port).

## Endpoint Architecture

### MCP Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/` | POST | **Unified MCP Gateway** - Aggregates all MCP servers, namespaced tools/resources |
| `/mcp/:server_id` | POST | **Individual Server Proxy** - Direct access to a specific MCP server |
| `/mcp/:server_id/stream` | POST | **Streaming Endpoint** - SSE streaming for individual MCP server |

### OpenAI-Compatible Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/` | GET | API documentation (shows all available endpoints) |
| `/health` | GET | Health check |
| `/chat/completions` | POST | Chat completions (with or without `/v1` prefix) |
| `/completions` | POST | Text completions (with or without `/v1` prefix) |
| `/embeddings` | POST | Embeddings (with or without `/v1` prefix) |
| `/models` | GET | List models (with or without `/v1` prefix) |
| `/oauth/token` | POST | OAuth 2.0 client credentials flow |

## Key Features

### 1. Method-Based Routing
- **GET /**: Returns API documentation
- **POST /**: Routes to MCP unified gateway
- No path conflicts due to HTTP method separation

### 2. Unified Authentication
All endpoints use **Bearer token authentication**:
```
Authorization: Bearer <your-client-id>
```

### 3. Backend API Response

The `McpServerInfo` struct now includes:
- **`proxy_url`**: Individual server endpoint (e.g., `http://localhost:3625/mcp/weather-server`)
- **`gateway_url`**: Unified gateway endpoint (always `http://localhost:3625/`)
- **`url`**: Legacy field (deprecated, use `proxy_url`)

### 4. Client Configuration

When configuring MCP clients, use:
- **For unified access** (all servers): `POST http://localhost:3625/`
- **For individual server**: `POST http://localhost:3625/mcp/{server_id}`
- **For streaming**: `POST http://localhost:3625/mcp/{server_id}/stream`

## Migration from Old Paths

| Old Path | New Path | Status |
|----------|----------|--------|
| `GET /mcp/health` | `GET /health` | ✅ Use general health endpoint |
| `POST /mcp` | `POST /` | ✅ Unified gateway at root |
| `POST /mcp/:client_id/:server_id` | `POST /mcp/:server_id` | ✅ Client ID removed (auth-based) |

## Benefits

1. **Single Base URL**: Configure once, use everywhere
2. **Cleaner Paths**: Less nesting, clearer purpose
3. **Better DX**: One endpoint to remember for MCP gateway
4. **Consistent Auth**: Same authentication model across all endpoints
5. **No Conflicts**: Method separation and namespacing prevent collisions

## Example Usage

### MCP Unified Gateway
```bash
curl -X POST http://localhost:3625/ \
  -H "Authorization: Bearer your-client-id" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/list",
    "params": {}
  }'
```

### Individual MCP Server
```bash
curl -X POST http://localhost:3625/mcp/weather-server \
  -H "Authorization: Bearer your-client-id" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "get_weather",
      "arguments": {"city": "San Francisco"}
    }
  }'
```

### OpenAI Chat
```bash
curl -X POST http://localhost:3625/chat/completions \
  -H "Authorization: Bearer your-client-id" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

---

**Last Updated**: 2026-01-20
**Status**: ✅ Production ready
