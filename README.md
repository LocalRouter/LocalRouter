# LocalRouter AI

A cross-platform desktop application for intelligent AI model routing with OpenAI-compatible API.

## Overview

LocalRouter AI provides a local OpenAI-compatible API gateway that intelligently routes requests across multiple AI providers (OpenAI, Anthropic, Ollama, OpenRouter, Gemini, etc.) with smart routing, cost optimization, and comprehensive monitoring.

## Features

- **OpenAI-Compatible API**: Drop-in replacement for OpenAI API
- **Multi-Provider Support**: Ollama, OpenAI, Anthropic, Google Gemini, OpenRouter, and more
- **Smart Routing**: Automatic model selection based on cost, performance, and availability
- **API Key Management**: Create and manage multiple API keys with different routing configurations
- **Rate Limiting**: Flexible rate limits on requests, tokens, and costs
- **Monitoring**: Real-time metrics and historical analytics
- **Desktop UI**: Native desktop app with system tray integration
- **Cross-Platform**: Works on macOS, Windows, and Linux

## Documentation

- **[CLAUDE.md](./CLAUDE.md)** - **START HERE**: Project guide explaining all documents and development workflow
- **[ARCHITECTURE.md](./ARCHITECTURE.md)** - Detailed system design and component breakdown
- **[PROGRESS.md](./PROGRESS.md)** - Implementation progress and feature tracking (150+ features)
- **[CONTRIBUTING.md](./CONTRIBUTING.md)** - Development workflow and contribution guidelines

## Quick Start

### Prerequisites

- Rust 1.75 or later
- Node.js 18 or later (for frontend)
- Tauri CLI

### Installation

```bash
# Install Tauri CLI
cargo install tauri-cli

# Clone the repository
git clone https://github.com/yourusername/localrouterai.git
cd localrouterai

# Build and run in development mode
cargo tauri dev
```

### Building for Production

```bash
# Build optimized binary
cargo tauri build
```

## Usage

### Starting the Server

The server starts automatically when you launch the application. By default, it listens on `http://localhost:3000`.

### Using the API

```bash
# Example: Chat completion
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer lr-your-api-key-here" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {"role": "user", "content": "Hello!"}
    ]
  }'
```

### Configuration

Configuration files are stored in:
- **Linux**: `~/.localrouter/`
- **macOS**: `~/Library/Application Support/LocalRouter/`
- **Windows**: `%APPDATA%\LocalRouter\`

## Project Structure

```
localrouterai/
├── src-tauri/           # Rust backend (Tauri)
│   ├── src/
│   │   ├── main.rs
│   │   ├── server/      # Web server
│   │   ├── config/      # Configuration management
│   │   ├── providers/   # Model providers
│   │   ├── router/      # Smart routing
│   │   ├── api_keys/    # API key management
│   │   ├── monitoring/  # Metrics and logging
│   │   ├── ui/          # Tauri commands
│   │   └── utils/       # Utilities
│   └── Cargo.toml
├── src/                 # Frontend (React/Vue/Svelte)
├── ARCHITECTURE.md      # System architecture
├── PROGRESS.md          # Development progress
└── README.md            # This file
```

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

## License

[MIT License](./LICENSE)

## Roadmap

- [x] Architecture design
- [ ] Core infrastructure
- [ ] Model provider implementations
- [ ] Smart routing system
- [ ] Web server and API
- [ ] API key management
- [ ] Monitoring and logging
- [ ] Desktop UI
- [ ] Testing and polish
- [ ] v1.0 Release

## Support

For issues and feature requests, please use the [GitHub Issues](https://github.com/yourusername/localrouterai/issues) page.

## Acknowledgments

Built with:
- [Rust](https://www.rust-lang.org/)
- [Tauri](https://tauri.app/)
- [Axum](https://github.com/tokio-rs/axum)
- [Tokio](https://tokio.rs/)
