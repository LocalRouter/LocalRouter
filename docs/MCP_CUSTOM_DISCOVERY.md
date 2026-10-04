# Discovering a custom MCP server

In **MCP Servers → Add → Custom**, enter an MCP URL or a subprocess command in **URL or command**, then click **Discover**. URLs select HTTP automatically; commands select STDIO. The transport selector remains editable.

Discovery reads server identity, supported MCP versions and capabilities with `server/discover`, falling back to legacy `initialize` when necessary. It checks HTTP authentication challenges and OAuth protected-resource metadata, then OAuth or OpenID Connect authorization-server metadata. Detected browser OAuth or Bearer authentication fills the form unless you selected authentication manually. An unauthenticated server may advertise login requirements before allowing identity or capabilities to be read.

You can supply headers, Bearer tokens, environment variables and a working directory before probing. Discovery uses those settings, preserves an existing server name and manual authentication choices, and ignores results that arrive after inputs change. All fields remain editable, and discovery failure does not prevent manual creation.

Discover runs the specified command or contacts the specified URL. It does not invoke tools, save a server or start an OAuth login. Temporary subprocesses and connections are closed after probing. Selecting browser OAuth and creating the server starts login through the existing authentication flow.

LocalRouter sets MCP protocol headers automatically. MCP does not define discovery of arbitrary vendor API-key header names, environment-variable names or credential values. If the server advertises a standard HTTP authentication challenge, discovery reports the `Authorization` header requirement; otherwise, consult the server's documentation and use the editable header or environment fields.

Specification references: [MCP lifecycle discovery](https://go.sdk.modelcontextprotocol.io/protocol/), [authorization-server discovery](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/authorization/authorization-server-discovery.mdx).
