<!-- @entry what-are-skills -->

Skills are curated, multi-step workflows that compose multiple MCP tool calls into a single high-level action. While raw MCP tools provide atomic operations (e.g., "read file", "search web"), skills orchestrate sequences of these tools to accomplish complex tasks like "research a topic and create a summary document" or "analyze a codebase and generate a report".

<!-- @entry skills-as-mcp-tools -->

Skills appear as regular MCP tools through the gateway. When a client calls `tools/list`, skills are listed alongside regular MCP tools with the naming convention `localrouter__skill__{skill_name}`.

Any LLM that supports MCP tool calling can invoke skills without knowing they're multi-step workflows — the execution engine handles the orchestration and returns the final result as a standard tool response.

<!-- @entry multi-step-workflows -->

A skill workflow defines an ordered sequence of steps, where each step calls an MCP tool and can reference outputs from previous steps. Steps run sequentially, passing data between them automatically.

Each step can be marked as required (workflow fails if it fails) or optional (workflow continues). The final step's output becomes the skill's response.

<!-- @entry skill-whitelisting -->

Skill access is controlled per-client through the same permission system used for MCP servers. Since skills are exposed as MCP tools under the `localrouter` virtual server, they can be individually whitelisted or blocked in the client's MCP server access settings.
