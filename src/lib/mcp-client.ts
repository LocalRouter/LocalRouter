import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js"
import { WebSocketClientTransport } from "@modelcontextprotocol/sdk/client/websocket.js"
import type {
  Tool,
  Resource,
  Prompt,
  TextContent,
  ImageContent,
} from "@modelcontextprotocol/sdk/types.js"

export type { Tool, Resource, Prompt, TextContent, ImageContent }

export type TransportType = "sse" | "websocket"

export interface McpClientConfig {
  serverPort: number
  clientToken: string
  serverId?: string // If provided, connect to specific server; otherwise use gateway
  transportType?: TransportType
}

export interface McpConnectionState {
  isConnected: boolean
  isConnecting: boolean
  error: string | null
  serverInfo?: {
    name: string
    version: string
    protocolVersion: string
  }
  capabilities?: {
    tools?: boolean
    resources?: boolean
    prompts?: boolean
    sampling?: boolean
  }
}

export interface ResourceContent {
  uri: string
  mimeType?: string
  text?: string
  blob?: string
}

export interface ReadResourceResult {
  contents: ResourceContent[]
}

export interface GetPromptResult {
  messages: Array<{
    role: string
    content: unknown
  }>
}

export type ResourceUpdateCallback = (uri: string, content: ReadResourceResult) => void

export class McpClientWrapper {
  private client: Client | null = null
  private transport: SSEClientTransport | WebSocketClientTransport | null = null
  private config: McpClientConfig
  private state: McpConnectionState = {
    isConnected: false,
    isConnecting: false,
    error: null,
  }
  private resourceSubscriptions = new Map<string, ResourceUpdateCallback>()
  private onStateChange?: (state: McpConnectionState) => void

  constructor(config: McpClientConfig, onStateChange?: (state: McpConnectionState) => void) {
    this.config = config
    this.onStateChange = onStateChange
  }

  private updateState(updates: Partial<McpConnectionState>) {
    this.state = { ...this.state, ...updates }
    this.onStateChange?.(this.state)
  }

  getState(): McpConnectionState {
    return { ...this.state }
  }

  private getEndpointUrl(): string {
    const { serverPort, serverId } = this.config
    if (serverId) {
      return `http://localhost:${serverPort}/mcp/${serverId}`
    }
    return `http://localhost:${serverPort}/`
  }

  async connect(): Promise<void> {
    if (this.state.isConnected || this.state.isConnecting) {
      return
    }

    this.updateState({ isConnecting: true, error: null })

    try {
      const endpoint = this.getEndpointUrl()
      const transportType = this.config.transportType || "sse"

      // Create transport based on type
      if (transportType === "websocket") {
        const wsUrl = endpoint.replace(/^http/, "ws")
        this.transport = new WebSocketClientTransport(new URL(wsUrl))
      } else {
        // SSE transport
        this.transport = new SSEClientTransport(new URL(endpoint), {
          requestInit: {
            headers: {
              Authorization: `Bearer ${this.config.clientToken}`,
            },
          },
        })
      }

      // Create MCP client
      this.client = new Client(
        {
          name: "localrouter-try-it-out",
          version: "1.0.0",
        },
        {
          capabilities: {},
        }
      )

      // Connect
      await this.client.connect(this.transport)

      // Get server info
      const serverInfo = this.client.getServerVersion()
      const capabilities = this.client.getServerCapabilities()

      this.updateState({
        isConnected: true,
        isConnecting: false,
        serverInfo: serverInfo ? {
          name: serverInfo.name,
          version: serverInfo.version,
          protocolVersion: "2024-11-05",
        } : undefined,
        capabilities: {
          tools: !!capabilities?.tools,
          resources: !!capabilities?.resources,
          prompts: !!capabilities?.prompts,
          sampling: false, // Sampling handled separately via Tauri events
        },
      })
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : "Connection failed"
      this.updateState({
        isConnecting: false,
        error: errorMessage,
      })
      throw error
    }
  }

  async disconnect(): Promise<void> {
    if (this.client) {
      try {
        await this.client.close()
      } catch {
        // Ignore close errors
      }
      this.client = null
    }
    if (this.transport) {
      try {
        await this.transport.close()
      } catch {
        // Ignore close errors
      }
      this.transport = null
    }
    this.resourceSubscriptions.clear()
    this.updateState({
      isConnected: false,
      isConnecting: false,
      error: null,
      serverInfo: undefined,
      capabilities: undefined,
    })
  }

  // Tools
  async listTools(): Promise<Tool[]> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.listTools()
    return result.tools
  }

  async callTool(name: string, args: Record<string, unknown>): Promise<{ content: unknown[]; isError?: boolean }> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.callTool({ name, arguments: args })
    return {
      content: result.content as unknown[],
      isError: result.isError as boolean | undefined,
    }
  }

  // Resources
  async listResources(): Promise<Resource[]> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.listResources()
    return result.resources
  }

  async readResource(uri: string): Promise<ReadResourceResult> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.readResource({ uri })
    return {
      contents: result.contents.map(c => ({
        uri: c.uri,
        mimeType: c.mimeType,
        text: "text" in c ? c.text : undefined,
        blob: "blob" in c ? c.blob : undefined,
      })),
    }
  }

  async subscribeToResource(uri: string, callback: ResourceUpdateCallback): Promise<void> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }

    // Store callback
    this.resourceSubscriptions.set(uri, callback)

    // Send subscription request
    await this.client.subscribeResource({ uri })
  }

  async unsubscribeFromResource(uri: string): Promise<void> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }

    this.resourceSubscriptions.delete(uri)
    await this.client.unsubscribeResource({ uri })
  }

  // Prompts
  async listPrompts(): Promise<Prompt[]> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.listPrompts()
    return result.prompts
  }

  async getPrompt(name: string, args: Record<string, string>): Promise<GetPromptResult> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.getPrompt({ name, arguments: args })
    return {
      messages: result.messages.map(m => ({
        role: m.role,
        content: m.content,
      })),
    }
  }
}

// Factory function
export function createMcpClient(
  config: McpClientConfig,
  onStateChange?: (state: McpConnectionState) => void
): McpClientWrapper {
  return new McpClientWrapper(config, onStateChange)
}
