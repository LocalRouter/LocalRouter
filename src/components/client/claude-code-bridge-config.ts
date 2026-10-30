/**
 * Claude Code MCP entries. The STDIO bridge is the default because Claude
 * Code accepts pushed events (Channels) only from a stdio server using the
 * pre-2026-07-28 handshake; HTTP serves tools only. Mirrors
 * src-tauri/src/launcher/integrations/claude_code.rs.
 */

/** How Claude Code must be launched to load LocalRouter's channel. */
export const CLAUDE_CHANNELS_LAUNCH_COMMAND = "claude --dangerously-load-development-channels server:localrouter"

/** The `mcpServers` JSON for the STDIO bridge. */
export function claudeCodeStdioConfig(binaryPath: string, clientId: string, secret: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        localrouter: {
          type: "stdio",
          command: binaryPath,
          args: ["--mcp-bridge", "--client-id", clientId],
          env: { LOCALROUTER_CLIENT_SECRET: secret },
        },
      },
    },
    null,
    2,
  )
}

/** The tools-only HTTP alternative. */
export function claudeCodeHttpConfig(baseUrl: string, secret: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        localrouter: {
          type: "http",
          url: baseUrl,
          headers: { Authorization: `Bearer ${secret}` },
        },
      },
    },
    null,
    2,
  )
}
