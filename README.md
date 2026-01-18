# LocalRouter AI

<div align="center">

**A powerful, local-first OpenAI-compatible API gateway with intelligent routing, multi-provider support, and comprehensive monitoring.**

[![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/tauri-2.x-24C8D8.svg)](https://tauri.app/)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](plan/2026-01-14-CONTRIBUTING.md)

[Features](#-features) • [Quick Start](#-quick-start) • [Documentation](#-documentation) • [Contributing](#-contributing)

</div>

---

## 🎯 What is LocalRouter AI?

LocalRouter AI is a **cross-platform desktop application** that acts as an intelligent proxy between your AI applications and multiple AI providers. It provides a single, unified OpenAI-compatible API that can route requests to the best available provider based on cost, performance, and availability.

**Perfect for:**
- 🚀 **Developers** building AI-powered applications who want flexibility in provider selection
- 💰 **Cost-conscious users** who want to optimize AI spending across providers
- 🔒 **Privacy-focused individuals** who want to run local models alongside cloud providers
- 🛠️ **Teams** who need centralized API key management and monitoring

## ✨ Features

### Core Capabilities

- **🔌 OpenAI-Compatible API**: Drop-in replacement for OpenAI API - just change the base URL
- **🌐 Multi-Provider Support**: Connect to 10+ AI providers simultaneously
  - Local: Ollama, LM Studio, any OpenAI-compatible server
  - Cloud: OpenAI, Anthropic, Google Gemini, OpenRouter, Mistral, Cohere, Perplexity
- **🧠 Smart Routing**: Intelligent request routing based on:
  - Cost optimization (use cheaper models when possible)
  - Performance requirements (latency, throughput)
  - Availability and health checking
  - Custom routing strategies
- **🔑 API Key Management**: Create multiple API keys with different permissions and rate limits
- **📊 Comprehensive Monitoring**: Real-time metrics and historical analytics
  - Four-tier metrics (request, API key, provider, global)
  - Token tracking and cost analysis
  - Latency percentiles (P50, P95, P99)
  - Interactive dashboards with Chart.js

### Advanced Features

- **🔐 OAuth 2.0 Support**: Client credentials flow for secure authentication
- **🤖 MCP (Model Context Protocol) Proxy**: Connect to MCP servers with authentication
  - STDIO transport (local processes)
  - SSE transport (HTTP Server-Sent Events)
  - OAuth auto-discovery and custom headers
- **⚡ Feature Adapters**: Unified interface for provider-specific capabilities
  - Prompt caching (Anthropic)
  - JSON mode (OpenAI, Anthropic)
  - Log probabilities (OpenAI)
  - Structured outputs (OpenAI)
  - Reasoning models (OpenAI o1/o3)
  - Thinking mode (Gemini 2.0 Flash)
- **🛡️ Rate Limiting**: Flexible limits on requests, tokens, and costs per API key
- **🔒 Secure Storage**: Encrypted API key storage with system keyring integration
- **📝 OpenAPI 3.1 Documentation**: Auto-generated, interactive API docs

### Desktop Experience

- **🖥️ Native Desktop UI**: Built with Tauri and React
- **📈 Real-time Dashboards**: Visualize metrics with interactive charts
- **⚙️ Easy Configuration**: Manage providers, models, API keys, and MCP servers
- **🔔 System Tray Integration**: Quick access with status indicator
- **🌍 Cross-Platform**: macOS, Windows, and Linux

## 🚀 Quick Start

### Prerequisites

- **Rust** 1.75 or later ([install](https://rustup.rs/))
- **Node.js** 18 or later ([install](https://nodejs.org/))
- **Tauri CLI** (we'll install this below)

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/localrouterai.git
cd localrouterai

# Run in development mode
cargo tauri dev
```

The application will start and the API server will be available at **http://localhost:3625**.

### First-Time Setup

1. **Launch the app** - The desktop UI will open automatically
2. **Configure a provider**:
   - Go to the **Providers** tab
   - Click **Add Provider** and select a provider (e.g., Ollama for local, OpenAI for cloud)
   - Enter your API key (if required) and save
3. **Create an API key**:
   - Go to the **API Keys** tab
   - Click **Create API Key**
   - Set rate limits and permissions
   - Copy your new LocalRouter API key (starts with `lr-`)
4. **Start using the API**:

```bash
curl http://localhost:3625/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-api-key-here" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {"role": "user", "content": "Hello! Tell me about yourself."}
    ]
  }'
```

## 📖 Usage Examples

### Chat Completions (Streaming)

```bash
curl http://localhost:3625/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-api-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Explain quantum computing"}],
    "stream": true
  }'
```

### Embeddings

```bash
curl http://localhost:3625/v1/embeddings \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-api-key" \
  -d '{
    "model": "text-embedding-ada-002",
    "input": "The quick brown fox jumps over the lazy dog"
  }'
```

### List Available Models

```bash
curl http://localhost:3625/v1/models \
  -H "Authorization: Bearer lr-your-api-key"
```

### Using with OpenAI SDK

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:3625/v1",
    api_key="lr-your-api-key"
)

response = client.chat.completions.create(
    model="gpt-4",
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.choices[0].message.content)
```

## 🗂️ Configuration

### Configuration Directory

Configuration files are stored in:
- **macOS**: `~/Library/Application Support/LocalRouter/` (production) or `~/.localrouter-dev/` (dev)
- **Linux**: `~/.localrouter/` (production) or `~/.localrouter-dev/` (dev)
- **Windows**: `%APPDATA%\LocalRouter\` (production) or `%APPDATA%\LocalRouter-dev\` (dev)

### Development Mode

For development without constant macOS keychain prompts:

```bash
export LOCALROUTER_KEYCHAIN=file
cargo tauri dev
```

**⚠️ Warning**: This stores secrets in plain text. Only use for development with test API keys.

### Provider Configuration

Providers are configured through the Desktop UI (**Providers** tab) or by editing `config.yaml`:

```yaml
providers:
  - id: ollama-local
    provider_type: Ollama
    name: "Local Ollama"
    base_url: "http://localhost:11434"
    enabled: true

  - id: openai-cloud
    provider_type: OpenAI
    name: "OpenAI GPT-4"
    api_key_ref: "openai-key-1"
    enabled: true
    health_check_enabled: true
```

## 📊 API Documentation

LocalRouter AI includes comprehensive OpenAPI 3.1 documentation:

- **Interactive Docs**: Available in the **Documentation** tab of the desktop app
- **OpenAPI Spec**: `http://localhost:3625/openapi.json`
- **Health Check**: `http://localhost:3625/health`

### Available Endpoints

| Endpoint | Description |
|----------|-------------|
| `POST /v1/chat/completions` | Chat completions (streaming & non-streaming) |
| `POST /v1/completions` | Text completions |
| `POST /v1/embeddings` | Generate embeddings |
| `GET /v1/models` | List available models |
| `POST /mcp/*` | MCP server proxy |
| `POST /oauth/token` | OAuth token endpoint |
| `GET /openapi.json` | OpenAPI specification |
| `GET /health` | Health check |

## 🏗️ Architecture

LocalRouter AI is built with a modular architecture:

```
┌─────────────────────────────────────────────────────────────┐
│                     Desktop UI (Tauri + React)              │
├─────────────────────────────────────────────────────────────┤
│                    Web Server (Axum on :3625)               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ Auth         │  │ Rate Limiting│  │ Routing      │      │
│  │ Middleware   │  │              │  │ Engine       │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
├─────────────────────────────────────────────────────────────┤
│                      Provider Layer                         │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐            │
│  │Ollama│ │OpenAI│ │Claude│ │Gemini│ │  ...  │            │
│  └──────┘ └──────┘ └──────┘ └──────┘ └──────┘            │
├─────────────────────────────────────────────────────────────┤
│  Config │ API Keys │ Monitoring │ MCP │ OAuth │ Storage   │
└─────────────────────────────────────────────────────────────┘
```

**Core Components:**
- **Web Server**: Axum-based HTTP server with OpenAI-compatible API
- **Provider Layer**: Abstraction over multiple AI providers with health checking
- **Router**: Intelligent request routing with fallback and cost optimization
- **Monitoring**: Four-tier metrics collection with real-time dashboards
- **MCP Proxy**: Model Context Protocol server integration
- **Storage**: Encrypted configuration and API key storage

## 📚 Documentation

Comprehensive documentation is available in the `plan/` directory:

| Document | Purpose |
|----------|---------|
| **[CLAUDE.md](CLAUDE.md)** | **START HERE** - Project guide and development workflow |
| **[plan/2026-01-14-ARCHITECTURE.md](plan/2026-01-14-ARCHITECTURE.md)** | Complete system design and technical specifications |
| **[plan/2026-01-14-PROGRESS.md](plan/2026-01-14-PROGRESS.md)** | Implementation progress (150+ features tracked) |
| **[plan/2026-01-14-CONTRIBUTING.md](plan/2026-01-14-CONTRIBUTING.md)** | Development setup and contribution guidelines |
| **[plan/2026-01-17-MCP_AUTH_REDESIGN.md](plan/2026-01-17-MCP_AUTH_REDESIGN.md)** | MCP and OAuth authentication design |
| **[plan/2026-01-17-FEATURE_ADAPTERS.md](plan/2026-01-17-FEATURE_ADAPTERS.md)** | Provider feature adapters documentation |

## 🧪 Development

### Building from Source

```bash
# Install dependencies
cargo build

# Run tests
cargo test

# Run linter
cargo clippy

# Format code
cargo fmt

# Development mode (uses ~/.localrouter-dev/)
cargo tauri dev

# Production build
cargo tauri build
```

### Project Structure

```
localrouterai/
├── src-tauri/              # Rust backend (~36,000 lines)
│   ├── src/
│   │   ├── server/         # Web server and API routes
│   │   ├── providers/      # Model provider implementations
│   │   ├── router/         # Smart routing system
│   │   ├── monitoring/     # Metrics and logging
│   │   ├── config/         # Configuration management
│   │   ├── api_keys/       # API key management (legacy)
│   │   ├── clients/        # Unified client system (new)
│   │   ├── mcp/            # MCP protocol support
│   │   ├── ui/             # Tauri command handlers
│   │   └── utils/          # Utilities
│   └── tests/              # Integration tests
├── src/                    # React frontend (~3,000 lines)
│   ├── components/
│   │   └── tabs/           # Main UI tabs
│   └── App.tsx
├── plan/                   # Planning documents
└── README.md               # This file
```

### Testing

LocalRouter AI has comprehensive test coverage:

- **367 total tests** (353 passing, 6 failing, 8 ignored)
- **Unit tests**: Individual component testing
- **Integration tests**: Provider API and feature adapter testing
- **MCP tests**: Protocol and transport testing (some in progress)

```bash
# Run all tests
cargo test

# Run unit tests only (faster)
cargo test --lib

# Run specific test file
cargo test --test provider_integration_tests

# Run with output
cargo test -- --nocapture
```

## 🗺️ Roadmap

### ✅ Completed (v0.1 - Current)

- [x] Core infrastructure (configuration, error handling, logging)
- [x] 10+ model provider implementations
- [x] Web server with OpenAI-compatible API
- [x] API key management with encrypted storage
- [x] Four-tier metrics system
- [x] Desktop UI with all main tabs
- [x] System tray integration
- [x] MCP server proxy (STDIO, SSE)
- [x] OAuth 2.0 authentication
- [x] Feature adapters (prompt caching, JSON mode, logprobs, etc.)
- [x] OpenAPI 3.1 documentation
- [x] Rate limiting (requests, tokens, cost)

### 🚧 In Progress (v0.2)

- [ ] Complete smart routing engine (strategies, fallback)
- [ ] Unified client architecture (merge API keys + OAuth clients)
- [ ] Fix remaining MCP test failures
- [ ] UI refactoring for simplified client management
- [ ] End-to-end testing

### 🔮 Planned (v0.3+)

- [ ] Custom routing strategies (user-defined rules)
- [ ] Model performance benchmarking
- [ ] Request retry and circuit breaker improvements
- [ ] Webhook notifications for events
- [ ] Multi-tenancy support
- [ ] Cloud sync for configuration (optional)
- [ ] CLI tool for headless operation
- [ ] Docker container support
- [ ] Plugin system for custom providers

**Overall Progress**: ~65% complete - Core application functional

## 🤝 Contributing

We welcome contributions! LocalRouter AI is open source and we'd love your help making it better.

**How to contribute:**

1. **Read the docs**: Start with [CLAUDE.md](CLAUDE.md) and [plan/2026-01-14-CONTRIBUTING.md](plan/2026-01-14-CONTRIBUTING.md)
2. **Check progress**: See [plan/2026-01-14-PROGRESS.md](plan/2026-01-14-PROGRESS.md) for available tasks
3. **Pick a feature**: Look for features marked ⬜ Not Started
4. **Follow the workflow**:
   - Fork the repository
   - Create a feature branch (`git checkout -b feat/amazing-feature`)
   - Make your changes following the architecture in [plan/2026-01-14-ARCHITECTURE.md](plan/2026-01-14-ARCHITECTURE.md)
   - Write tests for your changes
   - Run `cargo test && cargo clippy && cargo fmt`
   - Commit using [Conventional Commits](https://www.conventionalcommits.org/)
   - Submit a pull request

**Contribution Ideas:**
- 🐛 Fix bugs or failing tests
- ✨ Implement features from the roadmap
- 📝 Improve documentation
- 🧪 Add more tests
- 🎨 Enhance the UI
- 🌐 Add support for new providers
- 🚀 Optimize performance

## 📜 License

This project is licensed under the **AGPL-3.0-or-later License** - see the [LICENSE](LICENSE) file for details.

This means:
- ✅ You can use, modify, and distribute this software
- ✅ You can use it for commercial purposes
- ⚠️ You must disclose source code when distributing
- ⚠️ Network use constitutes distribution (must share modifications)
- ⚠️ Modified versions must use the same license

## 🙏 Acknowledgments

LocalRouter AI is built with incredible open source technologies:

- **[Rust](https://www.rust-lang.org/)** - Systems programming language
- **[Tauri](https://tauri.app/)** - Desktop application framework
- **[Axum](https://github.com/tokio-rs/axum)** - Web framework
- **[Tokio](https://tokio.rs/)** - Async runtime
- **[React](https://react.dev/)** - UI framework
- **[Chart.js](https://www.chartjs.org/)** - Charting library
- **[utoipa](https://github.com/juhaku/utoipa)** - OpenAPI code generation

## 📞 Support

- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/yourusername/localrouterai/issues)
- **Discussions**: Ask questions in [GitHub Discussions](https://github.com/yourusername/localrouterai/discussions)
- **Documentation**: See the [plan/](plan/) directory for comprehensive docs

## 🌟 Star History

If you find LocalRouter AI useful, please consider giving it a star! ⭐

---

<div align="center">

**Made with ❤️ by the LocalRouter AI community**

[Website](https://github.com/yourusername/localrouterai) • [Documentation](CLAUDE.md) • [Issues](https://github.com/yourusername/localrouterai/issues)

</div>
