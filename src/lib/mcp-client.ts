import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js"
import { WebSocketClientTransport } from "@modelcontextprotocol/sdk/client/websocket.js"
import {
  ResourceUpdatedNotificationSchema,
  CreateMessageRequestSchema,
  ElicitRequestSchema,
} from "@modelcontextprotocol/sdk/types.js"
import type {
  Tool,
  Resource,
  Prompt,
  TextContent,
  ImageContent,
  CreateMessageRequest,
  CreateMessageResult,
  ElicitRequest,
  Progress,
} from "@modelcontextprotocol/sdk/types.js"

export type { Tool, Resource, Prompt, TextContent, ImageContent }

// Re-export types needed by sampling/elicitation panels
export type { CreateMessageRequest, CreateMessageResult, ElicitRequest, Progress }

// Callback types for sampling and elicitation requests from servers
export type SamplingRequestHandler = (
  request: CreateMessageRequest["params"]
) => Promise<CreateMessageResult>

export type ElicitationRequestHandler = (
  request: ElicitRequest["params"]
) => Promise<{ action: "accept" | "decline"; content?: Record<string, unknown> }>

// Progress callback type
export type ProgressCallback = (progress: Progress) => void

export type TransportType = "sse" | "websocket"

export interface McpClientConfig {
  serverPort: number
  clientToken: string
  transportType?: TransportType
  deferredLoading?: boolean // Enable deferred loading for unified gateway
  mcpAccess?: string // MCP server access: "all", "none", or a specific server ID
  skillsAccess?: "all" | string // Skills access: "all" or specific skill name
}

// Detailed capability info for display
export interface ServerCapabilitiesInfo {
  tools?: { listChanged?: boolean }
  resources?: { subscribe?: boolean; listChanged?: boolean }
  prompts?: { listChanged?: boolean }
  logging?: boolean
  completions?: boolean
  experimental?: Record<string, unknown>
}

export interface ClientCapabilitiesInfo {
  sampling?: boolean
  elicitation?: { form?: boolean; url?: boolean }
  roots?: { listChanged?: boolean }
  experimental?: Record<string, unknown>
}

export interface McpConnectionState {
  isConnected: boolean
  isConnecting: boolean
  error: string | null
  serverInfo?: {
    name: string
    version: string
    protocolVersion: string
    instructions?: string
  }
  clientInfo?: {
    name: string
    version: string
  }
  serverCapabilities?: ServerCapabilitiesInfo
  clientCapabilities?: ClientCapabilitiesInfo
  // Legacy simplified capabilities for backward compatibility
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

export interface McpClientCallbacks {
  onStateChange?: (state: McpConnectionState) => void
  onSamplingRequest?: SamplingRequestHandler
  onElicitationRequest?: ElicitationRequestHandler
}

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
  private callbacks: McpClientCallbacks

  constructor(config: McpClientConfig, callbacks: McpClientCallbacks = {}) {
    this.config = config
    this.callbacks = callbacks
  }

  // Allow updating callbacks after construction (for React state updates)
  setCallbacks(callbacks: Partial<McpClientCallbacks>) {
    this.callbacks = { ...this.callbacks, ...callbacks }
  }

  private updateState(updates: Partial<McpConnectionState>) {
    this.state = { ...this.state, ...updates }
    this.callbacks.onStateChange?.(this.state)
  }

  getState(): McpConnectionState {
    return { ...this.state }
  }

  private getEndpointUrl(): string {
    return `http://localhost:${this.config.serverPort}/`
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
        // Build headers - include access control headers for internal test client
        const headers: Record<string, string> = {
          Authorization: `Bearer ${this.config.clientToken}`,
        }
        if (this.config.deferredLoading) {
          headers["X-Deferred-Loading"] = "true"
        }
        if (this.config.mcpAccess) {
          headers["X-MCP-Access"] = this.config.mcpAccess
        }
        if (this.config.skillsAccess) {
          headers["X-Skills-Access"] = this.config.skillsAccess
        }
        this.transport = new SSEClientTransport(new URL(endpoint), {
          requestInit: {
            headers,
          },
        })
      }

      // Create MCP client with proper capabilities declared
      // These tell the server what this client can handle
      this.client = new Client(
        {
          name: "localrouter-try-it-out",
          version: "1.0.0",
        },
        {
          capabilities: {
            // Declare support for receiving sampling requests from servers
            sampling: {},
            // Declare support for receiving elicitation requests (form mode)
            elicitation: { form: {} },
            // Declare support for filesystem roots with list change notifications
            roots: { listChanged: true },
          },
        }
      )

