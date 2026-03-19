<!-- @entry mcp-via-llm-overview -->

**MCP via LLM** is a client mode that transparently injects MCP server capabilities into standard OpenAI-compatible chat requests. Instead of requiring clients to understand MCP protocol, LocalRouter handles everything server-side — context injection, tool injection, tool execution, and automatic conversation looping.

Clients send normal `/v1/chat/completions` requests. LocalRouter augments them with MCP server instructions and tools, forwards to the LLM, intercepts any tool calls in the response, executes them via the MCP gateway, and loops until the LLM produces a final text response. The client receives the result as if the LLM answered directly.

**Key benefits:**

- **Zero client-side MCP integration** — any OpenAI-compatible app gets MCP capabilities
- **Server-side tool execution** — MCP tools run locally through the gateway, not on the client
- **Agentic loop** — multi-step tool use handled automatically with configurable iteration limits
- **Mixed tool support** — MCP tools execute server-side while client-defined tools are returned normally

> **Note:** This mode only works with the **Chat Completions API** (`/v1/chat/completions`). The legacy Completions API (`/v1/completions`) is not supported because it uses a single prompt string with no messages, tool calling, or system message support.

This mode is configured per-client and marked as **Experimental** in the UI.

<!-- @entry mcp-via-llm-how-it-works -->

## How It Works

When a client is configured with the **Both via LLM** mode, every chat completion request goes through an augmented pipeline:

1. **Request arrives** — client sends a standard chat completion with messages (and optionally its own tools)
2. **Gateway initialization** — on the first request, LocalRouter initializes the MCP gateway session, connecting to all configured MCP servers
3. **Instructions injection** — the unified gateway instructions (server descriptions, tool listings, and server-provided instructions) are injected as a system message
4. **Tool injection** — MCP tools are converted to OpenAI function-call format and appended to the request's tool list
5. **Resource & prompt injection** — MCP resources and prompts are optionally injected (see below)
6. **LLM call** — the augmented request is forwarded to the configured provider
7. **Response inspection** — LocalRouter checks if the response contains tool calls
8. **Tool classification** — each tool call is classified as MCP (server-side) or client (pass-through)
9. **Execution & loop** — MCP tool calls are executed via the gateway, results appended to the conversation, and the LLM is called again
10. **Final response** — when no more MCP tool calls remain, the response is returned to the client

<!-- @entry mcp-via-llm-request-augmentation -->

## Request Augmentation

LocalRouter augments the client's request before it reaches the LLM. This happens in a specific order to ensure consistent placement of injected content.

<!-- @entry mcp-via-llm-instructions -->

### Gateway Instructions

On the first request, the MCP gateway builds a **unified instructions document** describing all connected MCP servers, their capabilities, and any server-provided instructions. This is injected as a **system message** placed after all existing system messages but before the first non-system message.

For example, if the client sends:

```
[system] "You are a helpful assistant"
[user]   "Read the README file"
```

After injection the LLM sees:

```
[system] "You are a helpful assistant"
[system] "Unified MCP Gateway. Tools from MCP servers are namespaced with a `servername__` prefix. ..."
[user]   "Read the README file"
```

The gateway instructions include:

- **Server descriptions** — from each MCP server's `serverInfo.description` field
- **Server instructions** — from each MCP server's `instructions` field (the "welcome message")
- **Tool, resource, and prompt listings** — namespaced capabilities for each server
- **Unavailable servers** — servers that failed to connect, with error messages

This gives the LLM full context about what tools are available and how to use them, following the same pattern used by tools like Claude Code.

<!-- @entry mcp-via-llm-tool-injection -->

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

Tool names are namespaced by MCP server (e.g., `github__create_issue`) to avoid collisions across servers. Injected tools are merged with any tools the client already included in the request. If a client tool has the same name as an MCP tool, the MCP tool takes precedence.

<!-- @entry mcp-via-llm-resource-injection -->

### Resource Injection

When `expose_resources_as_tools` is enabled (default: `true`), a single `ResourceRead` tool is injected. The LLM can call it with a resource name (listed in the gateway instructions) to fetch content. This tool also supports reading skill files using the `<skill>/<path>` pattern.

<!-- @entry mcp-via-llm-prompt-injection -->

### Prompt Injection

When `inject_prompts` is enabled (default: `true`), MCP prompts are handled in two ways:

- **No-argument prompts** are resolved immediately and injected as system messages before the first non-system message
- **Parameterized prompts** are exposed as synthetic tools (prefixed with `mcp_prompt__`) that the LLM can call with arguments

<!-- @entry mcp-via-llm-agentic-loop -->

## Agentic Loop

When the LLM responds with MCP tool calls, LocalRouter enters an agentic loop:

```
Client Request
     ↓
[Inject instructions, tools, resources, prompts]
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

- **MCP tools** are executed server-side immediately via the gateway (in background tasks)
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

<!-- @entry mcp-via-llm-api-support -->

## API Support

| Endpoint | Supported | Notes |
|----------|-----------|-------|
| `POST /v1/chat/completions` | Yes | Full support including streaming |
| `POST /v1/completions` | No | Legacy endpoint uses a prompt string with no messages or tool calling support |
| `POST /mcp/*` | N/A | Direct MCP access, not applicable |

Clients using the MCP via LLM mode are blocked from the legacy Completions API and will receive a `403 Forbidden` response.

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
| `expose_resources_as_tools` | `true` | Inject a `ResourceRead` tool for fetching MCP resources |
| `inject_prompts` | `true` | Inject MCP prompt content into conversations |

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
