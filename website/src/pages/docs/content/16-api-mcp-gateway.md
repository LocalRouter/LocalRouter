<!-- @entry mcp-endpoint -->

`POST /mcp` is the single entry point for all MCP operations. It accepts JSON-RPC 2.0 requests following the Model Context Protocol specification. The endpoint supports both direct JSON responses and SSE streaming (based on the `Accept` header). Session state is tracked via the `Mcp-Session-Id` header — if omitted on the first request, a new session is created and the ID is returned in the response header.

<!-- @entry mcp-tool-namespacing -->

Tools from different MCP servers are namespaced using double underscores: `{server_id}__{tool_name}`. For example, the `search` tool from the `brave` server appears as `brave__search` in the tool list. When calling a tool, use the namespaced name — the gateway strips the prefix to route the call to the correct upstream server. Server IDs are the human-readable names configured in LocalRouter, not internal UUIDs.

<!-- @entry mcp-session-lifecycle -->

MCP sessions follow the protocol's lifecycle: `initialize` → `initialized` notification → operational requests → `close`. The `initialize` request negotiates protocol version and capabilities between the client and gateway. After `initialized`, the client can call `tools/list`, `tools/call`, `resources/list`, etc. Sessions persist across multiple HTTP requests using the `Mcp-Session-Id` header. Idle sessions are automatically cleaned up after a configurable timeout (default: 30 minutes).

<!-- @entry mcp-authentication -->

MCP endpoint authentication uses the same mechanism as the OpenAI endpoints — a Bearer token in the `Authorization` header. The client's MCP server access permissions (configured via `mcp_server_access` on the client) determine which upstream servers are accessible. Requests to `tools/call` for a server the client cannot access return a JSON-RPC error with code `-32600` (Invalid Request).

<!-- @entry mcp-methods -->

The MCP gateway supports all standard MCP methods, proxying them to the appropriate upstream server(s). Aggregate methods (`tools/list`, `resources/list`, `prompts/list`) query all accessible servers and merge the results. Targeted methods (`tools/call`, `resources/read`, `prompts/get`) route to a specific server based on the namespaced identifier.

<!-- @entry mcp-tools-list -->

`tools/list` returns an aggregated list of tools from all accessible MCP servers. Each tool includes its namespaced `name`, `description`, and `inputSchema` (JSON Schema). The list is cached per-session and refreshed when servers signal tool changes. If the virtual search tool is enabled and the total tool count exceeds the configured threshold, only the search tool is returned by default — clients must use it to discover specific tools.

<!-- @entry mcp-tools-call -->

`tools/call` invokes a specific tool on the target MCP server. The request includes the namespaced tool name and `arguments` object matching the tool's input schema. The gateway strips the namespace prefix, routes the call to the correct upstream server, and returns the result. Results follow the MCP `CallToolResult` format with `content` (array of text/image/resource blocks) and `isError` flag. Timeouts are configurable per-server.

<!-- @entry mcp-resources-list -->

`resources/list` returns an aggregated list of resources from all accessible MCP servers. Each resource includes a namespaced `uri`, `name`, `description`, and optional `mimeType`. Resources represent data the server can provide (files, database records, API data) that can be read via `resources/read`. The list is cached similarly to tools.

<!-- @entry mcp-resources-read -->

`resources/read` retrieves the content of a specific resource by its namespaced URI. The gateway routes the request to the server that owns the resource and returns the content in the MCP `ReadResourceResult` format. Content can be text (returned as `text` field) or binary (returned as base64-encoded `blob` field with a `mimeType`).

<!-- @entry mcp-prompts-list -->

`prompts/list` returns an aggregated list of prompt templates from all accessible MCP servers. Each prompt includes a namespaced `name`, `description`, and `arguments` schema defining required/optional parameters. Prompts are reusable templates that generate messages for specific use cases.

<!-- @entry mcp-prompts-get -->

`prompts/get` retrieves a specific prompt template with its arguments filled in. The request includes the namespaced prompt name and an `arguments` object. The server returns a `GetPromptResult` with `description` and `messages` — an array of role/content pairs ready to be sent to an LLM. This enables prompt sharing and standardization across applications.

<!-- @entry mcp-error-handling -->

MCP errors follow the JSON-RPC 2.0 error format with `code`, `message`, and optional `data` fields. Standard error codes include `-32700` (Parse error), `-32600` (Invalid request), `-32601` (Method not found), and `-32603` (Internal error). For aggregate requests (`tools/list`), partial failures are handled gracefully — tools from healthy servers are returned while failed servers are omitted, with error details available in response metadata. For targeted requests (`tools/call`), upstream server errors are wrapped in the JSON-RPC error format and returned to the client.
