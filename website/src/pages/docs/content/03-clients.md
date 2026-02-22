<!-- @entry clients-overview -->

LocalRouter implements a unified client system that consolidates API keys and OAuth clients into a single `Client` entity. Each client has a unique internal ID, human-readable name, enabled status, and a reference to a routing strategy. Clients are stored in the config file while their secrets are kept secure in the system keychain (`LocalRouter-Clients` service). The system supports multiple authentication methods for both inbound client access and outbound MCP server authentication, with built-in access control for LLM providers and MCP servers.

<!-- @entry creating-client-keys -->

Client API keys are created through the `ClientManager::create_client()` method, which auto-generates a client secret in the format `lr-{random}` using `crypto::generate_api_key()`. The secret is immediately stored in the system keychain under the service `LocalRouter-Clients` with the client ID as the account name, ensuring secrets are never persisted in plain text. Each created client also automatically generates a corresponding routing strategy (named `{client_name}'s strategy`) that defines model permissions and rate limits.

<!-- @entry authentication-methods -->

LocalRouter supports three distinct authentication methods for clients accessing the gateway. **Bearer token** authentication uses the client secret directly in the `Authorization: Bearer {secret}` header. **OAuth 2.0 client credentials** flow allows clients to exchange credentials at `POST /oauth/token` for temporary access tokens (1-hour expiration) stored in-memory. A third method uses the **internal test token** (`internal-test`) for UI testing only, which bypasses client restrictions. All methods use the same `Authorization: Bearer` header format.

<!-- @entry auth-api-key -->

API key (Bearer token) authentication is the simplest method: clients send their secret directly in the `Authorization: Bearer {client_secret}` header of every request. The `ClientManager::verify_secret()` method iterates through all enabled clients, retrieves their secrets from the keychain, and returns the matching client if credentials are valid. Last-used timestamps are automatically updated when authentication succeeds, providing usage tracking for monitoring client activity.

<!-- @entry auth-oauth -->

OAuth 2.0 client credentials flow is implemented via the `POST /oauth/token` endpoint. Clients submit `grant_type=client_credentials` with their `client_id` and `client_secret` to receive a short-lived access token (default 3600 seconds). The `TokenStore` maintains an in-memory map of valid tokens with expiration times, automatically cleaning up expired tokens every 5 minutes. The authentication middleware tries OAuth tokens first before checking direct secrets, enabling seamless token-based workflows.

<!-- @entry auth-stdio -->

STDIO pipe authentication for local MCP servers is configured through the `McpAuthConfig::EnvVars` variant, which passes authentication credentials as environment variables to the subprocess. Base environment variables are defined in `McpTransportConfig::Stdio.env`, while auth-specific variables come from `McpAuthConfig::EnvVars.env` and are merged at runtime. This method is ideal for local processes that accept credentials via environment variables rather than HTTP headers.

<!-- @entry scoped-permissions -->

Client access control is enforced through the `Client` struct fields and `ClientManager` methods like `can_access_llm()` and `can_access_mcp_server()`. Clients have an `allowed_llm_providers` list (empty means all providers are accessible) and `mcp_server_access` field that uses the `McpServerAccess` enum supporting `None` (no access), `All` (all servers), or `Specific(Vec<String>)` (named servers only). Model-level permissions are further controlled via the client's associated routing strategy, which defines `allowed_models` with provider-based and specific model selections.

<!-- @entry model-restrictions -->

Model restrictions are applied through the `Strategy` struct associated with each client, not directly on the client. The strategy's `allowed_models` field contains an `AvailableModelsSelection` enum with three modes: `selected_all=true` (all models allowed), `selected_providers` (list of providers where all models are allowed), or `selected_models` (list of specific provider-model pairs). The `is_model_allowed(provider, model)` method evaluates these in order, allowing hierarchical control from broad provider-level restrictions down to specific model blocking.

<!-- @entry provider-restrictions -->

LLM provider restrictions are controlled via the `allowed_models.selected_providers` field in the client's routing strategy. Setting `selected_providers: ["openai", "anthropic"]` restricts the client to only those two providers while blocking all others. If `selected_all` is true, all providers are available; if false and the provider list is non-empty, only listed providers are accessible.

<!-- @entry mcp-server-restrictions -->

MCP server restrictions use the `mcp_server_access` field in the `Client` struct, which implements the `McpServerAccess` enum. When set to `Specific(vec!["server1", "server2"])`, only those MCP servers are accessible. Access control is enforced in the `client_auth_middleware`, which authenticates the client and makes the client ID available for downstream access checks.
