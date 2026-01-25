<h1 align="center">LocalRouter</h1>

<p align="center">
  <strong>One local API for all your LLMs and MCP servers.</strong>
</p>

<p align="center">
  A privacy-first desktop app that acts as a unified gateway to your AI providers and tools.<br/>
  Manage credentials in one place. Route requests intelligently. Keep everything local.
</p>

<p align="center">
  <a href="https://localrouter.ai">Website</a> &bull;
  <a href="#installation">Install</a> &bull;
  <a href="#quick-start">Quick Start</a> &bull;
  <a href="#features">Features</a> &bull;
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License" /></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.75+-orange.svg" alt="Rust" /></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/tauri-2.x-24C8D8.svg" alt="Tauri" /></a>
  <a href="https://github.com/LocalRouter/LocalRouter/releases"><img src="https://img.shields.io/github/v/release/LocalRouter/LocalRouter?label=release" alt="Release" /></a>
</p>

<br/>

---

## Why LocalRouter?

Modern AI development means juggling multiple providers, API keys scattered across config files, and no easy way to control which apps access what. LocalRouter solves this:

- **One endpoint** &mdash; Point all your apps to `localhost:3625` instead of managing different provider URLs
- **One credential vault** &mdash; Store all API keys securely in your OS keychain, not in plaintext configs
- **Granular access control** &mdash; Give each app access to only the models and tools it needs
- **Intelligent routing** &mdash; Automatically route requests based on complexity, cost, or availability
- **Works offline** &mdash; Fall back to local models when you're not connected

```
┌─────────────────┐      ┌─────────────────┐      ┌─────────────────┐
│     Cursor      │      │                 │      │     OpenAI      │
│    OpenCode     │─────▶│  LocalRouter    │─────▶│    Anthropic    │
│   Open WebUI    │      │  localhost:3625 │      │     Ollama      │
│     Cline       │      │                 │      │     Gemini      │
└─────────────────┘      └─────────────────┘      └─────────────────┘
                                 │
                                 ▼
                         ┌─────────────────┐
                         │   MCP Servers   │
                         │  GitHub, Jira   │
                         │  Slack, Files   │
                         └─────────────────┘
```

---

## Installation

### Download

Get the latest release for your platform:

| Platform | Download |
|----------|----------|
| **macOS** (Intel & Apple Silicon) | [LocalRouter.dmg](https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_macOS.dmg) |
| **Windows** (64-bit) | [LocalRouter.msi](https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Windows.msi) |
| **Linux** (DEB) | [LocalRouter.deb](https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.deb) |
| **Linux** (RPM) | [LocalRouter.rpm](https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.rpm) |

[View all releases](https://github.com/LocalRouter/LocalRouter/releases)

### Build from Source

```bash
# Prerequisites: Rust 1.75+, Node.js 18+
git clone https://github.com/LocalRouter/LocalRouter.git
cd LocalRouter
cargo tauri dev
```

---

## Quick Start

**1. Launch LocalRouter** &mdash; The app starts and runs an API server at `localhost:3625`

**2. Add a provider** &mdash; Go to Providers tab, add OpenAI/Anthropic/Ollama with your API key

**3. Create an access key** &mdash; Go to API Keys tab, create a key (starts with `lr-`)

**4. Use it** &mdash; Point any OpenAI-compatible app to LocalRouter:

```bash
curl http://localhost:3625/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-key-here" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### Using with SDKs

```python
# Python (OpenAI SDK)
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:3625/v1",
    api_key="lr-your-key-here"
)

response = client.chat.completions.create(
    model="gpt-4",
    messages=[{"role": "user", "content": "Hello!"}]
)
```

```typescript
// TypeScript (OpenAI SDK)
import OpenAI from 'openai';

const client = new OpenAI({
  baseURL: 'http://localhost:3625/v1',
  apiKey: 'lr-your-key-here',
});

const response = await client.chat.completions.create({
  model: 'gpt-4',
  messages: [{ role: 'user', content: 'Hello!' }],
});
```

---

## Features

### Credential Management

Stop scattering API keys across config files. LocalRouter stores all credentials in your OS keychain and lets you control exactly what each app can access.

- **Multiple auth methods** &mdash; API keys, OAuth 2.0, or STDIO for local tools
- **Per-client permissions** &mdash; Assign specific models and MCP servers to each client
- **Secure by default** &mdash; Secrets stored in macOS Keychain, Windows Credential Manager, or Linux Secret Service

### Intelligent Routing

Never worry about provider outages or rate limits again.

- **Complexity-based routing** &mdash; Send complex requests to powerful models, simple ones to fast/cheap models
- **Automatic failover** &mdash; Route to secondary providers when primary is unavailable
- **Offline fallback** &mdash; Seamlessly switch to local Ollama models when offline
- **Cost optimization** &mdash; Configure routing rules to minimize spend

### Unified MCP Gateway

Connect once to LocalRouter, access all your MCP tools.

- **Single endpoint** &mdash; One MCP connection exposes tools from all your configured servers
- **Per-client access** &mdash; Control which MCP servers each client can use
- **STDIO & SSE transports** &mdash; Works with local processes and remote HTTP servers
- **OAuth support** &mdash; Auto-discovery and custom auth headers for cloud MCP servers

### Supported Providers

| Provider | Chat | Completions | Embeddings | Streaming |
|----------|:----:|:-----------:|:----------:|:---------:|
| OpenAI | ✅ | ✅ | ✅ | ✅ |
| Anthropic | ✅ | ✅ | - | ✅ |
| Google Gemini | ✅ | ✅ | ✅ | ✅ |
| Ollama | ✅ | ✅ | ✅ | ✅ |
| OpenRouter | ✅ | ✅ | ✅ | ✅ |
| Mistral | ✅ | ✅ | ✅ | ✅ |
| Cohere | ✅ | - | ✅ | ✅ |
| Perplexity | ✅ | - | - | ✅ |
| Groq | ✅ | ✅ | - | ✅ |
| LM Studio | ✅ | ✅ | ✅ | ✅ |
| Any OpenAI-compatible | ✅ | ✅ | ✅ | ✅ |

### Privacy First

LocalRouter runs entirely on your machine.

- **No telemetry** &mdash; Zero analytics, crash reporting, or usage tracking
- **No cloud sync** &mdash; All data stays on your computer
- **No external assets** &mdash; Everything bundled at build time, no CDN requests
- **Open source** &mdash; Audit the code yourself

---

## API Reference

LocalRouter exposes an OpenAI-compatible API at `http://localhost:3625`.

