# LocalRouter AI - API Endpoint Specification

This document defines the OpenAI-compatible API endpoints for LocalRouter AI. These endpoints follow the OpenAI API standard and work as a drop-in replacement for applications using the OpenAI SDK.

**Scope**: This specification covers the 5 core endpoints we're implementing first:
1. Chat Completions
2. Completions (Legacy)
3. Embeddings
4. Models
5. Generation Details

---

## Configuration & Security

### API Key Storage
- **All provider API keys** (OpenAI, Anthropic, Gemini, etc.) MUST be stored in the **encrypted storage** system
- Keys are never stored in plain text in configuration files
- Configuration files reference keys by ID: `api_key_ref: "key_id_123"`
- The encrypted storage module handles encryption/decryption at runtime

### Provider Configuration
- Provider settings (base URLs, endpoints, enabled/disabled status) are stored in the **configuration system** (`config/mod.rs`)
- Configuration is loaded from YAML files on startup
- Runtime changes to configuration are persisted back to disk
- Example configuration structure:
  ```yaml
  providers:
    - name: "my-openai"
      provider_type: OpenAI
      enabled: true
      api_key_ref: "openai_key_1"  # References encrypted storage
      endpoint: null  # Use default
      parameters:
        organization: "org-123"  # Optional parameters
  ```

### Authentication
- Client requests authenticate using LocalRouter API keys: `Authorization: Bearer lr-<key>`
- These keys are managed by the API key management system
- Keys are hashed (bcrypt) and stored in `api_keys.json`
- Each key can specify which models/routers to use

---

## Base URL

All endpoints are served under:
```
http://localhost:8080/v1
```

(Port configurable in settings)

---

## 1. Chat Completions

**The primary endpoint for conversational AI interactions.**

### Endpoint
```
POST /v1/chat/completions
```

### Request Headers
```
Authorization: Bearer <LOCALROUTER_API_KEY>
Content-Type: application/json
```

### Request Body

```typescript
{
  // ===== REQUIRED =====
  model: string;                    // Model ID (e.g., "gpt-4", "claude-3-opus")
  messages: Message[];              // Array of conversation messages

  // ===== SAMPLING PARAMETERS =====
  temperature?: number;             // [0, 2] Controls randomness. Default: 1.0
  top_p?: number;                   // (0, 1] Nucleus sampling. Default: 1.0

  // ===== OUTPUT CONTROL =====
  max_tokens?: number;              // Maximum tokens to generate
  stop?: string | string[];         // Stop sequences

  // ===== STREAMING =====
  stream?: boolean;                 // Enable SSE streaming. Default: false

  // ===== ADVANCED =====
  frequency_penalty?: number;       // [-2, 2] Reduce repetition
  presence_penalty?: number;        // [-2, 2] Encourage new topics

  // ===== TOOL CALLING =====
  tools?: Tool[];                   // Function definitions (if provider supports)
  tool_choice?: ToolChoice;         // 'auto' | 'none' | {function}

  // ===== USER TRACKING =====
  user?: string;                    // Stable user ID for rate limiting
}
```

### Message Schema

```typescript
type Message = {
  role: 'user' | 'system' | 'assistant';
  content: string | ContentPart[];  // Text or multimodal content
  name?: string;                    // Optional name/identifier
}

// For multimodal support (vision):
type ContentPart =
  | { type: 'text'; text: string }
  | {
      type: 'image_url';
      image_url: {
        url: string;                // URL or base64 data URI
        detail?: 'auto' | 'low' | 'high';
      }
    };
```

### Response (Non-Streaming)

```typescript
{
  id: string;                       // Generation ID (e.g., "gen-abc123")
  object: 'chat.completion';
  created: number;                  // Unix timestamp
  model: string;                    // Model that processed request
  choices: [{
    index: number;
    message: {
      role: 'assistant';
      content: string;
      tool_calls?: ToolCall[];      // If tools were used
    };
    finish_reason: string;          // 'stop', 'length', 'tool_calls', 'content_filter'
  }];
  usage: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}
```

### Response (Streaming - SSE)

