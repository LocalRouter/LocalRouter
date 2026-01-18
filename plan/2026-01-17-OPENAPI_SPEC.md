# OpenAPI Specification Implementation Plan - LocalRouter AI

## Executive Summary

This plan addresses the feasibility and implementation of creating an OpenAPI specification for LocalRouter AI that serves two purposes:
1. **Code generation** to ensure web server compliance with the spec
2. **Internal documentation** exposed in the Tauri UI

**Current State**: 33,513 lines of Rust implementing a production-ready OpenAI-compatible API with 11 endpoints, streaming support, 7 feature adapters, and NO existing OpenAPI specification.

**User's Original Preference**: Spec-first (write OpenAPI spec, generate Rust code from it)

**Recommended Approach**: **Code-first with utoipa** (generate OpenAPI spec from existing Rust code)

### Why Code-First Instead of Spec-First?

After analyzing your codebase, pure spec-first generation is **not feasible** for these reasons:

1. **33,513 lines of working code would need to be rewritten** - Your existing Axum routes, types, and handlers would be discarded and replaced with generated stubs
2. **Streaming SSE support would be lost** - Code generators don't handle Server-Sent Events streaming well
3. **Custom middleware would break** - Your authentication layer, rate limiting, and OAuth middleware are tightly integrated with Axum
4. **Feature adapters aren't representable** - Your 7 advanced feature adapters (thinking, caching, logprobs, etc.) aren't part of standard OpenAPI
5. **High risk of regressions** - Rewriting a working production codebase is extremely risky

### Proposed Alternative: Hybrid Approach

Use **utoipa** (code-first) to generate the spec, which still satisfies your requirements:

✅ **Spec is source of truth** - Generated at compile-time from code annotations
✅ **Guaranteed compliance** - Compiler enforces spec matches code (impossible to drift)
✅ **Code generation** - TypeScript/Python/etc. clients can be generated from the spec
✅ **Documentation in UI** - Spec served via `/openapi.json` and rendered in Tauri
✅ **Minimal changes** - Add macros to existing code (no rewrites)

**Timeline**: 3-4 weeks vs 3-6 months for pure spec-first rewrite

---

## 1. Technical Approach: utoipa + Scalar

### 1.1 Core Dependencies

**Backend (Rust)**:
```toml
[dependencies]
utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }
utoipa-axum = "0.2"
utoipa-scalar = { version = "0.2", features = ["axum"] }
```

**Frontend (TypeScript)**:
```json
{
  "dependencies": {
    "@scalar/api-reference-react": "^1.0.0"
  }
}
```

### 1.2 How It Works

```rust
// 1. Annotate types with schema derives
#[derive(Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    // ... existing fields
}

// 2. Annotate route handlers with path operations
#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, body = ChatCompletionResponse),
        (status = 400, body = ErrorResponse)
    )
)]
pub async fn chat_completions(/* existing handler */) -> ApiResult<Response> {
    // Handler code unchanged
}

// 3. Compiler generates OpenAPI spec automatically
#[derive(OpenApi)]
#[openapi(
    paths(chat_completions, list_models, /* ... */),
    components(schemas(ChatCompletionRequest, /* ... */))
)]
struct ApiDoc;

// 4. Serve spec at /openapi.json
let spec = ApiDoc::openapi().to_json();
```

**Result**: OpenAPI 3.1 spec is generated at compile-time and served via HTTP endpoint.

---

## 2. Current API Structure

### 2.1 Endpoints (11 total)

| Method | Path | Handler | Status |
|--------|------|---------|--------|
| POST | `/v1/chat/completions` | chat.rs | ✅ Implemented (streaming + non-streaming) |
| POST | `/v1/completions` | completions.rs | ✅ Implemented |
| POST | `/v1/embeddings` | embeddings.rs | ⚠️ Returns 501 Not Implemented |
| GET | `/v1/models` | models.rs | ✅ Implemented |
| GET | `/v1/models/{id}` | models.rs | ✅ Implemented |
| GET | `/v1/models/{provider}/{model}/pricing` | models.rs | ✅ Implemented |
| GET | `/v1/generation?id={id}` | generation.rs | ✅ Implemented |
| POST | `/mcp/{client_id}/{server_id}` | mcp.rs | ✅ Implemented (OAuth auth) |
| GET | `/health` | mod.rs | ✅ Implemented |
| GET | `/mcp/health` | mcp.rs | ✅ Implemented |
| GET | `/` | mod.rs | ✅ Implemented |

