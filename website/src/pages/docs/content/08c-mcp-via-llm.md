<!-- @entry mcp-via-llm-overview -->

**MCP via LLM** is a client mode that transparently injects MCP tools into standard OpenAI-compatible chat requests. Instead of requiring clients to understand MCP protocol, LocalRouter handles everything server-side — tool injection on the request, tool execution on the response, and automatic conversation looping.

Clients send normal `/v1/chat/completions` requests. LocalRouter augments them with available MCP tools, forwards to the LLM, intercepts any tool calls in the response, executes them via the MCP gateway, and loops until the LLM produces a final text response. The client receives the result as if the LLM answered directly.

**Key benefits:**

- **Zero client-side MCP integration** — any OpenAI-compatible app gets MCP capabilities
- **Server-side tool execution** — MCP tools run locally through the gateway, not on the client
- **Agentic loop** — multi-step tool use handled automatically with configurable iteration limits
- **Mixed tool support** — MCP tools execute server-side while client-defined tools are returned normally

This mode is configured per-client and marked as **Experimental** in the UI.

<!-- @entry mcp-via-llm-how-it-works -->

## How It Works

When a client is configured with the **Both via LLM** mode, every chat completion request goes through an augmented pipeline:

1. **Request arrives** — client sends a standard chat completion with messages (and optionally its own tools)
2. **Tool injection** — LocalRouter fetches available MCP tools from the gateway and converts them to OpenAI function-call format, appending them to the request's tool list
3. **LLM call** — the augmented request is forwarded to the configured provider
4. **Response inspection** — LocalRouter checks if the response contains tool calls
5. **Tool classification** — each tool call is classified as MCP (server-side) or client (pass-through)
6. **Execution & loop** — MCP tool calls are executed via the gateway, results appended to the conversation, and the LLM is called again
7. **Final response** — when no more MCP tool calls remain, the response is returned to the client

<!-- @entry mcp-via-llm-injection -->

### Tool Injection

LocalRouter queries the MCP gateway for all tools available to the client (respecting per-client server restrictions and whitelists). Each MCP tool is converted to an OpenAI-compatible function tool:

```json
{
  "type": "function",
  "function": {
    "name": "github__create_issue",
    "description": "Create a new GitHub issue",
    "parameters": {
      "type": "object",
      "properties": {
        "repo": { "type": "string" },
        "title": { "type": "string" },
        "body": { "type": "string" }
      },
      "required": ["repo", "title"]
    }
  }
}
```

If `expose_resources_as_tools` is enabled, MCP resources are also converted to synthetic read tools. If `inject_prompts` is enabled, MCP prompt content is prepended to the conversation.

The injected tools are merged with any tools the client already included in the request. Tool names are namespaced by MCP server to avoid collisions.

<!-- @entry mcp-via-llm-agentic-loop -->

### Agentic Loop

When the LLM responds with MCP tool calls, LocalRouter enters an agentic loop:

```
Client Request
     ↓
[Inject MCP tools]
     ↓
Call LLM ←──────────────────┐
     ↓                      │
Response has tool calls?    │
     ↓ yes                  │
Classify: MCP or client?    │
     ↓ MCP                  │
Execute via gateway         │
     ↓                      │
Append results to messages ─┘
     ↓ no more MCP calls
Return final response to client
```

The loop continues until:
- The LLM returns a response with **no MCP tool calls** (text-only or client-only tools)
- **Max iterations** reached (default: 4)
- **Timeout** exceeded (default: 300 seconds)

Each iteration appends the tool call and its result as assistant/tool messages, building up the conversation context for the next LLM call.

<!-- @entry mcp-via-llm-mixed-tools -->

### Mixed Tool Execution

When the LLM response contains both MCP tools and client-defined tools, LocalRouter handles them differently:

- **MCP tools** are executed server-side immediately via the gateway
- **Client tools** are returned to the client in the response for client-side handling

When the client sends back its tool results in the next request, LocalRouter merges them with the MCP tool results that were already executed, and continues the agentic loop.

This means clients can define their own tools (e.g., UI interactions, local file access) alongside MCP tools, and everything works seamlessly — MCP execution is invisible to the client.

<!-- @entry mcp-via-llm-sessions -->

## Session Management

Each client using MCP via LLM mode gets a dedicated session that tracks:

- **Conversation history** — all messages including injected tool calls and results
- **MCP gateway session** — a persistent connection to the MCP gateway for tool execution
- **Pending state** — background MCP executions waiting for client tool results

Sessions are identified by client ID and managed by the `McpViaLlmManager`. Key behaviors:

- Sessions are created on first request and reused for subsequent requests
- Each session has a configurable TTL (default: 3600 seconds)
- Expired sessions are cleaned up automatically every 60 seconds
- Maximum concurrent sessions can be limited (default: 100)

<!-- @entry mcp-via-llm-config -->

## Configuration

MCP via LLM is configured globally with per-client overrides available.

<!-- @entry mcp-via-llm-config-options -->

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `session_ttl_seconds` | `3600` | How long a session stays alive after last activity |
| `max_concurrent_sessions` | `100` | Maximum number of parallel sessions |
| `max_loop_iterations` | `4` | Maximum agentic loop iterations per request |
| `max_loop_timeout_seconds` | `300` | Total timeout for the agentic loop |
| `expose_resources_as_tools` | `true` | Convert MCP resources into synthetic read tools |
| `inject_prompts` | `true` | Prepend MCP prompt content to conversations |

<!-- @entry mcp-via-llm-config-per-client -->

### Per-Client Override

Each client can override the global MCP via LLM settings. In the client configuration, set the client mode to **Both via LLM** and adjust:

- Loop iteration and timeout limits
- Whether resources are exposed as tools
- Whether prompts are injected
- Which MCP servers are available (via the standard per-client server whitelist)

Clients using other modes (MCP-only, LLM-only, or standard Both) are unaffected.

<!-- @entry mcp-via-llm-client-modes -->

## Client Modes

LocalRouter supports four client modes that determine how MCP and LLM interact:

| Mode | MCP | LLM | Description |
|------|-----|-----|-------------|
| **LLM Only** | — | Direct | Standard OpenAI proxy, no MCP |
| **MCP Only** | Direct | — | MCP gateway only, no LLM routing |
| **Both** | Direct | Direct | Client handles MCP and LLM separately |
| **Both via LLM** | Server-side | Direct | MCP tools injected into LLM requests transparently |

**Both via LLM** is the only mode where LocalRouter actively mediates between MCP and LLM. In all other modes, the client is responsible for orchestrating tool use.

This mode is ideal for:
- Apps that only support OpenAI protocol but need MCP capabilities
- Reducing client complexity by moving tool orchestration server-side
- Adding MCP tools to existing LLM workflows without code changes