When `stream: true`, responses are sent as Server-Sent Events:

```
data: {"id":"gen-abc","object":"chat.completion.chunk","created":1705267200,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

data: {"id":"gen-abc","object":"chat.completion.chunk","created":1705267200,"model":"gpt-4","choices":[{"index":0,"delta":{"content":" there"},"finish_reason":null}]}

data: {"id":"gen-abc","object":"chat.completion.chunk","created":1705267200,"model":"gpt-4","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}

data: [DONE]
```

**Streaming Response Object:**
```typescript
{
  id: string;
  object: 'chat.completion.chunk';
  created: number;
  model: string;
  choices: [{
    index: number;
    delta: {
      role?: string;                // Only in first chunk
      content?: string;             // Content delta
      tool_calls?: ToolCall[];
    };
    finish_reason: string | null;   // Null until final chunk
  }];
  usage?: {                         // Only in final chunk
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}
```

### Tool Calling Support

```typescript
interface Tool {
  type: 'function';
  function: {
    name: string;
    description?: string;
    parameters: JSONSchema;         // JSON Schema describing parameters
  };
}

type ToolChoice =
  | 'auto'                          // Model decides
  | 'none'                          // Disable tools
  | { type: 'function'; function: { name: string } };  // Force specific tool
```

### Error Response

```typescript
{
  error: {
    message: string;
    type: string;                   // 'invalid_request_error', 'rate_limit_error', etc.
    param?: string;                 // Parameter that caused error
    code?: string | number;
  }
}
```

### HTTP Status Codes

- `200` - Success
- `400` - Bad Request (invalid parameters)
- `401` - Unauthorized (invalid API key)
- `403` - Forbidden (rate limited, quota exceeded)
- `429` - Rate Limited
- `500` - Server Error
- `502` - Bad Gateway (provider error)
- `503` - Service Unavailable

### Provider-Specific Parameter Handling

The gateway transforms parameters based on the target provider:

**OpenAI:**
- Supports: `temperature`, `top_p`, `frequency_penalty`, `presence_penalty`, `max_tokens`, `stop`, `tools`, `tool_choice`
- NOT supported: `top_k` (silently dropped)

**Anthropic Claude:**
- Supports: `temperature`, `top_p`, `top_k`, `max_tokens`
- System messages extracted to separate `system` parameter
- NOT supported: `frequency_penalty`, `presence_penalty` (silently dropped)

**Google Gemini:**
- Supports: `temperature`, `top_p`, `top_k`, `max_tokens`, `stop`
- System messages prepended to first user message
- NOT supported: `frequency_penalty`, `presence_penalty` (silently dropped)

**Ollama:**
- Supports: `temperature`, `top_p`, `max_tokens`, `stop`
- All parameters are Ollama-compatible
- Free local models

---

## 2. Completions (Legacy)

**Text-in, text-out completions without chat message format.**

### Endpoint
```
POST /v1/completions
```

### Request Headers
```
Authorization: Bearer <LOCALROUTER_API_KEY>
Content-Type: application/json
```

### Request Body

```typescript
{
  // ===== REQUIRED =====
  model: string;                    // Model ID
  prompt: string | string[];        // Text prompt(s)

  // ===== SAMPLING =====
  temperature?: number;             // [0, 2] Default: 1.0
  top_p?: number;                   // (0, 1] Default: 1.0

  // ===== OUTPUT =====
  max_tokens?: number;              // Maximum tokens to generate
  stop?: string | string[];         // Stop sequences

  // ===== ADVANCED =====
  frequency_penalty?: number;       // [-2, 2]
  presence_penalty?: number;        // [-2, 2]

  // ===== STREAMING =====
  stream?: boolean;                 // Enable SSE streaming

  // ===== MISC =====
  user?: string;                    // User ID for rate limiting
}
```

### Response (Non-Streaming)

```typescript
{
  id: string;
  object: 'text_completion';
  created: number;
  model: string;
  choices: [{
    text: string;                   // Generated text
    index: number;
    finish_reason: string;          // 'stop', 'length'
    logprobs?: null;
  }];
  usage: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}
```

