<p align="center">
  <img src="website/public/favicon.svg" alt="LocalRouter" width="80" />
</p>

<h1 align="center">LocalRouter</h1>

<p align="center">
  <strong>Local firewall for LLMs, MCPs, and Skills.</strong>
</p>

<p align="center">
  Centralized credential store with per-client access control. Automatic model failover across providers.<br/>
  Unified MCP gateway aggregating all MCPs and skills. Runtime approval firewall for sensitive operations.
</p>

<p align="center">
  <img src="website/public/icons/apple.svg" alt="macOS" width="16" height="16" />&nbsp;macOS&nbsp;&nbsp;
  <img src="website/public/icons/microsoft-windows.svg" alt="Windows" width="16" height="16" />&nbsp;Windows&nbsp;&nbsp;
  <img src="website/public/icons/penguin.svg" alt="Linux" width="16" height="16" />&nbsp;Linux
</p>

<p align="center">
  <a href="https://localrouter.ai"><strong>Website</strong></a> &bull;
  <a href="https://localrouter.ai/download"><strong>Download</strong></a>
</p>

<p align="center">
<i>Built with Claude and a hammer.</i>
</p>

---

## Development

### Prerequisites

- Rust 1.75+
- Node.js 18+

### Run

```bash
git clone https://github.com/LocalRouter/LocalRouter.git
cd LocalRouter
cargo tauri dev
```

### Test & Lint

```bash
cargo test && cargo clippy && cargo fmt
```

### Architecture

Built with Rust (backend) and React (frontend) using Tauri 2.x.

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

## Docker

A Linux container image is published to GHCR with each release:

```bash
xhost +SI:localuser:$(whoami)
docker run --rm -it \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -v "$(pwd)/localrouter-home:/home/app" \
  -p 3625:3625 \
  -e DISPLAY="$DISPLAY" \
  --device /dev/dri \
  ghcr.io/localrouter/localrouter:latest
```

X11 forwarding is required, so this works on Linux hosts only. See
[docs/DOCKER.md](docs/DOCKER.md) for full instructions and limitations.

---

## License

[GNU Affero General Public License v3.0](LICENSE)
