# Endpoint Path Analysis - Unified API Surface

## Summary
**Architecture: UNIFIED ROOT PATH** - MCP and OpenAI-compatible endpoints coexist under the same root path `/`.

All endpoints use consistent authentication (Bearer tokens) and share the same base URL for a seamless developer experience.

## Complete Endpoint Inventory

### LLM/OpenAI-Compatible API Endpoints

#### System/Documentation
- `GET /` - Root handler with API information
- `GET /health` - Health check
- `GET /openapi.json` - OpenAPI 3.1 specification (JSON)
- `GET /openapi.yaml` - OpenAPI 3.1 specification (YAML)

#### Chat & Completions (with and without /v1 prefix)
- `POST /v1/chat/completions` | `/chat/completions` - Chat completions
- `POST /v1/completions` | `/completions` - Text completions
- `POST /v1/embeddings` | `/embeddings` - Embeddings

#### Models
- `GET /v1/models` | `/models` - List all models
- `GET /v1/models/:id` | `/models/:id` - Get specific model
- `GET /v1/models/:provider/:model/pricing` | `/models/:provider/:model/pricing` - Get pricing

#### Generation
- `GET /v1/generation` | `/generation` - Get generation status

### OAuth Endpoints
- `POST /oauth/token` - OAuth 2.0 token endpoint (client credentials flow)

### MCP (Model Context Protocol) Endpoints

**Note**: MCP endpoints share the same base URL as OpenAI endpoints for a unified API surface.

#### Unified Gateway
- `POST /` - Unified MCP gateway (aggregates all MCP servers, namespaced tools/resources)

#### Individual Servers
- `POST /mcp/:server_id` - Individual MCP server proxy
- `POST /mcp/:server_id/stream` - MCP streaming endpoint (SSE)

## Conflict Analysis

### Path Prefixes by Component

| Component | Path Prefixes | Methods |
|-----------|---------------|---------|
| **LLM API** | `/health`, `/openapi.*`, `/v1/*`, `/chat/*`, `/completions`, `/embeddings`, `/models/*`, `/generation` | GET, POST |
| **OAuth** | `/oauth/*` | POST |
| **MCP** | `/` (POST only), `/mcp/*` | POST |
| **System** | `/` (GET only) | GET |

### Key Design Decisions

