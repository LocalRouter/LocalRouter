<!-- @entry memory-overview -->

**Experimental.** The Memory System provides persistent conversation memory for LLM sessions using Zillis memsearch, a markdown-first memory engine with hybrid vector search. When enabled for a client, conversations are automatically recorded and indexed so that LLMs can recall relevant context from past sessions via semantic search.

Memory is configured globally (embedding provider, session timeouts, compaction settings) but must be explicitly enabled per client. There is no global toggle that enables memory for all clients — each client must individually opt in via `memory_enabled`.

<!-- @entry memory-architecture -->

Each client with memory enabled gets an isolated directory under `~/.localrouter/memory/{client_id}/` with the following structure:

- `sessions/` — Watched by a per-client `memsearch watch` daemon. Contains active session transcripts and compacted summaries that are indexed for search.
- `archive/` — Not watched by memsearch. Stores raw transcripts permanently after compaction, enabling re-compaction if the LLM provider or settings change.
- `.memsearch/` — Internal memsearch state (Milvus Lite database). Managed automatically.

A `.memsearch.toml` configuration file is generated per client, specifying the embedding provider settings.

<!-- @entry memory-modes -->

Memory auto-capture is supported in two client modes:

- **McpViaLlm** — Each MCP via LLM session serves as a natural conversation boundary. User and assistant exchanges are automatically appended to the session transcript after each turn in the orchestrator.
- **Both** — Conversations are detected via message hash prefix matching on the stateless OpenAI chat API. The system computes hashes for each incoming message and checks if previously stored hashes form a prefix of the new sequence, identifying continuations versus new conversations.

Memory is not supported for `LlmOnly` clients (no MCP means no `MemoryRecall` tool exposure) or `McpOnly` clients (no conversation content is visible to the proxy).

<!-- @entry memory-recall-tool -->

The `MemoryRecall` virtual MCP tool allows LLMs to search past conversation memories. It is exposed through the `_memory` virtual MCP server and performs semantic search scoped to the requesting client's indexed sessions.

The tool name is configurable (default: `MemoryRecall`) via the `recall_tool_name` setting. Search uses a two-layer progressive disclosure approach: an initial `memsearch search` retrieves relevant chunks, then `memsearch expand` fetches the full markdown section around top results for richer context. Results are returned with source labels and relevance scores, or a "no relevant memories found" message if the index is empty.

<!-- @entry memory-sessions -->

Conversations are grouped into sessions based on temporal proximity. A session starts on first activity from a client when no active session exists, and ends when either the inactivity timeout (default: 3 hours) or the maximum session duration (default: 8 hours) is exceeded. These thresholds are checked by a background monitor task that runs every 60 seconds.

Within a session, multiple conversations are recorded in a single markdown file separated by `# Conversation N (timestamp)` headers. In McpViaLlm mode, each MCP via LLM session ID maps to one conversation. In Both mode, conversation boundaries are detected by message hash prefix matching across successive chat completion requests.

<!-- @entry memory-compaction -->

When a session expires, LLM-powered compaction can optionally summarize the transcript. The `memsearch compact` command is invoked with a configurable LLM provider, producing a summary that is written to `sessions/{id}-summary.md`. The original transcript is then moved to `archive/{id}.md`, removing it from the watched directory.

Memsearch detects the deletion of the original file (dropping old chunks from the index) and the creation of the summary file (indexing the compact representation). Both the raw transcript and the summary are kept permanently, enabling re-compaction if a compacted conversation is resumed or if the user changes their compaction LLM settings.

<!-- @entry memory-privacy -->

**Privacy warning:** When memory is enabled for a client, full conversation transcripts are recorded and stored locally on disk. This includes all user messages and assistant responses for every exchange in that client's sessions.

The UI provides a link to open the memory folder (`~/.localrouter/memory/`) in the system file manager so users can review, inspect, or delete stored memories at any time. Memory must be consciously enabled per client to ensure users opt in to conversation recording for each client individually.

<!-- @entry memory-config -->

Memory is configured in the global settings and enabled per client. Key configuration options:

| Setting | Default | Description |
|---------|---------|-------------|
| `embedding` | `onnx` | Embedding provider: built-in ONNX bge-m3 (~558MB auto-download) or Ollama with a specified model |
| `recall_tool_name` | `MemoryRecall` | Name of the virtual MCP tool exposed to LLMs |
| `search_top_k` | 5 | Number of search results returned by MemoryRecall |
| `session_inactivity_minutes` | 180 | Inactivity timeout before a session expires (3 hours) |
| `max_session_minutes` | 480 | Maximum session duration before forced expiry (8 hours) |
| `compaction` | disabled | Optional LLM summarization at session end, requiring an `llm_provider` and optional `llm_model` |

Per-client enablement is controlled by the `memory_enabled` field on each client. It defaults to disabled (`None`); set to `true` to enable memory recording and the MemoryRecall tool for that client.
