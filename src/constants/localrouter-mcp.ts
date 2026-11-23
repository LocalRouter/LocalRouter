export const LOCALROUTER_MCP_ID = "_localrouter_admin"
export const LOCALROUTER_MCP_NAME = "LocalRouter"
export const LOCALROUTER_MCP_PATH = "/mcp/localrouter"

export function localrouterMcpUrl(port: number) {
  return `http://localhost:${port}${LOCALROUTER_MCP_PATH}`
}