### 2.2 Type Definitions

**Location**: `src-tauri/src/server/types.rs` (490 lines)

**Key Types**:
- `ChatCompletionRequest` / `ChatCompletionResponse`
- `ChatCompletionChunk` (for streaming)
- `CompletionRequest` / `CompletionResponse`
- `EmbeddingRequest` / `EmbeddingResponse`
- `ModelsResponse`, `ModelData`, `ModelPricing`
- `GenerationDetailsResponse`
- `ErrorResponse`, `ApiError`
- `ChatMessage`, `MessageContent`, `ContentPart`
- `TokenUsage`, `PromptTokensDetails`, `CompletionTokensDetails`
- `ResponseFormat`, `Tool`, `ToolChoice`

### 2.3 Feature Adapters (7 total)

Extensions documented in `extensions` field:

1. **extended_thinking** (Anthropic) - Deep reasoning mode
2. **reasoning_tokens** (OpenAI) - o1 model reasoning
3. **thinking_level** (Gemini) - Thinking depth control
4. **structured_outputs** - JSON schema validation
5. **prompt_caching** - Cost optimization (50-90% savings)
6. **logprobs** - Token probability extraction
7. **json_mode** - Lightweight JSON validation

---

## 3. Implementation Phases

### Phase 1: Backend Foundation (3-4 days)

**Objective**: Add OpenAPI generation to backend

**Steps**:
1. Add utoipa dependencies to Cargo.toml
2. Annotate all types in `types.rs` with `#[derive(ToSchema)]`
3. Add schema metadata (examples, descriptions, constraints)
4. Test compilation

**Files Modified**:
- `src-tauri/Cargo.toml` - Add dependencies
- `src-tauri/src/server/types.rs` - Add schema derives (15+ types)

**Success Criteria**:
- ✅ `cargo check` passes
- ✅ All types have schema annotations
- ✅ Examples compile

**Example**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Chat Completion Request",
    description = "Request for chat completion API",
    example = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello!"}]
    })
)]
pub struct ChatCompletionRequest {
    #[schema(example = "gpt-4", description = "Model to use")]
    pub model: String,

    #[schema(min_items = 1, description = "Chat messages")]
    pub messages: Vec<ChatMessage>,

    #[schema(minimum = 0.0, maximum = 2.0)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    // ... 20+ more fields
}
```

---

### Phase 2: Route Documentation (3-4 days)

**Objective**: Document all API endpoints

**Steps**:
1. Add `#[utoipa::path]` macro to each route handler
2. Document request/response types
3. Document error responses
4. Handle streaming endpoint special case
5. Document authentication requirements

**Files Modified**:
- `src-tauri/src/server/routes/chat.rs` - Chat completions
- `src-tauri/src/server/routes/completions.rs` - Text completions
- `src-tauri/src/server/routes/embeddings.rs` - Embeddings
- `src-tauri/src/server/routes/models.rs` - Models listing
- `src-tauri/src/server/routes/generation.rs` - Generation tracking
- `src-tauri/src/server/routes/mcp.rs` - MCP proxy

**Success Criteria**:
- ✅ All 11 endpoints documented
- ✅ Streaming response documented
- ✅ Auth requirements specified

**Example**:
```rust
#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    tag = "chat",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, description = "Success", body = ChatCompletionResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 429, description = "Rate limited", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    ),
    params(
        ("stream" = Option<bool>, Query, description = "Enable streaming via SSE")
    )
)]
pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<ChatCompletionRequest>,
) -> ApiResult<Response> {
    // Handler unchanged
}
```

---

### Phase 3: OpenAPI Module (2 days)

**Objective**: Create central spec generation module

**Steps**:
1. Create `src-tauri/src/server/openapi/` directory
2. Create `mod.rs` with OpenAPI builder
3. Create `extensions.rs` to document feature adapters
4. Define security schemes
5. Add server info and metadata

**Files Created**:
- `src-tauri/src/server/openapi/mod.rs` - Main OpenAPI builder
- `src-tauri/src/server/openapi/extensions.rs` - Feature adapter schemas