### Response (Streaming - SSE)

Similar to chat completions, but with `text` instead of `content`:

```
data: {"id":"gen-xyz","object":"text_completion.chunk","created":1705267200,"choices":[{"text":"Hello","index":0,"finish_reason":null}]}

data: [DONE]
```

### Notes

- This is a legacy endpoint maintained for backward compatibility
- Most providers now prefer the chat completions format
- LocalRouter will convert completion requests to chat format internally when needed
- Providers like Anthropic and OpenAI's newer models only support chat format

---

## 3. Embeddings

**Convert text to vector embeddings for semantic search and retrieval.**

### Endpoint
```
POST /v1/embeddings
```

### Request Headers
```
Authorization: Bearer <LOCALROUTER_API_KEY>
Content-Type: application/json
```

### Request Body

```typescript
{
  // ===== REQUIRED =====
  model: string;                    // Embedding model (e.g., "text-embedding-3-small")
  input: string | string[];         // Text(s) to embed

  // ===== OPTIONAL =====
  encoding_format?: 'float' | 'base64';  // Output format. Default: 'float'
  dimensions?: number;              // Custom dimensions (if model supports)
  user?: string;                    // User ID for tracking
}
```

### Response

```typescript
{
  object: 'list';
  data: [{
    object: 'embedding';
    embedding: number[];            // Vector (float array) or base64 string
    index: number;                  // Input index
  }];
  model: string;
  usage: {
    prompt_tokens: number;
    total_tokens: number;
  };
}
```

### Supported Embedding Models

**OpenAI:**
- `text-embedding-3-small` - 1536 dimensions, $0.02/1M tokens
- `text-embedding-3-large` - 3072 dimensions, $0.13/1M tokens
- `text-embedding-ada-002` - 1536 dimensions (legacy)

**Other Providers:**
- Provider-specific embedding models available based on configuration
- LocalRouter normalizes the response format to OpenAI standard

### Example Request

```bash
curl http://localhost:8080/v1/embeddings \
  -H "Authorization: Bearer lr-your-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "text-embedding-3-small",
    "input": "The quick brown fox jumps over the lazy dog"
  }'
```

### Example Response

```json
{
  "object": "list",
  "data": [{
    "object": "embedding",
    "embedding": [0.0023, -0.0091, 0.0156, ...],  // 1536 floats
    "index": 0
  }],
  "model": "text-embedding-3-small",
  "usage": {
    "prompt_tokens": 10,
    "total_tokens": 10
  }
}
```

### Notes

- Batch embedding: Send array of strings to embed multiple texts in one request
- Dimension reduction: Some models support custom `dimensions` parameter
- Use embeddings for: semantic search, clustering, recommendations, anomaly detection

---

## 4. List Models

**Discover available models and their capabilities.**

### Endpoint
```
GET /v1/models
```

### Request Headers
```
Authorization: Bearer <LOCALROUTER_API_KEY>
```

### Query Parameters

None - returns all available models from all configured providers.

### Response

```typescript
{
  object: 'list';
  data: [{
    id: string;                     // Model ID (e.g., "gpt-4-turbo", "claude-3-opus")
    object: 'model';
    owned_by: string;               // Provider name (e.g., "openai", "anthropic")
    created?: number;               // Unix timestamp (if available)

    // LocalRouter-specific metadata
    provider: string;               // Provider instance name
    parameter_count?: number;       // Model size in parameters (if known)
    context_window: number;         // Maximum context length in tokens
    supports_streaming: boolean;    // Whether streaming is supported

    capabilities: string[];         // ['chat', 'completion', 'vision', 'function_calling', etc.]

    pricing?: {
      input_cost_per_1k: number;    // USD per 1K input tokens
      output_cost_per_1k: number;   // USD per 1K output tokens
      currency: string;             // "USD"
    };
  }];
}
```

### Capability Values

```typescript
type Capability =
  | 'chat'                          // Chat completions
  | 'completion'                    // Legacy completions
  | 'embedding'                     // Embeddings
  | 'vision'                        // Image understanding
  | 'function_calling'              // Tool/function calling
  | 'streaming'                     // Streaming support
  | 'structured_output';            // JSON mode
```

