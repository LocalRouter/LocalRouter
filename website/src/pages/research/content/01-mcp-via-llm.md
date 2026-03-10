<!-- @entry mcp-via-llm-abstract -->

**MCP via LLM** introduces a fourth client mode that transparently injects MCP server tools into LLM API requests and executes them server-side. The client speaks only the standard OpenAI chat completions protocol — no MCP SDK, no tool handling code, no agentic loop. The gateway becomes an autonomous intermediary that orchestrates tool calls on the client's behalf, enabling any OpenAI-compatible application to gain MCP capabilities without modification.

<!-- @entry mcp-via-llm-problem -->

Large Language Model applications increasingly rely on tool use for interacting with external systems. The Model Context Protocol (MCP) provides a standard for tool discovery and invocation, but integrating MCP into existing applications requires significant client-side changes: installing an MCP SDK, implementing tool call dispatch logic, and managing multi-turn agentic loops.

This creates a chicken-and-egg problem. Most existing applications speak the OpenAI chat completions protocol. To benefit from MCP tools, they would need to be rewritten — yet the value proposition of MCP only materializes once applications can actually use it.

**The key question**: Can we make MCP tools available to any OpenAI-compatible client *without requiring any client-side changes*?

<!-- @entry mcp-via-llm-approach -->

The approach moves the agentic loop from client to server. When a client is configured in `McpViaLlm` mode, the gateway intercepts `/v1/chat/completions` requests and:

1. **Discovers** all available MCP tools via the Unified MCP Gateway
2. **Injects** them as standard function tools into the LLM request
3. **Forwards** the augmented request to the upstream LLM provider
4. **Classifies** the response: does the LLM want to call tools?
5. **Executes** MCP tool calls server-side, appends results to the conversation history, and loops back to step 3
6. **Returns** the final text response (or client-directed tool calls) to the original caller

The client sees a normal chat completion response. It never knows tools were involved.

```
Client                    Gateway (MCP via LLM)              LLM Provider
  |                              |                               |
  |-- POST /chat/completions --> |                               |
  |                              |-- Inject MCP tools ---------->|
  |                              |<- tool_calls: [mcp_tool_A] --|
  |                              |                               |
  |                              |-- Execute mcp_tool_A -------->| (MCP Server)
  |                              |-- Append result, re-call ---->| (LLM)
  |                              |<- Final text response --------|
  |                              |                               |
  |<--- Text response ----------|                               |
```

<!-- @entry mcp-via-llm-mixed -->

A particularly novel aspect of the design is **mixed tool execution** — handling responses where the LLM wants to call both MCP tools (server-executable) and client-defined tools (that only the client can execute).

**The problem**: Client-defined tools may have side effects (sending an email, making a payment) and cannot be retried. MCP tools are safe to execute in the background. When the LLM returns both types in a single response, naively executing MCP tools first would block the client from executing its own tools promptly.

**The solution**: Parallel background execution with history reconstruction.

```
LLM returns: [mcp_tool_A, client_tool_B, mcp_tool_C]

1. Spawn background tasks for mcp_tool_A and mcp_tool_C
2. Return ONLY client_tool_B to the client immediately
3. Client executes client_tool_B, sends result back
4. Gateway detects this is a "mixed resume":
   - Await background MCP task handles
   - Reconstruct full message history in original tool call order
   - Merge all results (MCP + client)
5. Continue agentic loop with reconstructed history
```

Background task handles are stored in a `PendingMixedExecution` struct with a `Drop` implementation that automatically aborts any still-running tasks if the session expires, preventing resource leaks.

<!-- @entry mcp-via-llm-streaming -->

Streaming support required solving a multi-segment streaming problem. The agentic loop may iterate multiple times (tool call, execute, re-call), yet the client expects a single coherent SSE stream.

**Multi-segment streaming** works by:

- Streaming each agentic loop iteration as a segment of the same SSE connection
- Naturally pausing between segments during tool execution (the client sees no data during this window)
- Suppressing intermediate `finish_reason: "tool_calls"` when all tools are MCP-only (the client shouldn't know about internal tool use)
- Maintaining SSE keepalive during tool execution to prevent connection timeouts

The result is that the client sees a single streaming response that may have natural pauses (while tools execute), followed by more streaming content — indistinguishable from a slow but continuous generation.

<!-- @entry mcp-via-llm-synthesis -->

Beyond tools, the gateway synthesizes other MCP primitives into the tool format:

**Resources as tools**: MCP resources (file contents, API data) are exposed as synthetic `mcp_resource__*` tools. Each is a zero-argument function that the LLM can call to fetch the resource content on demand, enabling lazy resource loading without client involvement.

**Prompts as system messages**: MCP prompts with no arguments are automatically injected as system messages in every request. Parameterized prompts are exposed as `mcp_prompt__*` synthetic tools with JSON Schema built from the prompt's argument definitions, allowing the LLM to invoke them with parameters.

<!-- @entry mcp-via-llm-sessions -->

Each client gets its own session tracking conversation history, including injected tool calls and results that the client never sees. This hidden history ensures multi-turn conversations maintain context about previously executed MCP tools.

Sessions use TTL-based cleanup (default: 60 minutes) with a background task running every 60 seconds to evict expired sessions.

**Usage aggregation** across loop iterations provides transparency. The response includes an `extensions.mcp_via_llm` metadata block:

```json
{
  "iterations": 3,
  "mcp_tools_called": ["filesystem__read_file", "github__search"],
  "total_prompt_tokens": 2145,
  "total_completion_tokens": 892
}
```

<!-- @entry mcp-via-llm-results -->

The implementation achieves complete transparency — any application that can call `/v1/chat/completions` automatically gains access to all configured MCP tools without code changes. The approach has been validated with:

- **Standard chat clients** (e.g., ChatGPT-compatible UIs) gaining filesystem access, GitHub integration, and database queries through MCP servers
- **CLI tools** executing multi-step workflows involving multiple MCP servers
- **Existing agent frameworks** benefiting from additional tool capabilities without SDK changes

| Component | Lines of Code | Purpose |
|-----------|:---:|---------|
| orchestrator.rs | 965 | Core agentic loop + tool execution |
| orchestrator_stream.rs | 860 | Multi-segment streaming adapter |
| gateway_client.rs | 493 | Typed MCP gateway wrapper |
| manager.rs | 424 | Session management + dispatch |
| session.rs | 97 | Session + pending state models |
| **Total** | **2,839** | Full implementation (excl. tests) |

The agentic loop is configurable per-client: max iterations (default: 10), loop timeout (default: 5 minutes), resource/prompt synthesis toggles, and per-client session TTL.