**Success Criteria**:
- ✅ Spec compiles
- ✅ All endpoints included
- ✅ Security scheme defined

**Implementation**:
```rust
// src-tauri/src/server/openapi/mod.rs

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "LocalRouter AI API",
        version = "0.1.0",
        description = "OpenAI-compatible API gateway with intelligent routing",
        contact(
            name = "LocalRouter AI",
            url = "https://github.com/yourusername/localrouterai"
        )
    ),
    servers(
        (url = "http://localhost:3625", description = "Local server"),
    ),
    paths(
        crate::server::routes::chat::chat_completions,
        crate::server::routes::completions::completions,
        crate::server::routes::embeddings::embeddings,
        crate::server::routes::models::list_models,
        crate::server::routes::models::get_model,
        crate::server::routes::models::get_model_pricing,
        crate::server::routes::generation::get_generation,
    ),
    components(
        schemas(
            crate::server::types::ChatCompletionRequest,
            crate::server::types::ChatCompletionResponse,
            // ... all other types
        ),
    ),
    tags(
        (name = "chat", description = "Chat completion endpoints"),
        (name = "models", description = "Model management"),
        (name = "monitoring", description = "Usage tracking"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::Http::new(
                        utoipa::openapi::security::HttpAuthScheme::Bearer
                    )
                )
            )
        }
    }
}

pub fn get_openapi_json() -> String {
    ApiDoc::openapi().to_json().unwrap()
}

pub fn get_openapi_yaml() -> String {
    ApiDoc::openapi().to_yaml().unwrap()
}
```

---

### Phase 4: Spec Serving Endpoints (1 day)

**Objective**: Serve OpenAPI spec via HTTP

**Steps**:
1. Add GET `/openapi.json` endpoint
2. Add GET `/openapi.yaml` endpoint
3. Test with curl
4. Validate spec with external tools

**Files Modified**:
- `src-tauri/src/server/mod.rs` - Add routes

**Success Criteria**:
- ✅ Spec accessible at `/openapi.json`
- ✅ Spec accessible at `/openapi.yaml`
- ✅ Spec validates with `swagger-cli validate`

**Implementation**:
```rust
// In src-tauri/src/server/mod.rs

fn build_app(state: AppState) -> Router {
    Router::new()
        // NEW: OpenAPI spec endpoints
        .route("/openapi.json", get(serve_openapi_json))
        .route("/openapi.yaml", get(serve_openapi_yaml))

        // Existing routes...
        .route("/v1/chat/completions", post(routes::chat_completions))
        // ...
}

async fn serve_openapi_json() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        openapi::get_openapi_json()
    )
}

async fn serve_openapi_yaml() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/yaml")],
        openapi::get_openapi_yaml()
    )
}
```

**Testing**:
```bash
# Start server
cargo tauri dev

# Get JSON spec
curl http://localhost:3625/openapi.json | jq .

# Get YAML spec
curl http://localhost:3625/openapi.yaml

# Validate spec
npx @apidevtools/swagger-cli validate http://localhost:3625/openapi.json
```

---

### Phase 5: Feature Adapter Documentation (2 days)

**Objective**: Document all 7 feature adapters in OpenAPI spec

**Steps**:
1. Create schema definitions for each adapter's parameters
2. Add examples for each adapter
3. Document in `extensions` field
4. Add usage examples to spec

**Files Created**:
- `src-tauri/src/server/openapi/extensions.rs`

**Success Criteria**:
- ✅ All 7 adapters documented
- ✅ Examples for each adapter
- ✅ Clear descriptions

**Implementation**:
```rust
// src-tauri/src/server/openapi/extensions.rs

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Extended Thinking Parameters",
    description = "Anthropic's extended thinking mode for deeper reasoning",
    example = json!({"enabled": true, "budget_tokens": 5000})
)]
pub struct ExtendedThinkingParams {
    #[schema(default = true)]
    pub enabled: Option<bool>,

    #[schema(minimum = 1000, maximum = 100000)]
    pub budget_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Prompt Caching Parameters",
    description = "Enable prompt caching for cost optimization (50-90% savings)",
    example = json!({"cache_control": {"type": "ephemeral"}})
)]
pub struct PromptCachingParams {
    pub cache_control: Option<CacheControl>,
}

// ... Similar for all 7 adapters
```

