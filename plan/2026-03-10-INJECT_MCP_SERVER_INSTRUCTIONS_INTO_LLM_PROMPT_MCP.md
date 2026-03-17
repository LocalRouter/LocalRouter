# Inject MCP Server Instructions into LLM Prompt (MCP via LLM)

## Context

When an MCP server initializes, it can return an `instructions` field (and `serverInfo.description`) describing how to use its tools and resources. Currently, the MCP gateway already extracts and stores these in `InstructionsContext` / `MergedCapabilities`, and returns them in the JSON-RPC `initialize` response. However, the MCP via LLM orchestrator **discards** the initialize response and never injects these instructions into the LLM's prompt.

Claude Code handles this by injecting a `# MCP Server Instructions` system prompt section with per-server `## ServerName\n{instructions}` blocks.

## Approach

**Extract instructions from the gateway's `InstructionsContext` after initialization, and inject them as a system message into the LLM request** — following the same pattern as `inject_prompt_messages`.

### Option: Use `InstructionsContext` directly (chosen)

Rather than parsing the JSON initialize response, use the gateway's existing `get_session_instructions_context()` API which returns `InstructionsContext` with per-server `instructions` and `description` fields. This avoids duplicating formatting logic and gives us structured data.

## Files to Modify

### 1. `crates/lr-mcp-via-llm/src/gateway_client.rs`
- Add a `get_server_instructions()` method that calls `gateway.get_session_instructions_context(session_key)` and returns a Vec of `(name, instructions, description)` tuples for servers that have non-empty instructions or descriptions.

### 2. `crates/lr-mcp-via-llm/src/orchestrator.rs`
- Add `inject_server_instructions(request, server_instructions)` function:
  - Formats instructions similar to Claude Code: `# MCP Server Instructions\n\n## ServerName\n{description}\n{instructions}`
  - Injects as a system message at the same position as prompt messages (before first non-system message)
- Call it in `run_agentic_loop()` right after `gw_client.initialize()` (line ~111), since instructions are only available after initialization

### 3. `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`
- Same injection call after `gw_client.initialize()` in the streaming orchestrator (line ~65)

### 4. `crates/lr-config/src/types.rs`
- Add `inject_server_instructions: bool` field to `McpViaLlmConfig` (default: `true`)
- Update `Default` impl

### 5. `crates/lr-config/src/migration.rs`
- Add migration for the new config field

### 6. Tests
- Add unit test for `inject_server_instructions()` in `crates/lr-mcp-via-llm/src/tests.rs`
- Verify instructions are injected as system message before user messages
- Verify empty instructions produce no injection

## Implementation Details

### Formatting (following Claude Code pattern)
```
# MCP Server Instructions

The following MCP servers have provided instructions for how to use their tools and resources:

## my-filesystem
A filesystem access server for reading and writing files.

Use the read_file tool to read files. Always confirm before writing.

## my-database
Query the database using SQL. Never run DELETE or DROP statements.
```

- Include `description` first (if present), then `instructions` (if present)
- Skip servers with neither
- If no servers have instructions, don't inject anything

### Injection point
After `gw_client.initialize()` and before tool injection, since instructions provide context for how to use the tools.

## Verification

1. `cargo test -p lr-mcp-via-llm` — unit tests pass
2. `cargo clippy` — no warnings
3. Manual: configure an MCP server with instructions, send a request via MCP-via-LLM, verify the system message appears in the request to the LLM provider (check logs)