### Example Response

```json
{
  "object": "list",
  "data": [
    {
      "id": "gpt-4-turbo",
      "object": "model",
      "owned_by": "openai",
      "created": 1705267200,
      "provider": "my-openai",
      "context_window": 128000,
      "supports_streaming": true,
      "capabilities": ["chat", "vision", "function_calling", "structured_output"],
      "pricing": {
        "input_cost_per_1k": 0.01,
        "output_cost_per_1k": 0.03,
        "currency": "USD"
      }
    },
    {
      "id": "claude-3-opus",
      "object": "model",
      "owned_by": "anthropic",
      "provider": "my-anthropic",
      "context_window": 200000,
      "supports_streaming": true,
      "capabilities": ["chat", "vision"],
      "pricing": {
        "input_cost_per_1k": 0.015,
        "output_cost_per_1k": 0.075,
        "currency": "USD"
      }
    },
    {
      "id": "llama3.3:70b",
      "object": "model",
      "owned_by": "meta",
      "provider": "my-ollama",
      "parameter_count": 70000000000,
      "context_window": 8192,
      "supports_streaming": true,
      "capabilities": ["chat"],
      "pricing": {
        "input_cost_per_1k": 0.0,
        "output_cost_per_1k": 0.0,
        "currency": "USD"
      }
    }
  ]
}
```

### Notes

- Models list is dynamically generated from all **enabled** provider instances
- Pricing information is pulled from provider implementations
- Context window and capabilities are detected from provider APIs or hard-coded
- Health status is **not** included in this endpoint (see Generation Details for health)

---

## 5. Generation Details

**Retrieve detailed information about a specific generation.**

### Endpoint
```
GET /v1/generation?id={generation_id}
```

### Request Headers
```
Authorization: Bearer <LOCALROUTER_API_KEY>
```

### Query Parameters

- `id` (required) - The generation ID returned from chat/completions response

### Response

```typescript
{
  id: string;                       // Generation ID
  model: string;                    // Model used
  provider: string;                 // Provider instance name
  created: number;                  // Unix timestamp
  finish_reason: string;            // 'stop', 'length', 'tool_calls', etc.

  // Token usage
  tokens: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };

  // Cost calculation
  cost?: {
    prompt_cost: number;            // USD
    completion_cost: number;        // USD
    total_cost: number;             // USD
    currency: string;               // "USD"
  };

  // Performance metrics
  latency_ms: number;               // Total request latency

  // Provider health at time of request
  provider_health?: {
    status: 'healthy' | 'degraded' | 'unhealthy';
    latency_ms?: number;
  };

  // Request metadata
  api_key_id: string;               // Masked API key
  user?: string;                    // User ID if provided
  stream: boolean;                  // Whether request was streamed
}
```

### Example Request

```bash
curl "http://localhost:8080/v1/generation?id=gen-abc123" \
  -H "Authorization: Bearer lr-your-key"
```

### Example Response

```json
{
  "id": "gen-abc123",
  "model": "gpt-4-turbo",
  "provider": "my-openai",
  "created": 1705267200,
  "finish_reason": "stop",
  "tokens": {
    "prompt_tokens": 150,
    "completion_tokens": 75,
    "total_tokens": 225
  },
  "cost": {
    "prompt_cost": 0.0015,
    "completion_cost": 0.00225,
    "total_cost": 0.00375,
    "currency": "USD"
  },
  "latency_ms": 1523,
  "provider_health": {
    "status": "healthy",
    "latency_ms": 89
  },
  "api_key_id": "lr-abc***xyz",
  "stream": false
}
```

### Notes

- Generation details are stored in memory/database during request processing
- Useful for cost tracking and debugging
- For streaming requests, token counts are available after stream completes
- Generation IDs are unique across all requests
- Details are retained for a configurable period (e.g., 7 days)

---

## Configuration Integration

### Provider Configuration Example