Add to OpenAPI builder:
```rust
components(
    schemas(
        // ... existing schemas
        crate::server::openapi::extensions::ExtendedThinkingParams,
        crate::server::openapi::extensions::PromptCachingParams,
        crate::server::openapi::extensions::LogprobsParams,
        crate::server::openapi::extensions::JsonModeParams,
        crate::server::openapi::extensions::StructuredOutputsParams,
    ),
)
```

---

### Phase 6: Tauri UI Integration (2-3 days)

**Objective**: Create documentation tab in Tauri UI

**Steps**:
1. Install Scalar React component
2. Create DocumentationTab component
3. Add Tauri command to fetch spec
4. Add tab to navigation
5. Style to match existing UI

**Files Created**:
- `src/components/tabs/DocumentationTab.tsx`

**Files Modified**:
- `package.json` - Add @scalar/api-reference-react
- `src/App.tsx` - Add documentation tab route
- `src/components/Sidebar.tsx` - Add documentation to tabs
- `src-tauri/src/ui/commands.rs` - Add get_openapi_spec command
- `src-tauri/src/main.rs` - Register command

**Success Criteria**:
- ✅ Documentation tab visible in UI
- ✅ Spec loads and displays
- ✅ Navigation works
- ✅ Styling matches app

**Implementation**:

**1. Add dependency**:
```json
// package.json
{
  "dependencies": {
    "@scalar/api-reference-react": "^1.0.0"
  }
}
```

**2. Create DocumentationTab.tsx**:
```typescript
import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ApiReferenceReact } from '@scalar/api-reference-react'
import '@scalar/api-reference-react/style.css'
import Button from '../ui/Button'

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
}

export default function DocumentationTab() {
  const [spec, setSpec] = useState<string>('')
  const [config, setConfig] = useState<ServerConfig | null>(null)
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    loadServerConfig()
    loadOpenAPISpec()
  }, [])

  const loadServerConfig = async () => {
    const serverConfig = await invoke<ServerConfig>('get_server_config')
    setConfig(serverConfig)
  }

  const loadOpenAPISpec = async () => {
    try {
      setIsLoading(true)
      const openApiSpec = await invoke<string>('get_openapi_spec')
      setSpec(openApiSpec)
    } catch (err) {
      console.error('Failed to load spec:', err)
    } finally {
      setIsLoading(false)
    }
  }

  const downloadSpec = (format: 'json' | 'yaml') => {
    const blob = new Blob([spec], {
      type: format === 'json' ? 'application/json' : 'application/yaml'
    })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `localrouter-openapi.${format}`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
  }

  if (isLoading) {
    return <div className="p-6">Loading OpenAPI specification...</div>
  }

  const port = config?.actual_port ?? config?.port ?? 3625
  const baseUrl = `http://${config?.host ?? '127.0.0.1'}:${port}`

  return (
    <div className="h-full flex flex-col">
      <div className="p-4 border-b flex justify-between items-center">
        <h2 className="text-xl font-bold">API Documentation</h2>
        <div className="flex gap-2">
          <Button onClick={() => downloadSpec('json')}>Download JSON</Button>
          <Button onClick={() => downloadSpec('yaml')}>Download YAML</Button>
          <Button onClick={loadOpenAPISpec}>Refresh</Button>
        </div>
      </div>

      <div className="flex-1 overflow-auto">
        <ApiReferenceReact
          configuration={{
            spec: { content: spec },
            servers: [{ url: baseUrl, description: 'LocalRouter AI Server' }],
            authentication: {
              preferredSecurityScheme: 'bearer_auth',
            },
            darkMode: true,
            layout: 'modern',
            theme: 'purple',
            showSidebar: true,
          }}
        />
      </div>
    </div>
  )
}
```

**3. Add Tauri command**:
```rust
// src-tauri/src/ui/commands.rs

#[tauri::command]
pub async fn get_openapi_spec() -> Result<String, String> {
    Ok(crate::server::openapi::get_openapi_json())
}
```

**4. Register command**:
```rust
// src-tauri/src/main.rs

