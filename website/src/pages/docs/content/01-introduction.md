<!-- @entry what-is-localrouter -->

LocalRouter is a local, privacy-first API gateway that runs on your machine and provides a single OpenAI-compatible endpoint at `localhost:3625`. It routes requests across 19+ LLM providers (OpenAI, Anthropic, Gemini, Ollama, LM Studio, and more), proxies MCP servers through a unified gateway, and supports intelligent model selection with the built-in RouteLLM classifier. All data stays on your machine — zero telemetry, zero external assets, and secrets stored in your OS keychain.

<!-- @entry key-concepts -->

**Clients** are API consumers that authenticate with LocalRouter via API keys (`lr-*` prefix) or OAuth tokens. Each client references a **Strategy** that defines which models, providers, and MCP servers it can access, along with rate limits. **Providers** are upstream LLM services (cloud or local) that LocalRouter routes requests to. **MCP Servers** are tool providers connected via STDIO, SSE, or Streamable HTTP transports — LocalRouter merges them behind a single `/mcp` endpoint with automatic tool namespacing (`server__tool`). **Skills** are curated multi-step workflows exposed as MCP tools that compose multiple tool calls into a single action.

<!-- @entry architecture-overview -->

LocalRouter is built with a Rust/Tauri 2.x backend and a React/TypeScript frontend. The backend runs an Axum HTTP server on port 3625 that exposes OpenAI-compatible endpoints (`/v1/chat/completions`, `/v1/models`, etc.) and MCP proxy endpoints (`/mcp`). The **routing engine** receives incoming requests, authenticates the client, checks rate limits, selects a model (via auto-routing or explicit model ID), and dispatches to the appropriate provider. The **MCP gateway** manages connections to upstream MCP servers, namespaces their tools, and proxies JSON-RPC calls. The **monitoring system** collects per-request metrics (latency, tokens, cost) in memory with time-series bucketing. The frontend communicates with the backend via Tauri IPC commands for configuration, while API consumers interact through the HTTP server.
