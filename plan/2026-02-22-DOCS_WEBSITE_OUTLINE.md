# Website Documentation Section - Outline

**Date**: 2026-02-22
**Status**: In Progress

## Goal

Add a `/docs` section to the LocalRouter website with a sidebar-navigated outline of all features. Sections contain headings and sub-headings only — no prose content yet. The API Reference sections at the end cover both the OpenAI-compatible gateway and the Unified MCP Gateway endpoints.

## Docs Outline

### 1. Introduction
- What is LocalRouter
- Key concepts (providers, clients, routers, MCP gateway)
- Architecture overview

### 2. Getting Started
- Installation (macOS, Windows, Linux)
- First run
- Configuring your first provider
- Pointing apps to `localhost:3625`

### 3. Clients
- Overview (what is a client)
- Creating client keys
- Authentication methods
  - API key
  - OAuth browser flow
  - STDIO
- Scoped permissions
  - Model restrictions
  - Provider restrictions
  - MCP server restrictions

### 4. Providers
- Supported providers (19)
  - Anthropic, OpenAI, Gemini, Ollama, LMStudio, OpenRouter, Groq, Cerebras, Cohere, DeepInfra, Mistral, Perplexity, TogetherAI, xAI, Generic OpenAI-compatible
- Adding provider API keys
- Provider health checks
  - Circuit breaker
  - Latency tracking
- Feature adapters
  - Prompt caching
  - JSON mode
  - Structured outputs
  - Logprobs

### 5. Model Selection & Routing
- Auto routing (`model: "auto"`)
- RouteLLM classifier (strong/weak)
- Fallback chains
  - Provider failover
  - Offline fallback (Ollama/LMStudio)
- Routing strategies
  - Lowest cost
  - Highest performance
  - Local first
  - Remote first
- Error classification
  - Rate limited → retry different provider
  - Policy violation → fallback
  - Context length exceeded → fallback

### 6. Rate Limiting
- Request rate limits
- Token limits (input/output)
- Cost limits (USD/month)
- Per-key vs per-router limits

### 7. Unified MCP Gateway
- Overview & architecture
- Tool namespacing (`server__tool`)
- Transport types
  - STDIO
  - SSE
  - Streamable HTTP
- Deferred tool loading
- Virtual search tool
- Session management (1-hour TTL)
- Response caching (5-minute TTL)
- Partial failure handling
- MCP OAuth browser authentication
  - OAuth 2.0 + PKCE flow
  - Auto-discovery
  - Token refresh

### 8. Skills
- What are skills
- Skills as MCP tools
- Multi-step workflows
- Per-client skill whitelisting
- Standard MCP interface

### 9. Firewall
- Runtime approval flow
  - Allow once
  - Allow for session
  - Deny
- Request inspection & modification
- Granular approval policies
  - Per-client
  - Per-model
  - Per-MCP server
  - Per-skill

### 10. GuardRails
- Content safety scanning
- Detection types
  - Prompt injection
  - Jailbreak attempts
  - PII leakage
  - Code injection
- Detection sources
  - Built-in rules
  - Microsoft Presidio
  - LLM Guard
- Custom regex rules
- Parallel scanning (zero-latency on clean requests)

### 11. Marketplace
- Overview
- Registry sources
  - Official registry
  - Community registry
  - Private registries
- MCP-exposed search
- Gated installation (approval required)

### 12. Monitoring & Logging
- Access log writer (JSON Lines, daily rotation, 30-day retention)
- In-memory metrics
  - 24-hour time-series (1-minute granularity)
  - Tokens, cost, requests, latency
  - Per-key, per-provider, global
  - Latency percentiles (P50, P95, P99)
- Historical log parser
- Graph data generation

### 13. Configuration
- YAML config structure
- Config file location
  - macOS: `~/Library/Application Support/LocalRouter/`
  - Linux: `~/.localrouter/`
  - Windows: `%APPDATA%\LocalRouter\`
- Config migration
- Environment variables
  - `LOCALROUTER_KEYCHAIN=file`

### 14. Privacy & Security
- Local-only by design
- Zero telemetry
- No cloud sync
- OS keychain storage
- Restrictive CSP
- AGPL-3.0 license

### 15. API Reference: OpenAI Gateway
- `GET /v1/models` — List available models
- `POST /v1/chat/completions` — Chat completions (streaming supported)
- `POST /v1/completions` — Text completions
- `POST /v1/embeddings` — Embeddings
- `GET /health` — Health check
- `GET /openapi.json` — OpenAPI specification
- Authentication (API key via `Authorization: Bearer lr-...`)
- Streaming (SSE format)
- Error responses

### 16. API Reference: MCP Gateway
- `POST /mcp` — Unified MCP endpoint
- Tool namespacing convention
- Session lifecycle
- Authentication
- Supported MCP methods
  - `tools/list`
  - `tools/call`
  - `resources/list`
  - `resources/read`
  - `prompts/list`
  - `prompts/get`
- Error handling & partial failures

## Implementation Plan

### Files to Create
- `website/src/pages/Docs.tsx` — Main docs page with sidebar + content area

### Files to Modify
- `website/src/App.tsx` — Add `/docs` route
- `website/src/components/Navigation.tsx` — Add "Docs" link
- `website/src/components/Footer.tsx` — Update documentation link to `/docs`

### Approach
- Single-page docs with sidebar navigation using anchor links
- Sidebar lists all 16 sections; clicking scrolls to that section
- Each section shows heading + sub-headings with "Coming soon" placeholder
- Matches existing site styling (Tailwind, dark mode support)
- Mobile-responsive with collapsible sidebar
