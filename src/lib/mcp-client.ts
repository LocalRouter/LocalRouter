import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js"
import { WebSocketClientTransport } from "@modelcontextprotocol/sdk/client/websocket.js"
import {
  ResourceUpdatedNotificationSchema,
  ToolListChangedNotificationSchema,
  ResourceListChangedNotificationSchema,
  PromptListChangedNotificationSchema,
  CreateMessageRequestSchema,
  ElicitRequestSchema,
  ListRootsRequestSchema,
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
  mcpAccess?: string // MCP server access: "all", "none", or a specific server ID
  skillsAccess?: "all" | string // Skills access: "all" or specific skill name
  codingAgentAccess?: string // Coding agent type (e.g., "claude_code") for direct mode
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
  onToolsListChanged?: () => void
  onResourcesListChanged?: () => void
  onPromptsListChanged?: () => void
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
  private connectionGeneration = 0
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

    const generation = ++this.connectionGeneration
    let client: Client | null = null
    let transport: SSEClientTransport | WebSocketClientTransport | null = null
    this.updateState({ isConnecting: true, error: null })

    try {
      const endpoint = this.getEndpointUrl()
      const transportType = this.config.transportType || "sse"

      // Create transport based on type
      if (transportType === "websocket") {
        const wsUrl = endpoint.replace(/^http/, "ws")
        transport = new WebSocketClientTransport(new URL(wsUrl))
      } else {
        // SSE transport
        // Build headers - include access control headers for internal test client
        const headers: Record<string, string> = {
          Authorization: `Bearer ${this.config.clientToken}`,
        }
        if (this.config.mcpAccess) {
          headers["X-MCP-Access"] = this.config.mcpAccess
        }
        if (this.config.skillsAccess) {
          headers["X-Skills-Access"] = this.config.skillsAccess
        }
        if (this.config.codingAgentAccess) {
          headers["X-Coding-Agent-Access"] = this.config.codingAgentAccess
        }
        transport = new SSEClientTransport(new URL(endpoint), {
          requestInit: {
            headers,
          },
        })
      }

      // Declare client capabilities — stored for later reporting in connection info
      const declaredCapabilities = {
        // Declare support for receiving sampling requests from servers
        sampling: {},
        // Declare support for receiving elicitation requests (form mode)
        elicitation: { form: {} },
        // Declare support for filesystem roots with list change notifications
        roots: { listChanged: true },
      }

      // Create MCP client with proper capabilities declared
      // These tell the server what this client can handle
      client = new Client(
        {
          name: "localrouter-try-it-out",
          version: "1.0.0",
        },
        {
          capabilities: declaredCapabilities,
        }
      )

      // Register request handler for sampling/createMessage requests from servers
      // This allows MCP servers to request LLM completions through the client
      client.setRequestHandler(CreateMessageRequestSchema, async (request) => {
        if (this.callbacks.onSamplingRequest) {
          const result = await this.callbacks.onSamplingRequest(request.params)
          return result
        }

        // If no handler registered, return an error
        throw new Error("Sampling requests are not handled by this client")
      })

      // Register request handler for elicitation requests from servers
      // This allows MCP servers to request user input through the client
      client.setRequestHandler(ElicitRequestSchema, async (request) => {
        if (this.callbacks.onElicitationRequest) {
          const result = await this.callbacks.onElicitationRequest(request.params)
          return result
        }

        // If no handler registered, decline the request
        return { action: "decline" as const }
      })

      // This client exposes no local filesystem roots.
      client.setRequestHandler(ListRootsRequestSchema, async () => ({ roots: [] }))

      // Capture the connection before awaiting so disconnect can cancel startup.
      this.client = client
      this.transport = transport
      await client.connect(transport)
      if (generation !== this.connectionGeneration) {
        await client.close().catch(() => {})
        await transport.close().catch(() => {})
        return
      }
      client.onclose = () => {
        if (generation !== this.connectionGeneration) return
        ++this.connectionGeneration
        this.client = null
        this.transport = null
        this.resourceSubscriptions.clear()
        this.resetConnectionState()
      }

      // Register notification handler for resource updates
      client.setNotificationHandler(ResourceUpdatedNotificationSchema, (notification) => {
        const uri = notification.params.uri
        console.log("[MCP Client] Received resource update notification for:", uri)

        // Look up callback for this URI and call it
        const callback = this.resourceSubscriptions.get(uri)
        if (callback) {
          // Read the updated resource content
          this.readResource(uri)
            .then((content) => {
              if (generation === this.connectionGeneration && this.resourceSubscriptions.get(uri) === callback) {
                callback(uri, content)
              }
            })
            .catch((err) => {
              console.error("[MCP Client] Failed to read updated resource:", err)
            })
        } else {
          console.log("[MCP Client] No subscription callback for URI:", uri)
        }
      })

      // Register notification handlers for list changes
      client.setNotificationHandler(ToolListChangedNotificationSchema, () => {
        console.log("[MCP Client] Received tools/list_changed notification")
        this.callbacks.onToolsListChanged?.()
      })

      client.setNotificationHandler(ResourceListChangedNotificationSchema, () => {
        console.log("[MCP Client] Received resources/list_changed notification")
        this.callbacks.onResourcesListChanged?.()
      })

      client.setNotificationHandler(PromptListChangedNotificationSchema, () => {
        console.log("[MCP Client] Received prompts/list_changed notification")
        this.callbacks.onPromptsListChanged?.()
      })

      // Get server info
      const serverInfo = client.getServerVersion()
      const serverCapabilities = client.getServerCapabilities()
      const instructions = client.getInstructions()

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
        sampling: !!declaredCapabilities.sampling,
        elicitation: declaredCapabilities.elicitation
          ? { form: !!(declaredCapabilities.elicitation as Record<string, unknown>).form }
          : undefined,
        roots: declaredCapabilities.roots
          ? { listChanged: !!(declaredCapabilities.roots as Record<string, unknown>).listChanged }
          : undefined,
      }

      // Read the negotiated protocol version from the transport
      const negotiatedProtocolVersion =
        (transport as unknown as { _protocolVersion?: string })?._protocolVersion || "unknown"

      this.updateState({
        isConnected: true,
        isConnecting: false,
        serverInfo: serverInfo ? {
          name: serverInfo.name,
          version: serverInfo.version,
          protocolVersion: negotiatedProtocolVersion,
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
      // Close only this attempt; an older attempt must never tear down a retry.
      const isCurrent = generation === this.connectionGeneration
      if (isCurrent) {
        ++this.connectionGeneration
        this.client = null
        this.transport = null
        this.resourceSubscriptions.clear()
        this.resetConnectionState(error instanceof Error ? error.message : "Connection failed")
      }
      await client?.close().catch(() => {})
      await transport?.close().catch(() => {})
      if (isCurrent) throw error
    }
  }

  private resetConnectionState(error: string | null = null) {
    this.updateState({
      isConnected: false,
      isConnecting: false,
      error,
      serverInfo: undefined,
      clientInfo: undefined,
      serverCapabilities: undefined,
      clientCapabilities: undefined,
      capabilities: undefined,
    })
  }

  async disconnect(): Promise<void> {
    ++this.connectionGeneration
    const client = this.client
    const transport = this.transport
    this.client = null
    this.transport = null
    this.resourceSubscriptions.clear()
    this.resetConnectionState()
    await client?.close().catch(() => {})
    await transport?.close().catch(() => {})
  }

  private async listAllPages<T>(
    load: (cursor?: string) => Promise<{ items: T[]; nextCursor?: string }>,
  ): Promise<T[]> {
    const items: T[] = []
    const seenCursors = new Set<string>()
    let cursor: string | undefined
    do {
      const page = await load(cursor)
      items.push(...page.items)
      cursor = page.nextCursor
      if (cursor !== undefined) {
        if (seenCursors.has(cursor)) throw new Error("Server returned a repeated pagination cursor")
        seenCursors.add(cursor)
      }
    } while (cursor !== undefined)
    return items
  }

  // Tools
  async listTools(): Promise<Tool[]> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const client = this.client
    return this.listAllPages(async cursor => {
      const result = await client.listTools(cursor === undefined ? undefined : { cursor })
      return { items: result.tools, nextCursor: result.nextCursor }
    })
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
    const client = this.client
    return this.listAllPages(async cursor => {
      const result = await client.listResources(cursor === undefined ? undefined : { cursor })
      return { items: result.resources, nextCursor: result.nextCursor }
    })
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

    const generation = this.connectionGeneration
    const previous = this.resourceSubscriptions.get(uri)
    // Store callback
    this.resourceSubscriptions.set(uri, callback)

    // Roll back the local subscription when the server rejects it.
    try {
      await this.client.subscribeResource({ uri })
    } catch (error) {
      if (generation === this.connectionGeneration && this.resourceSubscriptions.get(uri) === callback) {
        if (previous) this.resourceSubscriptions.set(uri, previous)
        else this.resourceSubscriptions.delete(uri)
      }
      throw error
    }
  }

  async unsubscribeFromResource(uri: string): Promise<void> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }

    const generation = this.connectionGeneration
    const callback = this.resourceSubscriptions.get(uri)
    await this.client.unsubscribeResource({ uri })
    if (generation === this.connectionGeneration && this.resourceSubscriptions.get(uri) === callback) this.resourceSubscriptions.delete(uri)
  }

  // Prompts
  async listPrompts(): Promise<Prompt[]> {
    if (!this.client || !this.state.isConnected) {
      throw new Error("Not connected")
    }
    const client = this.client
    return this.listAllPages(async cursor => {
      const result = await client.listPrompts(cursor === undefined ? undefined : { cursor })
      return { items: result.prompts, nextCursor: result.nextCursor }
    })
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