| Endpoint | Description |
|----------|-------------|
| `POST /v1/chat/completions` | Chat completions (streaming supported) |
| `POST /v1/completions` | Text completions |
| `POST /v1/embeddings` | Generate embeddings |
| `GET /v1/models` | List available models |
| `POST /mcp/*` | MCP server proxy |
| `POST /oauth/token` | OAuth token endpoint |
| `GET /openapi.json` | OpenAPI 3.1 specification |
| `GET /health` | Health check |

Interactive API documentation is available in the app's Documentation tab.

---

## Configuration

Configuration is stored in platform-specific directories:

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Application Support/LocalRouter/` |
| Linux | `~/.localrouter/` |
| Windows | `%APPDATA%\LocalRouter\` |

Most configuration is done through the UI, but you can also edit `config.yaml` directly:

```yaml
providers:
  - id: openai-main
    provider_type: OpenAI
    name: "OpenAI"
    api_key_ref: "openai-key"
    enabled: true

  - id: ollama-local
    provider_type: Ollama
    name: "Local Ollama"
    base_url: "http://localhost:11434"
    enabled: true

mcp_servers:
  - id: github
    name: "GitHub"
    transport: SSE
    url: "https://api.github.com/mcp"
    oauth_provider: github
```

---

## Architecture

LocalRouter is built with Rust (backend) and React (frontend) using Tauri 2.x for the desktop shell.

```
src-tauri/src/
├── server/         # Axum web server, OpenAI-compatible API
├── providers/      # Provider implementations, feature adapters
├── router/         # Request routing, rate limiting
├── mcp/            # MCP proxy (STDIO, SSE transports)
├── monitoring/     # Metrics collection, dashboards
├── config/         # YAML config, validation
└── ui/             # Tauri command handlers

src/
├── components/     # React UI components
└── views/          # Main application views
```

See [CLAUDE.md](CLAUDE.md) for detailed architecture documentation.

---

## Contributing

We welcome contributions! LocalRouter is actively developed and there's plenty to do.

### Getting Started

```bash
# Clone and run
git clone https://github.com/LocalRouter/LocalRouter.git
cd LocalRouter
cargo tauri dev

# Run tests
cargo test

# Lint and format
cargo clippy && cargo fmt
```

### Development Tips

- Use `LOCALROUTER_KEYCHAIN=file` to avoid macOS keychain prompts during development
- Dev mode uses `~/.localrouter-dev/` to keep it separate from production
- Check [plan/2026-01-14-PROGRESS.md](plan/2026-01-14-PROGRESS.md) for available tasks

### Commit Convention

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(router): add complexity-based routing
fix(providers): handle rate limit errors gracefully
docs: update API reference
```

---

## Roadmap

### Completed

- OpenAI-compatible API (chat, completions, embeddings)
- 10+ provider implementations
- MCP proxy with STDIO and SSE transports
- OAuth 2.0 authentication
- Rate limiting (requests, tokens, cost)
- Desktop UI with system tray
- OpenAPI 3.1 documentation

### In Progress

- Smart routing engine (complexity-based, cost optimization)
- Unified client architecture
- UI improvements

### Planned

- CLI tool for headless operation
- Docker container
- Custom routing strategies
- Webhook notifications
- Plugin system for custom providers

---

## License

LocalRouter is licensed under the [GNU Affero General Public License v3.0](LICENSE).

This means you can use, modify, and distribute the software freely. If you modify LocalRouter and run it as a network service, you must make your modifications available under the same license.

---

## Links

- **Website**: [localrouter.ai](https://localrouter.ai)
- **Releases**: [GitHub Releases](https://github.com/LocalRouter/LocalRouter/releases)
- **Issues**: [GitHub Issues](https://github.com/LocalRouter/LocalRouter/issues)
- **Discussions**: [GitHub Discussions](https://github.com/LocalRouter/LocalRouter/discussions)

---

<p align="center">
  <sub>Built with Rust, Tauri, Axum, and React</sub>
</p>