.invoke_handler(tauri::generate_handler![
    // ... existing commands
    get_openapi_spec,
])
```

**5. Add to navigation**:
```typescript
// src/App.tsx - Add case for 'documentation' tab
// src/components/Sidebar.tsx - Add 'documentation' to tabs list
```

---

### Phase 7: Advanced Features (2-3 days)

**Objective**: Add export and enhanced functionality

**Steps**:
1. Add Postman collection export
2. Add cURL command copying
3. Pre-fill authentication headers
4. Add API key selector

**Files Modified**:
- `src/components/tabs/DocumentationTab.tsx`
- `package.json` - Add openapi-to-postman

**Success Criteria**:
- ✅ Export to Postman works
- ✅ cURL commands copyable
- ✅ Auth pre-filled

**Implementation**:

**1. Postman Export**:
```typescript
import OpenapiToPostman from 'openapi-to-postman'

const exportPostman = () => {
  OpenapiToPostman.convert({
    type: 'json',
    data: JSON.parse(spec)
  }, (err, result) => {
    if (err) return console.error(err)

    const blob = new Blob([JSON.stringify(result.collection, null, 2)], {
      type: 'application/json'
    })
    // Download...
  })
}
```

**2. API Key Selector Dropdown**:

Add a dropdown above the Scalar component to select from existing API keys. This pre-fills the Authorization header for "Try It Out" functionality.

```typescript
interface ApiKeyInfo {
  id: string
  name: string
  key: string
  created_at: string
}

const [apiKeys, setApiKeys] = useState<ApiKeyInfo[]>([])
const [selectedKeyId, setSelectedKeyId] = useState<string>('')
const [selectedKey, setSelectedKey] = useState<string>('')

useEffect(() => {
  loadApiKeys()
}, [])

const loadApiKeys = async () => {
  const keys = await invoke<ApiKeyInfo[]>('list_api_keys')
  setApiKeys(keys)
  if (keys.length > 0) {
    setSelectedKeyId(keys[0].id)
    setSelectedKey(keys[0].key)
  }
}

const handleKeyChange = (event: React.ChangeEvent<HTMLSelectElement>) => {
  const keyId = event.target.value
  const key = apiKeys.find(k => k.id === keyId)
  if (key) {
    setSelectedKeyId(keyId)
    setSelectedKey(key.key)
  }
}

// UI with dropdown
<div className="p-4 border-b">
  <div className="flex justify-between items-center">
    <div className="flex gap-4 items-center">
      <h2 className="text-xl font-bold">API Documentation</h2>

      {/* API Key Selector */}
      <div className="flex items-center gap-2">
        <label className="text-sm font-medium">Test with API Key:</label>
        <select
          value={selectedKeyId}
          onChange={handleKeyChange}
          className="px-3 py-1 border rounded"
        >
          {apiKeys.length === 0 ? (
            <option value="">No API keys available</option>
          ) : (
            apiKeys.map(key => (
              <option key={key.id} value={key.id}>
                {key.name} ({key.key.slice(0, 8)}...)
              </option>
            ))
          )}
        </select>
      </div>
    </div>

    <div className="flex gap-2">
      <Button onClick={() => downloadSpec('json')}>Download JSON</Button>
      <Button onClick={() => downloadSpec('yaml')}>Download YAML</Button>
      <Button onClick={loadOpenAPISpec}>Refresh</Button>
    </div>
  </div>
</div>

// Pass selectedKey to Scalar configuration
<ApiReferenceReact
  configuration={{
    spec: { content: spec },
    servers: [{ url: baseUrl }],
    authentication: {
      preferredSecurityScheme: 'bearer_auth',
      apiKey: {
        token: selectedKey  // Pre-fills Authorization header
      }
    },
    // ... rest of config
  }}
/>
```

**Benefits**:
- Users can test endpoints immediately without manually entering API keys
- Dropdown shows key names for easy identification
- When API key is selected, all "Try It Out" requests automatically include `Authorization: Bearer <key>`
- Key is masked in dropdown for security (shows first 8 chars + ...)

---

### Phase 8: Testing & Validation (2-3 days)

**Objective**: Comprehensive testing

**Steps**:
1. Add integration tests for spec validity
2. Test all "Try It Out" functionality
3. Test exports (JSON, YAML, Postman)
4. Validate spec with external tools
5. Generate test client from spec

**Files Created**:
- `src-tauri/tests/openapi_tests.rs`

**Success Criteria**:
- ✅ All tests pass
- ✅ Spec validates externally
- ✅ Client can be generated
- ✅ Try It Out works for all endpoints

**Implementation**:

**Integration Test**:
```rust
// src-tauri/tests/openapi_tests.rs

