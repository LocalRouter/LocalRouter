<!-- @entry openai-authentication -->

All OpenAI-compatible endpoints require authentication via a Bearer token in the `Authorization` header. Use your LocalRouter client secret (format: `lr-{random}`) or an OAuth access token obtained from `POST /oauth/token`. Example:

```
Authorization: Bearer lr-your_secret_key_here
```

Requests without a valid token receive a `401 Unauthorized` response. The special token `internal-test` is available for UI testing only and bypasses client restrictions.

All endpoints below are served at the root path (e.g., `GET /models`). The `/v1` prefix is also accepted for compatibility with clients that include it (e.g., `GET /v1/models`). Both the OpenAI gateway and MCP gateway share the same root — their endpoints do not conflict.

<!-- @entry openai-models -->

`GET /models` returns a list of all models available to the authenticated client, filtered by the client's strategy permissions. The response follows the OpenAI models list format with `id`, `object`, `created`, and `owned_by` fields.

Model IDs use the format `provider/model_name` (e.g., `openai/gpt-4o`, `anthropic/claude-sonnet-4-20250514`). The special model `localrouter/auto` is included when the client's strategy has auto-routing configured.

<!-- @entry openai-chat-completions -->

`POST /chat/completions` is the primary endpoint for LLM inference. It accepts the standard OpenAI chat completions request format with `model`, `messages`, `temperature`, `max_tokens`, `stream`, `tools`, and other parameters. The `model` field accepts either a specific model ID (`openai/gpt-4o`) or `localrouter/auto` for intelligent routing.

Responses follow the OpenAI format with `choices`, `usage` (token counts), and `model` (the actual model used). Streaming is supported via `stream: true`.

<!-- @entry openai-completions -->

`POST /completions` provides the legacy text completions API. It accepts a `prompt` string (instead of `messages`) along with standard parameters like `model`, `max_tokens`, `temperature`, and `stop`. This endpoint is primarily for backward compatibility with older applications.

The response includes `choices` with `text` and `finish_reason` fields. Not all providers support this endpoint — those that don't will return a `400` error.

<!-- @entry openai-embeddings -->

`POST /embeddings` generates vector embeddings for input text. The request includes `model` (must be an embedding model like `openai/text-embedding-3-small`) and `input` (a string or array of strings). The response contains an array of embedding objects, each with a `float[]` vector and token usage data.

Embedding dimensions vary by model. This endpoint is useful for RAG pipelines, semantic search, and similarity comparisons.

<!-- @entry openai-health -->

`GET /health` returns the server's health status. This endpoint does not require authentication and is suitable for load balancer health checks and monitoring.

The response includes the server status, uptime, version, and the number of configured providers and clients. A `200` status code indicates the server is healthy and ready to accept requests.

<!-- @entry openai-spec -->

`GET /openapi.json` returns the full OpenAPI 3.0 specification for all LocalRouter endpoints. The spec is auto-generated from the Rust source code using `utoipa` annotations and includes request/response schemas, authentication requirements, and endpoint descriptions.

This spec can be imported into API clients like Postman or used to generate client libraries in any language.

<!-- @entry openai-streaming -->

When `stream: true` is set in a chat completions request, the response uses Server-Sent Events (SSE). Each event contains a `data:` line with a JSON chunk following the OpenAI streaming format: `choices[0].delta` contains incremental content tokens.

The stream begins with a chunk containing the role, continues with content deltas, and ends with a `data: [DONE]` sentinel. Token usage is included in the final chunk before `[DONE]`. The `Content-Type` header is set to `text/event-stream`.

<!-- @entry openai-errors -->

Error responses follow the OpenAI error format with an `error` object containing `message`, `type`, `param`, and `code` fields. Common error codes:

- `401` — Invalid or missing authentication token
- `403` — Client lacks permission for the requested model or action (also used for firewall denials)
- `404` — Model not found or not available to the client
- `429` — Rate limit exceeded (includes `Retry-After` header)
- `500` — Internal server error or upstream provider failure
- `502` — Upstream provider returned an invalid response
- `503` — All providers in the fallback chain are unavailable
