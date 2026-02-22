<!-- @entry what-are-skills -->

Skills are curated, multi-step workflows that compose multiple MCP tool calls into a single high-level action. While raw MCP tools provide atomic operations (e.g., "read file", "search web"), skills orchestrate sequences of these tools to accomplish complex tasks like "research a topic and create a summary document" or "analyze a codebase and generate a report".

Skills are defined as configurations that specify the sequence of tool calls, input/output mappings, and control flow.

<!-- @entry skills-as-mcp-tools -->

Skills are exposed to LLM clients as regular MCP tools through the gateway. When a client calls `tools/list`, skills appear alongside regular MCP tools with the `localrouter__skill__{skill_name}` naming convention.

This means any LLM that supports MCP tool calling can invoke skills without knowing they're multi-step workflows — the skill execution engine handles the orchestration internally and returns the final result as a standard tool response.

<!-- @entry multi-step-workflows -->

A skill workflow defines an ordered sequence of steps, where each step calls an MCP tool and can reference outputs from previous steps. The execution engine runs steps sequentially, passing data between them via variable references.

Error handling is configurable per-step — a step can be marked as required (workflow fails if it fails) or optional (workflow continues). The final step's output becomes the skill's response to the calling LLM.

<!-- @entry skill-whitelisting -->

Skill access is controlled per-client through the same permission system used for MCP servers. A client's `mcp_server_access` configuration determines which skills are available — since skills are exposed as MCP tools under the `localrouter` virtual server, they can be individually whitelisted or blocked.

This allows administrators to grant different clients access to different skill sets based on their use case and trust level.