#### 1. Root Path Separation by HTTP Method ✅
- **GET /**: System information handler (OpenAI & MCP endpoint documentation)
- **POST /**: MCP unified gateway (JSON-RPC requests)
- **Status**: No conflict - different HTTP methods serve different purposes

#### 2. Path Namespacing ✅
- **MCP Unified Gateway**: `POST /` (root path, method-separated from GET)
- **MCP Individual Servers**: `/mcp/:server_id` (clear namespace for individual proxy)
- **OAuth**: `/oauth/token` (clear authentication namespace)
- **OpenAI API**: `/v1/*` and unprefixed variants (`/chat/*`, `/models`, etc.)
- **Status**: All paths are mutually exclusive

#### 3. Removed Endpoints ✅
- **Removed**: `GET /mcp/health` (redundant with `GET /health`)
- **Removed**: `POST /mcp/:client_id/:server_id` (deprecated wildcard route with client_id)
- **Benefit**: Simpler API surface, no ambiguous routes

#### 4. Unified Authentication ✅
- All endpoints use Bearer token authentication
- MCP routes use OAuth client auth middleware
- OpenAI routes use API key or OAuth auth layer
- **Status**: Consistent authentication model across all endpoints

### HTTP Method Matrix

| Path | GET | POST | Notes |
|------|-----|------|-------|
| `/` | ✅ System | ✅ MCP | GET=API docs, POST=MCP gateway |
| `/health` | ✅ System | - | Health check |
| `/openapi.json` | ✅ System | - | OpenAPI spec (JSON) |
| `/openapi.yaml` | ✅ System | - | OpenAPI spec (YAML) |
| `/v1/chat/completions` | - | ✅ OpenAI | Chat API |
| `/chat/completions` | - | ✅ OpenAI | No /v1 prefix |
| `/v1/completions` | - | ✅ OpenAI | Completions API |
| `/completions` | - | ✅ OpenAI | No /v1 prefix |
| `/v1/embeddings` | - | ✅ OpenAI | Embeddings API |
| `/embeddings` | - | ✅ OpenAI | No /v1 prefix |
| `/v1/models` | ✅ OpenAI | - | List models |
| `/models` | ✅ OpenAI | - | No /v1 prefix |
| `/v1/models/:id` | ✅ OpenAI | - | Get model |
| `/models/:id` | ✅ OpenAI | - | No /v1 prefix |
| `/v1/models/:provider/:model/pricing` | ✅ OpenAI | - | Model pricing |
| `/models/:provider/:model/pricing` | ✅ OpenAI | - | No /v1 prefix |
| `/v1/generation` | ✅ OpenAI | - | Generation status |
| `/generation` | ✅ OpenAI | - | No /v1 prefix |
| `/oauth/token` | - | ✅ OAuth | Client credentials |
| `/mcp/:server_id` | - | ✅ MCP | Individual server proxy |
| `/mcp/:server_id/stream` | - | ✅ MCP | SSE streaming |

## Authentication & Authorization

### LLM API Routes
- **Middleware**: `AuthLayer` (API key or OAuth bearer token)
- **Header**: `Authorization: Bearer <api-key>` or `Authorization: Bearer <oauth-token>`

### OAuth Routes
- **Middleware**: None (this IS the auth endpoint)
- **Auth**: Client credentials in body or Basic Auth header

### MCP Routes
- **Middleware**: `client_auth_middleware` (OAuth bearer token)
- **Header**: `Authorization: Bearer <oauth-token>`
- **Validation**: Checks `allowed_mcp_servers` for authorization

## Router Build Order (from `build_app()`)

```rust
1. Base router with LLM routes (/health, /, /openapi.*, /v1/*, etc.)
2. Apply AuthLayer to base router
3. Merge OAuth routes (no auth - these ARE the auth endpoints)
4. Merge MCP routes (separate auth middleware - OAuth only)
5. Apply logging middleware
6. Apply CORS layer
```

## Conclusion

### ✅ Unified API Surface Implemented

LocalRouter AI now provides a single, unified API surface for both OpenAI-compatible and MCP endpoints:

1. **Single Base URL**: All endpoints share `http://localhost:3625` (or configured port)
2. **Method-Based Routing**: `POST /` serves MCP gateway, `GET /` serves API documentation
3. **Clean Paths**: Unified gateway at `/`, individual servers at `/mcp/:server_id`
4. **Consistent Auth**: All endpoints use Bearer token authentication
5. **No Conflicts**: HTTP method separation and namespacing ensure no path collisions

### Benefits of Unified Architecture

1. **Developer Experience**: One URL to remember for all API interactions
2. **Simpler Configuration**: No need to track separate MCP and LLM endpoints
3. **Clean API Surface**: Fewer paths, clearer purpose for each endpoint
4. **Future-Proof**: Easy to add new features under existing namespaces

### Implementation Changes

**Completed:**
1. ✅ Removed redundant `/mcp/health` endpoint
2. ✅ Moved unified gateway: `/mcp` → `/` (POST method only)
3. ✅ Simplified individual servers: `/mcp/:client_id/:server_id` → `/mcp/:server_id`
4. ✅ Updated OpenAPI documentation paths
5. ✅ Backend generates `proxy_url` and `gateway_url` in `McpServerInfo`
6. ✅ Consistent Bearer token authentication across all MCP endpoints

**Recommended Next Steps:**
1. Update UI to show unified endpoint messaging
2. Test all MCP and OpenAI endpoints with new paths
3. Update any external documentation or client examples

---

**Architecture Date**: 2026-01-20
**Status**: ✅ Unified root path implementation complete
