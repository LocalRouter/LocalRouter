<!-- @entry marketplace-overview -->

The Marketplace is a built-in discovery and installation system for MCP servers. It provides a browsable catalog of MCP servers from multiple registry sources, with one-click installation that handles configuration, authentication setup, and transport configuration automatically. The marketplace UI shows server descriptions, supported tools, installation requirements, and user ratings.

<!-- @entry registry-sources -->

The marketplace aggregates MCP servers from multiple registry sources, each providing a curated list of servers with metadata. Registries are fetched periodically and cached locally. Multiple registries can be enabled simultaneously, and their catalogs are merged in the marketplace UI with source attribution.

<!-- @entry registry-official -->

The official registry contains MCP servers vetted and maintained by the LocalRouter team. These servers are tested for compatibility, security, and reliability. The official registry is enabled by default and serves as the primary source of trusted MCP servers. Servers in this registry follow a standardized metadata format and include verified installation instructions.

<!-- @entry registry-community -->

Community registries contain MCP servers contributed by the broader community. These are aggregated from public MCP server lists and repositories. Community servers are labeled as such in the marketplace UI, and users should review their source code and permissions before installation. Community registries can be enabled or disabled in settings.

<!-- @entry registry-private -->

Private registries allow organizations to host their own internal catalog of MCP servers. Configure a private registry by providing a URL to a registry-format JSON endpoint. This is useful for distributing internal tools, proprietary integrations, or pre-configured MCP servers across a team. Private registries support authentication via API keys or OAuth tokens.

<!-- @entry mcp-exposed-search -->

The marketplace catalog is also exposed as an MCP tool (`localrouter__search_marketplace`), allowing LLMs to search for and discover available MCP servers during a conversation. An LLM can search by keyword, category, or capability, receive a list of matching servers with descriptions, and suggest installation to the user. This enables dynamic tool discovery — the LLM can find and request new tools as needed for a task.

<!-- @entry gated-installation -->

MCP server installation from the marketplace goes through a gated approval process. When a user or LLM requests installation, the UI shows the server's required permissions (file system access, network access, environment variables), the transport type, and any required API keys. The user must explicitly approve the installation before the server is added to the configuration. This prevents unauthorized tool installation and ensures users understand what access each server requires.