```yaml
# config.yaml
providers:
  - name: "my-openai"
    provider_type: OpenAI
    enabled: true
    api_key_ref: "openai_prod_key"  # References encrypted storage
    parameters:
      organization: "org-123abc"     # Optional

  - name: "my-anthropic"
    provider_type: Anthropic
    enabled: true
    api_key_ref: "anthropic_key_1"

  - name: "local-ollama"
    provider_type: Ollama
    enabled: true
    endpoint: "http://localhost:11434"
    # No api_key_ref - Ollama is local and free
```

### Encrypted Storage Reference

API keys are **never** stored in plain text:

```rust
// Load API key from encrypted storage
let api_key = encrypted_storage
    .get_secret(&config.api_key_ref)
    .await?;

// Create provider with decrypted key
let provider = OpenAIProvider::new(api_key);
```

### API Key Management

Client API keys are managed separately:

```json
// api_keys.json (stored with bcrypt hashes)
{
  "keys": [
    {
      "id": "lr-abc123",
      "name": "Production App",
      "hash": "$2b$12$...",
      "model_selection": {
        "type": "direct",
        "model": "gpt-4-turbo"
      },
      "rate_limits": {
        "requests_per_minute": 60,
        "tokens_per_minute": 90000
      }
    }
  ]
}
```

---

## Implementation Notes

### Streaming Implementation

- Use Server-Sent Events (SSE) for streaming
- Set headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`
- Each chunk prefixed with `data: `
- Final message is `data: [DONE]`
- Use Futures `Stream` trait for provider streaming
- Convert provider-specific streaming format to OpenAI format

### Request Flow

```
Client Request
    ↓
[1] Authenticate API Key
    ↓
[2] Check Rate Limits
    ↓
[3] Validate Request Schema
    ↓
[4] Determine Target Provider (via router or direct)
    ↓
[5] Load Provider from Registry
    ↓
[6] Transform Request to Provider Format
    ↓
[7] Call Provider
    ↓
[8] Transform Response to OpenAI Format
    ↓
[9] Record Usage (tokens, cost)
    ↓
[10] Return Response to Client
```

### Error Handling

- Validate all required fields before processing
- Return OpenAI-compatible error responses
- Log errors with context (provider, model, api_key_id)
- Don't expose provider API keys in error messages
- Include request ID for debugging

### Security

- All provider API keys encrypted at rest
- Client API keys hashed with bcrypt
- Never log raw API keys
- Validate request body size limits
- Implement rate limiting per API key
- CORS configuration for web clients

---

## Testing

### Manual Testing

```bash
# Chat completion
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer lr-your-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Say hello"}]
  }'

# Streaming
curl http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer lr-your-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Count to 10"}],
    "stream": true
  }'

# List models
curl http://localhost:8080/v1/models \
  -H "Authorization: Bearer lr-your-key"

# Embeddings
curl http://localhost:8080/v1/embeddings \
  -H "Authorization: Bearer lr-your-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "text-embedding-3-small",
    "input": "Hello world"
  }'

# Generation details
curl "http://localhost:8080/v1/generation?id=gen-abc123" \
  -H "Authorization: Bearer lr-your-key"
```

### Integration Tests

- Test each endpoint with valid and invalid inputs
- Test streaming and non-streaming modes
- Test with multiple providers (OpenAI, Anthropic, Ollama)
- Test rate limiting
- Test error handling (invalid API key, missing model, etc.)
- Test token counting accuracy
- Test cost calculation

---

## Future Extensions

### Not Yet Implemented

These endpoints are documented in the full spec but **not** in initial scope:

- Image generation (`POST /v1/images/generations`)
- Audio transcription (`POST /v1/audio/transcriptions`)
- Audio speech synthesis (`POST /v1/audio/speech`)
- Moderation (`POST /v1/moderations`)
- Batch processing (`POST /v1/batches`)
- Token counting (`POST /v1/tokens/count`)
- Admin endpoints (provider/model/key management via API)

We may add these in future phases based on demand.

---

**Last Updated**: 2026-01-14
**Version**: 1.0.0
**Status**: Initial Implementation Plan
