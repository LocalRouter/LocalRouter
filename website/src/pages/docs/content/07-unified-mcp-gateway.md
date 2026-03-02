<!-- @entry mcp-overview -->

The Unified MCP Gateway aggregates multiple upstream MCP (Model Context Protocol) servers behind a single HTTP endpoint. Clients connect once to LocalRouter and gain access to tools, resources, and prompts from all configured MCP servers without managing individual connections.

The gateway handles connection lifecycle, transport differences, authentication, and error isolation — if one upstream server fails, others continue working.

<!-- @entry tool-namespacing -->

To avoid naming collisions across MCP servers, the gateway prefixes every tool name with the server's ID using a double-underscore separator: `{server_id}__{tool_name}`. For example, a `search` tool from a server named `brave` becomes `brave__search`.

When a `tools/call` request arrives, the gateway strips the prefix to identify the target server and original tool name. This namespacing is transparent to upstream servers — they receive calls with their original tool names.

<!-- @entry transport-types -->

The MCP gateway supports three transport types for connecting to upstream MCP servers, matching the MCP specification. Each transport is configured per-server and handles connection establishment, message framing, and reconnection differently.

<!-- @entry transport-stdio -->

STDIO transport launches the MCP server as a child process and communicates via stdin/stdout pipes. This is the most common transport for local tools like file system access, databases, or CLI wrappers.

The gateway manages the process lifecycle — starting the process on first use, monitoring for crashes, and restarting as needed. Environment variables (including auth credentials) can be passed to the subprocess at launch.

<!-- @entry transport-sse -->

SSE (Server-Sent Events) transport connects to a remote MCP server over HTTP. The client sends JSON-RPC requests via POST and receives responses through a persistent SSE stream.

This transport is suitable for remote MCP servers that use the older SSE-based MCP transport specification.

<!-- @entry transport-streamable-http -->

Streamable HTTP is the modern MCP transport that uses standard HTTP POST requests with optional SSE streaming for responses. Each request is a standalone HTTP call, and the server can respond with a direct JSON response or upgrade to SSE for streaming.

This is the recommended transport for new remote MCP server implementations.

<!-- @entry deferred-tool-loading -->

By default, MCP server connections are deferred until first use. When LocalRouter starts, it does not immediately connect to all configured MCP servers — instead, it waits until a client sends a `tools/list` request or calls a tool on that server.

This reduces startup time and avoids errors from servers that may not be running. Once a server is accessed, its tools are cached until the connection is refreshed or the server is restarted.

<!-- @entry virtual-search-tool -->

When many MCP servers are configured (10+), the combined tool list can become very large, consuming excessive context in LLM conversations.

The virtual search tool addresses this by exposing a single `localrouter__search_tools` meta-tool that lets LLMs search across all available tools by keyword. Instead of receiving hundreds of tool definitions upfront, the LLM can query for relevant tools on demand. The search matches against tool names, descriptions, and server names.

<!-- @entry session-management -->

The MCP gateway maintains per-client sessions to track state across multiple requests. Each client connection gets a unique session ID (returned via the `Mcp-Session-Id` header) that maps to its upstream server connections and cached data.

Sessions handle the MCP protocol handshake, capability negotiation, and per-session state like resource subscriptions. Sessions expire after inactivity and are cleaned up automatically.

<!-- @entry response-caching -->

The gateway caches responses from upstream MCP servers to reduce latency and avoid redundant calls. Tool lists are cached per-server and invalidated when the server signals a change or the cache expires. Resource contents can also be cached.

Caching is transparent to clients — they always receive fresh-looking responses while the gateway handles staleness checks behind the scenes.

<!-- @entry partial-failure-handling -->

When a request spans multiple upstream servers (e.g., `tools/list` aggregates from all servers), the gateway handles partial failures gracefully. If one server is unreachable, tools from healthy servers are still returned — the failed server's tools are simply omitted.

For `tools/call` targeting a specific server, a failure is returned directly to the client since there's no fallback.

<!-- @entry mcp-oauth -->

For remote MCP servers that require OAuth authentication, LocalRouter implements a full OAuth 2.0 client flow with PKCE (Proof Key for Code Exchange).

This handles the complete browser-based authentication flow — opening the authorization URL, receiving the callback, exchanging the code for tokens, and storing them securely. OAuth configuration can be provided explicitly or discovered automatically from the server's well-known metadata endpoint.

<!-- @entry oauth-pkce-flow -->

The OAuth PKCE flow opens a browser window where you grant access to the MCP server. After authorization, LocalRouter receives an access token and refresh token, stored securely in the keychain.

PKCE prevents authorization code interception attacks without requiring a client secret, making it suitable for desktop applications.

<!-- @entry oauth-auto-discovery -->

When an MCP server requires OAuth but no explicit configuration is provided, LocalRouter attempts auto-discovery by fetching the server's `/.well-known/oauth-authorization-server` metadata document.

If discovery succeeds, the OAuth flow is configured automatically without requiring manual endpoint setup.

<!-- @entry oauth-token-refresh -->

Access tokens from OAuth flows have limited lifetimes (typically 1 hour). When a token expires, LocalRouter automatically uses the stored refresh token to obtain a new access token without requiring any action from you.

If the refresh token itself expires or is revoked, you'll be prompted to re-authenticate through the browser flow.