#[tokio::test]
async fn test_openapi_spec_validity() {
    let spec_json = openapi::get_openapi_json();
    let spec: serde_json::Value = serde_json::from_str(&spec_json).unwrap();

    // Validate OpenAPI version
    assert_eq!(spec["openapi"], "3.1.0");

    // Validate all paths present
    let paths = spec["paths"].as_object().unwrap();
    assert!(paths.contains_key("/v1/chat/completions"));
    assert!(paths.contains_key("/v1/models"));
    assert!(paths.contains_key("/v1/generation"));

    // Validate security scheme
    let security = &spec["components"]["securitySchemes"]["bearer_auth"];
    assert!(security.is_object());
}

#[tokio::test]
async fn test_spec_matches_actual_routes() {
    // Start server
    let state = create_test_state().await;
    let app = build_app(state);

    // Get spec
    let spec_json = openapi::get_openapi_json();
    let spec: OpenAPI = serde_json::from_str(&spec_json).unwrap();

    // Verify each path in spec is handled by a route
    for (path, _) in spec.paths {
        let response = app.oneshot(
            Request::builder()
                .uri(path)
                .method("OPTIONS")
                .body(Body::empty())
                .unwrap()
        ).await.unwrap();

        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }
}
```

**External Validation**:
```bash
# Validate with swagger-cli
npx @apidevtools/swagger-cli validate http://localhost:3625/openapi.json

# Generate TypeScript client
npx openapi-typescript http://localhost:3625/openapi.json -o test-client.ts
tsc test-client.ts  # Should compile without errors
```

**Manual Testing Checklist**:
- [ ] Documentation tab loads in UI
- [ ] All 11 endpoints visible in docs
- [ ] Click on each endpoint shows details
- [ ] API key dropdown loads all available keys
- [ ] API key selection pre-fills Authorization header
- [ ] "Try It Out" works for GET /v1/models
- [ ] "Try It Out" works for POST /v1/chat/completions
- [ ] Authorization header pre-fills when API key selected
- [ ] Streaming endpoint documented correctly
- [ ] Feature adapter examples visible
- [ ] Download JSON spec works
- [ ] Download YAML spec works
- [ ] Export to Postman works
- [ ] Copy cURL command works
- [ ] Refresh button reloads spec

### Update CLAUDE.md Documentation

Add a new section to `CLAUDE.md` about OpenAPI requirements:

```markdown
## OpenAPI Documentation Requirements

LocalRouter AI uses OpenAPI 3.1 specification for API documentation. The spec is automatically generated from code annotations using utoipa.

### When Adding New Endpoints

When adding a new API endpoint, you MUST:

1. **Annotate the route handler** with `#[utoipa::path]`:
   ```rust
   #[utoipa::path(
       post,
       path = "/v1/your-endpoint",
       tag = "category",
       request_body = YourRequestType,
       responses(
           (status = 200, description = "Success", body = YourResponseType),
           (status = 400, description = "Bad request", body = ErrorResponse),
           (status = 401, description = "Unauthorized", body = ErrorResponse)
       ),
       security(("bearer_auth" = []))
   )]
   pub async fn your_handler(/* ... */) -> ApiResult<Response> {
       // Implementation
   }
   ```

2. **Add types to OpenAPI builder** in `src-tauri/src/server/openapi/mod.rs`:
   ```rust
   paths(
       // ... existing paths
       crate::server::routes::your_module::your_handler,
   ),
   components(
       schemas(
           // ... existing schemas
           crate::server::types::YourRequestType,
           crate::server::types::YourResponseType,
       ),
   )
   ```

3. **Ensure request/response types have schemas**:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
   #[schema(
       title = "Your Type",
       description = "What this type represents",
       example = json!({"field": "value"})
   )]
   pub struct YourType {
       #[schema(example = "example value")]
       pub field: String,
   }
   ```

4. **Refresh the documentation** after adding endpoints:
   - Compile: `cargo check` (spec regenerates automatically)
   - Verify: Access http://localhost:3625/openapi.json
   - Test: Open Documentation tab in UI and verify new endpoint appears

### When Modifying Existing Endpoints

When modifying an existing endpoint:

1. Update the `#[utoipa::path]` annotation if:
   - Path or method changes
   - Request/response types change
   - New query parameters or headers added
   - Error responses change

