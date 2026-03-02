<!-- @entry what-is-localrouter -->

LocalRouter is a local, privacy-first API gateway that runs on your machine and provides a single OpenAI-compatible endpoint at `localhost:3625`. It routes requests across 19+ LLM providers (OpenAI, Anthropic, Gemini, Ollama, LM Studio, and more), proxies MCP servers through a unified gateway, and supports intelligent model selection with the built-in RouteLLM classifier.

All data stays on your machine — zero telemetry, zero external assets, and secrets stored in your OS keychain.

<!-- @entry key-concepts -->

**Clients** are API consumers that authenticate with LocalRouter via API keys (`lr-*` prefix) or OAuth tokens. Each client references a **Strategy** that defines which models, providers, and MCP servers it can access, along with rate limits.

**Providers** are upstream LLM services (cloud or local) that LocalRouter routes requests to.

**MCP Servers** are tool providers connected via STDIO, SSE, or Streamable HTTP transports — LocalRouter merges them behind a single endpoint with automatic tool namespacing (`server__tool`).

**Skills** are curated multi-step workflows exposed as MCP tools that compose multiple tool calls into a single action.

<!-- @entry architecture-overview -->

LocalRouter runs an HTTP server on port 3625 that serves two gateways at the same root path:

**OpenAI-compatible gateway.** Exposes standard endpoints (`/chat/completions`, `/models`, `/embeddings`, etc.) that any OpenAI-compatible client can use. The routing engine authenticates the client, checks rate limits, selects a model (via auto-routing or explicit model ID), and dispatches to the appropriate provider.

**MCP gateway.** Manages connections to upstream MCP servers, namespaces their tools, and proxies JSON-RPC calls. Clients connect once and gain access to all configured MCP servers.

**Monitoring.** Collects per-request metrics (latency, tokens, cost) viewable from the dashboard.

The UI is a native desktop window for managing configuration, while API consumers interact through the HTTP server.
