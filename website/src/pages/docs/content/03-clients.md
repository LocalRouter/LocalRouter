<!-- @entry clients-overview -->

Clients represent the applications and tools that connect to LocalRouter. Each client has a name, an API key, and a reference to a routing strategy that controls which models, providers, and MCP servers it can access.

Client secrets are stored securely in your OS keychain — never in config files.

<!-- @entry creating-client-keys -->

When you create a client in the UI, LocalRouter auto-generates an API key in the format `lr-{random}` and stores it in the system keychain. A routing strategy is also created automatically for the client, which you can customize to control model access and rate limits.

<!-- @entry authentication-methods -->

LocalRouter supports two authentication methods for clients accessing the gateway.

**Bearer token** — Send the client API key directly in the `Authorization: Bearer {key}` header. This is the simplest method and works with any OpenAI-compatible client.

**OAuth 2.0 client credentials** — Exchange credentials at `POST /oauth/token` for a temporary access token (1-hour expiration). Useful for workflows that prefer short-lived tokens over long-lived API keys.

Both methods use the same `Authorization: Bearer` header format.

<!-- @entry auth-api-key -->

API key authentication is the recommended method for most use cases. Configure your OpenAI-compatible client with `http://localhost:3625` as the base URL and your `lr-*` key as the API key. LocalRouter verifies the key on each request and tracks the last-used timestamp for monitoring.

<!-- @entry auth-oauth -->

The OAuth 2.0 client credentials flow works by sending a `POST /oauth/token` request with `grant_type=client_credentials`, `client_id`, and `client_secret`. LocalRouter returns a short-lived access token (default 3600 seconds) that can be used in place of the API key.

Expired tokens are cleaned up automatically. When a token expires, request a new one from the token endpoint.

<!-- @entry auth-stdio -->

For local MCP servers launched via STDIO transport, authentication credentials are passed as environment variables to the subprocess. You can configure base environment variables in the MCP server settings, along with auth-specific variables that are merged at runtime.

<!-- @entry scoped-permissions -->

Each client has configurable access controls for both LLM providers and MCP servers.

**Provider access** — By default, clients can access all configured providers. You can restrict a client to specific providers (e.g., only OpenAI and Anthropic) from the client's strategy settings.

**MCP server access** — Clients can be configured with no MCP access, access to all servers, or access to specific named servers only.

Model-level permissions are controlled through the client's routing strategy.

<!-- @entry model-restrictions -->

Model restrictions are defined in the client's routing strategy and support three modes:

- **All models** — The client can use any model from any allowed provider.
- **Selected providers** — The client can use any model from specific providers (e.g., all OpenAI models).
- **Selected models** — The client can only use specific provider/model pairs (e.g., `openai/gpt-4o` and `anthropic/claude-sonnet-4-20250514`).

These modes can be combined for hierarchical control — for example, allowing all Anthropic models but only specific OpenAI models.

<!-- @entry provider-restrictions -->

Provider restrictions limit which LLM providers a client can access. Set this in the client's strategy by selecting specific providers. When a client sends a request to a restricted provider, it receives an error.

If no restrictions are set, all configured providers are accessible.

<!-- @entry mcp-server-restrictions -->

MCP server access is configured per-client with three options:

- **None** — The client cannot access any MCP servers.
- **All** — The client can access all configured MCP servers.
- **Specific** — The client can only access named MCP servers from a whitelist.

Access is enforced on every MCP request — attempts to call tools on restricted servers return an error.