2. Update type schemas if:
   - New fields added/removed
   - Field types change
   - Validation constraints change (min, max, required, etc.)

3. Update examples to reflect realistic current usage

### Best Practices

- **Keep schemas in sync**: Always update OpenAPI annotations when changing types
- **Add descriptions**: Use `description` attribute for fields and endpoints
- **Provide examples**: Include realistic examples for complex types
- **Document errors**: List all possible error responses
- **Test "Try It Out"**: Verify endpoints work in Documentation tab before committing

### Validation

Before committing changes:

```bash
# Ensure spec compiles
cargo check

# Validate spec is valid OpenAPI 3.1
npx @apidevtools/swagger-cli validate http://localhost:3625/openapi.json

# Run tests
cargo test
```

### Common Mistakes

❌ **Don't**: Add endpoint without `#[utoipa::path]` annotation
✅ **Do**: Always annotate new endpoints

❌ **Don't**: Skip adding types to OpenAPI builder
✅ **Do**: Register all request/response types in `openapi/mod.rs`

❌ **Don't**: Leave `#[derive(ToSchema)]` off new types
✅ **Do**: Add schema derives to all API types

❌ **Don't**: Forget to update examples when behavior changes
✅ **Do**: Keep examples current and realistic
```

**File**: `/Users/matus/dev/localrouterai/CLAUDE.md`
**Section**: Add after "Quick Start Checklist"
**Impact**: Ensures all future development maintains OpenAPI spec compliance

---

## 4. Critical Files Summary

### Files to Create (5)

| File | Lines | Purpose |
|------|-------|---------|
| `src-tauri/src/server/openapi/mod.rs` | ~150 | OpenAPI builder and spec generator |
| `src-tauri/src/server/openapi/extensions.rs` | ~200 | Feature adapter schemas |
| `src/components/tabs/DocumentationTab.tsx` | ~200 | UI for documentation |
| `src-tauri/tests/openapi_tests.rs` | ~100 | Integration tests |
| `src-tauri/src/server/routes/openapi.rs` | ~50 | Spec serving endpoints |

### Files to Modify (9)

| File | Changes | Impact |
|------|---------|--------|
| `src-tauri/Cargo.toml` | Add utoipa dependencies | Low |
| `src-tauri/src/server/types.rs` | Add `#[derive(ToSchema)]` to 15+ types | Medium |
| `src-tauri/src/server/routes/*.rs` | Add `#[utoipa::path]` to 11 handlers | Medium |
| `src-tauri/src/server/mod.rs` | Add spec endpoints | Low |
| `src-tauri/src/ui/commands.rs` | Add get_openapi_spec command | Low |
| `src-tauri/src/main.rs` | Register command | Low |
| `src/App.tsx` | Add documentation tab route | Low |
| `src/components/Sidebar.tsx` | Add documentation to tabs | Low |
| `CLAUDE.md` | Add OpenAPI documentation requirements | Low |

---

## 5. Timeline & Effort

| Phase | Duration | Complexity | Dependencies |
|-------|----------|------------|--------------|
| Phase 1: Backend Foundation | 3-4 days | Medium | None |
| Phase 2: Route Documentation | 3-4 days | Medium | Phase 1 |
| Phase 3: OpenAPI Module | 2 days | Low | Phase 2 |
| Phase 4: Spec Serving | 1 day | Low | Phase 3 |
| Phase 5: Feature Adapters | 2 days | Low | Phase 3 |
| Phase 6: UI Integration | 2-3 days | Medium | Phase 4 |
| Phase 7: Advanced Features | 2-3 days | Medium | Phase 6 |
| Phase 8: Testing | 2-3 days | Medium | All |

**Total: 17-24 days (3.5-5 weeks)**

**Conservative estimate with buffer: 4 weeks**

---

## 6. Risks & Mitigations

### Risk 1: Streaming Response Documentation

**Problem**: OpenAPI doesn't have first-class SSE support

**Mitigation**:
- Document as alternative 200 response with `text/event-stream`
- Add examples showing SSE format
- Use `x-stream-format` extension