      // Register request handler for sampling/createMessage requests from servers
      // This allows MCP servers to request LLM completions through the client
      this.client.setRequestHandler(CreateMessageRequestSchema, async (request) => {
        console.log("[MCP Client] Received sampling/createMessage request:", request.params)

        if (this.callbacks.onSamplingRequest) {
          const result = await this.callbacks.onSamplingRequest(request.params)
          return result
        }

        // If no handler registered, return an error
        throw new Error("Sampling requests are not handled by this client")
      })

      // Register request handler for elicitation requests from servers
      // This allows MCP servers to request user input through the client
      this.client.setRequestHandler(ElicitRequestSchema, async (request) => {
        console.log("[MCP Client] Received elicitation request:", request.params)

        if (this.callbacks.onElicitationRequest) {
          const result = await this.callbacks.onElicitationRequest(request.params)
          return result
        }

        // If no handler registered, decline the request
        return { action: "decline" as const }
      })

      // Connect
      await this.client.connect(this.transport)

      // Register notification handler for resource updates
      this.client.setNotificationHandler(ResourceUpdatedNotificationSchema, (notification) => {
        const uri = notification.params.uri
        console.log("[MCP Client] Received resource update notification for:", uri)

        // Look up callback for this URI and call it
        const callback = this.resourceSubscriptions.get(uri)
        if (callback) {
          // Read the updated resource content
          this.readResource(uri)
            .then((content) => {
              callback(uri, content)
            })
            .catch((err) => {
              console.error("[MCP Client] Failed to read updated resource:", err)
            })
        } else {
          console.log("[MCP Client] No subscription callback for URI:", uri)
        }
      })

      // Get server info
      const serverInfo = this.client.getServerVersion()
      const serverCapabilities = this.client.getServerCapabilities()
      const instructions = this.client.getInstructions()

      // Build detailed capability info
      const serverCapsInfo: ServerCapabilitiesInfo = {
        tools: serverCapabilities?.tools ? { listChanged: serverCapabilities.tools.listChanged } : undefined,
        resources: serverCapabilities?.resources ? {
          subscribe: serverCapabilities.resources.subscribe,
          listChanged: serverCapabilities.resources.listChanged,
        } : undefined,
        prompts: serverCapabilities?.prompts ? { listChanged: serverCapabilities.prompts.listChanged } : undefined,
        logging: !!serverCapabilities?.logging,
        completions: !!serverCapabilities?.completions,
        experimental: serverCapabilities?.experimental as Record<string, unknown> | undefined,
      }

      const clientCapsInfo: ClientCapabilitiesInfo = {
        sampling: true, // We declared sampling support
        elicitation: { form: true }, // We declared form elicitation support
        roots: { listChanged: true },
      }

      this.updateState({
        isConnected: true,
        isConnecting: false,
        serverInfo: serverInfo ? {
          name: serverInfo.name,
          version: serverInfo.version,
          protocolVersion: "2024-11-05",
          instructions,
        } : undefined,
        clientInfo: {
          name: "localrouter-try-it-out",
          version: "1.0.0",
        },
        serverCapabilities: serverCapsInfo,
        clientCapabilities: clientCapsInfo,
        // Legacy simplified capabilities
        capabilities: {
          tools: !!serverCapabilities?.tools,
          resources: !!serverCapabilities?.resources,
          prompts: !!serverCapabilities?.prompts,
          sampling: !!this.callbacks.onSamplingRequest,
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
      clientInfo: undefined,
      serverCapabilities: undefined,
      clientCapabilities: undefined,
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

  async callTool(
    name: string,
    args: Record<string, unknown>,
    onProgress?: ProgressCallback
  ): Promise<{ content: unknown[]; isError?: boolean }> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const result = await this.client.callTool(
      { name, arguments: args },
      undefined, // resultSchema
      { onprogress: onProgress } // RequestOptions with progress callback
    )
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

// Factory function - supports both old and new callback signatures
export function createMcpClient(
  config: McpClientConfig,
  callbacksOrOnStateChange?: McpClientCallbacks | ((state: McpConnectionState) => void)
): McpClientWrapper {
  // Support both old signature (just onStateChange) and new signature (full callbacks)
  const callbacks: McpClientCallbacks =
    typeof callbacksOrOnStateChange === "function"
      ? { onStateChange: callbacksOrOnStateChange }
      : callbacksOrOnStateChange ?? {}

  return new McpClientWrapper(config, callbacks)
}