```yaml
responses:
  200:
    description: Success (streaming or non-streaming)
    content:
      application/json:
        schema: { $ref: '#/components/schemas/ChatCompletionResponse' }
      text/event-stream:
        schema:
          type: string
          x-stream-format: server-sent-events
```

### Risk 2: Extension Field Complexity

**Problem**: 7 feature adapters may confuse users expecting pure OpenAI

**Mitigation**:
- Mark `extensions` as optional
- Add clear description: "LocalRouter-specific (not part of OpenAI API)"
- Provide examples: one pure OpenAI, one with extensions

### Risk 3: Spec Drift

**Problem**: Spec could fall out of sync with code

**Mitigation**:
- utoipa generates spec at compile-time (impossible to drift)
- Add CI validation
- Integration tests verify routes match spec

### Risk 4: Performance Impact

**Problem**: Generating spec could slow down startup

**Mitigation**:
- utoipa generates at compile-time (zero runtime cost)
- Spec endpoints only called by UI (not on every API request)
- Consider lazy initialization if needed

---

## 7. Validation Strategy

### Compile-Time Validation

```bash
cargo check  # Validates all schemas
cargo clippy # Catches schema issues
cargo test   # Unit tests pass
```

### Runtime Validation

```rust
#[tokio::test]
async fn test_spec_validity() {
    let spec = openapi::get_openapi_json();
    let parsed: OpenAPI = serde_json::from_str(&spec).unwrap();
    assert_eq!(parsed.openapi, "3.1.0");
}
```

### External Validation

```bash
# Validate with swagger-cli
npx @apidevtools/swagger-cli validate http://localhost:3625/openapi.json

# Generate client to verify usability
npx openapi-typescript http://localhost:3625/openapi.json -o client.ts
tsc client.ts
```

---

## 8. Alternative Approaches Considered

### Option A: Pure Spec-First (User's Original Choice)

**Rejected because**:
- Would require rewriting 33,513 lines of code
- Streaming support would be lost
- Custom middleware would break
- Feature adapters not representable
- High risk of regressions
- 3-6 month timeline vs 3-4 weeks

### Option B: Manual OpenAPI Spec

**Rejected because**:
- High risk of spec drift
- Manual maintenance burden
- No compile-time validation
- Error-prone

### Option C: aide Framework

**Rejected because**:
- Less mature than utoipa
- Fewer examples and documentation
- More invasive code changes

---

## 9. Success Criteria

Implementation is complete when:

- [ ] All 11 endpoints documented in OpenAPI 3.1
- [ ] All request/response types have schemas
- [ ] Spec served at `/openapi.json` and `/openapi.yaml`
- [ ] Documentation tab visible in UI
- [ ] "Try It Out" works for all endpoints
- [ ] Streaming documented correctly
- [ ] All 7 feature adapters documented
- [ ] Export to JSON/YAML/Postman works
- [ ] Spec validates externally
- [ ] TypeScript client can be generated
- [ ] No breaking changes to existing API
- [ ] Integration tests pass

---

## 10. Next Steps

**Immediate Actions**:

1. **Review this plan** - Confirm code-first approach is acceptable
2. **Phase 1 kickoff** - Add utoipa dependencies
3. **Quick win** - Annotate 2-3 types to prove concept
4. **Validation** - Generate first partial spec and validate

**Decision Required**:

Do you want to proceed with the **code-first (utoipa)** approach or still prefer **pure spec-first** despite the identified risks?

---

## Appendix: Why utoipa vs Pure Spec-First

| Factor | utoipa (Code-First) | Pure Spec-First |
|--------|---------------------|-----------------|
| **Code Changes** | Add macros to existing code | Rewrite all routes |
| **Timeline** | 3-4 weeks | 3-6 months |
| **Risk** | Low (additive changes) | High (complete rewrite) |
| **Streaming** | Supported | Requires workarounds |
| **Middleware** | Preserved | Needs reimplementation |
| **Feature Adapters** | Fully documented | Limited support |
| **Spec Sync** | Compile-time guarantee | Manual maintenance |
| **Breaking Changes** | None | Complete API overhaul |
| **Learning Curve** | Minimal (add macros) | Steep (new architecture) |

**Recommendation**: Use utoipa (code-first) to achieve your goals without rewriting working code.
