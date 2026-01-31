import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/Badge"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { Checkbox } from "@/components/ui/checkbox"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Wrench, FileText, MessageSquare, Radio, HelpCircle, AlertCircle, X, Circle, Info, Users, Globe, Zap } from "lucide-react"
import { ConnectionInfoPanel } from "./connection-info-panel"
import { ToolsPanel } from "./tools-panel"
import { ResourcesPanel } from "./resources-panel"
import { PromptsPanel } from "./prompts-panel"
import { SamplingPanel } from "./sampling-panel"
import { ElicitationPanel } from "./elicitation-panel"
import {
  createMcpClient,
  type McpClientWrapper,
  type McpConnectionState,
  type ReadResourceResult,
  type CreateMessageRequest,
  type CreateMessageResult,
  type ElicitRequest,
  type Tool,
  type Resource,
  type Prompt,
  type GetPromptResult,
} from "@/lib/mcp-client"

// Types for pending requests that need user action
export interface PendingSamplingRequest {
  id: string
  params: CreateMessageRequest["params"]
  timestamp: Date
  resolve: (result: CreateMessageResult) => void
  reject: (error: Error) => void
}

export interface PendingElicitationRequest {
  id: string
  params: ElicitRequest["params"]
  timestamp: Date
  resolve: (result: { action: "accept" | "decline"; content?: Record<string, unknown> }) => void
  reject: (error: Error) => void
}

// Types for completed requests (history)
export interface CompletedSamplingRequest {
  id: string
  params: PendingSamplingRequest["params"]
  timestamp: Date
  status: "completed" | "rejected"
  response?: CreateMessageResult
  error?: string
}

export interface CompletedElicitationRequest {
  id: string
  params: PendingElicitationRequest["params"]
  timestamp: Date
  status: "submitted" | "cancelled"
  response?: Record<string, unknown>
}

// Types for tool execution state
export interface ToolExecutionState {
  selectedTool: Tool | null
  formValues: Record<string, unknown>
  isExecuting: boolean
  result: { success: boolean; data: unknown } | null
  error: string | null
}

// Types for resource state
export interface ResourceState {
  selectedResource: Resource | null
  content: ReadResourceResult | null
  isReading: boolean
  error: string | null
}

// Types for prompt state
export interface PromptState {
  selectedPrompt: Prompt | null
  argValues: Record<string, string>
  isGetting: boolean
  result: GetPromptResult | null
  error: string | null
}

// Types for sampling state (lifted)
export interface SamplingState {
  completedRequests: CompletedSamplingRequest[]
  selectedRequestId: string | null
}

// Types for elicitation state (lifted)
export interface ElicitationState {
  completedRequests: CompletedElicitationRequest[]
  selectedRequestId: string | null
  formValues: Record<string, unknown>
}

interface McpClient {
  id: string
  name: string
  client_id: string
  enabled: boolean
}

interface McpServer {
  id: string
  name: string
  transport_type: string
  enabled: boolean
  status?: string
}

type McpTestMode = "client" | "all" | "direct"

interface ServerConfig {
  host: string
  port: number
  actual_port: number | null
  enable_cors: boolean
}

interface McpTabProps {
  innerPath: string | null
  onPathChange: (path: string | null) => void
}

export function McpTab({ innerPath, onPathChange }: McpTabProps) {
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [mode, setMode] = useState<McpTestMode>("all")
  const [clients, setClients] = useState<McpClient[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>("")
  const [clientApiKey, setClientApiKey] = useState<string | null>(null)
  const [selectedServerId, setSelectedServerId] = useState<string>("")
  const [serverPort, setServerPort] = useState<number | null>(null)
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // MCP Client state
  const mcpClientRef = useRef<McpClientWrapper | null>(null)
  const [connectionState, setConnectionState] = useState<McpConnectionState>({
    isConnected: false,
    isConnecting: false,
    error: null,
  })

  // Subscription and notification state (lifted from child components)
  const [subscribedUris, setSubscribedUris] = useState<Set<string>>(new Set())
  const [resourceUpdates, setResourceUpdates] = useState<Map<string, ReadResourceResult>>(new Map())

  // Pending sampling and elicitation requests from MCP servers
  const [pendingSamplingRequests, setPendingSamplingRequests] = useState<PendingSamplingRequest[]>([])
  const [pendingElicitationRequests, setPendingElicitationRequests] = useState<PendingElicitationRequest[]>([])

  // Auto-approve settings (lifted from SamplingPanel to persist across tab switches)
  const [autoApproveSampling, setAutoApproveSampling] = useState(false)

  // Deferred loading for unified gateway (reduces token consumption for large catalogs)
  const [deferredLoading, setDeferredLoading] = useState(false)

  // Sampling panel state (lifted to persist across tab switches)
  const [samplingState, setSamplingState] = useState<SamplingState>({
    completedRequests: [],
    selectedRequestId: null,
  })

  // Elicitation panel state (lifted to persist across tab switches)
  const [elicitationState, setElicitationState] = useState<ElicitationState>({
    completedRequests: [],
    selectedRequestId: null,
    formValues: {},
  })

  // Tools panel state (lifted to persist across tab switches)
  const [toolState, setToolState] = useState<ToolExecutionState>({
    selectedTool: null,
    formValues: {},
    isExecuting: false,
    result: null,
    error: null,
  })

  // Resources panel state (lifted to persist across tab switches)
  const [resourceState, setResourceState] = useState<ResourceState>({
    selectedResource: null,
    content: null,
    isReading: false,
    error: null,
  })

  // Prompts panel state (lifted to persist across tab switches)
  const [promptState, setPromptState] = useState<PromptState>({
    selectedPrompt: null,
    argValues: {},
    isGetting: false,
    result: null,
    error: null,
  })

  // Counter for generating unique request IDs
  const requestIdCounter = useRef(0)

  // Parse inner path to get subtab
  const parseInnerPath = (path: string | null) => {
    if (!path) return "connection"
    const parts = path.split("/")
    return parts[0] || "connection"
  }

  const activeSubtab = parseInnerPath(innerPath)

  const handleSubtabChange = (tab: string) => {
    onPathChange(tab)
    // Clear notifications for the tab being viewed
    if (tab === "resources") {
      setResourceUpdates(new Map())
    }
    // Note: We don't clear pending requests when switching tabs - they need user action
  }

  // Handle resource subscription update (called from mcp-client)
  const handleResourceUpdate = useCallback((uri: string, content: ReadResourceResult) => {
    // Only track updates when not viewing resources tab
    if (activeSubtab !== "resources") {
      setResourceUpdates(prev => {
        const next = new Map(prev)
        next.set(uri, content)
        return next
      })
    }
  }, [activeSubtab])

  // Handle marking a single resource as viewed
  const handleResourceViewed = useCallback((uri: string) => {
    setResourceUpdates(prev => {
      const next = new Map(prev)
      next.delete(uri)
      return next
    })
  }, [])

  // Determine if target is gateway or a specific server
  const isGatewayTarget = mode === "client" || mode === "all"

  // Fetch client API key when client selection changes
  useEffect(() => {
    const fetchClientKey = async () => {
      if (mode === "client" && selectedClientId) {
        try {
          const secret = await invoke<string>("get_client_value", { id: selectedClientId })
          setClientApiKey(secret)
        } catch (error) {
          console.error("Failed to get client API key:", error)
          setClientApiKey(null)
        }
      }
    }
    fetchClientKey()
  }, [mode, selectedClientId])

  // Initialize data
  useEffect(() => {
    const init = async () => {
      try {
        const [config, servers, testToken, clientsList] = await Promise.all([
          invoke<ServerConfig>("get_server_config"),
          invoke<McpServer[]>("list_mcp_servers"),
          invoke<string>("get_internal_test_token"),
          invoke<McpClient[]>("list_clients"),
        ])

        setServerPort(config.actual_port ?? config.port)
        const enabledServers = servers.filter((s) => s.enabled)
        setMcpServers(enabledServers)
        if (enabledServers.length > 0) {
          setSelectedServerId(enabledServers[0].id)
        }
        setInternalTestToken(testToken)
        const enabledClients = clientsList.filter(c => c.enabled)
        setClients(enabledClients)
        if (enabledClients.length > 0) {
          setSelectedClientId(enabledClients[0].id)
        }
      } catch (error) {
        console.error("Failed to initialize MCP tab:", error)
      }
    }
    init()

    // Listen for MCP server status changes
    const unsubscribe = listen("mcp-servers-changed", () => {
      invoke<McpServer[]>("list_mcp_servers").then((servers) => {
        setMcpServers(servers.filter((s) => s.enabled))
      })
    })

    return () => {
      unsubscribe.then((fn) => fn())
      // Cleanup: disconnect client on unmount
      if (mcpClientRef.current) {
        mcpClientRef.current.disconnect()
      }
    }
  }, [])

  // Handle connection state changes from the client
  const handleStateChange = useCallback((state: McpConnectionState) => {
    setConnectionState(state)
  }, [])

  // Handler for sampling requests from MCP servers
  const handleSamplingRequest = useCallback(
    (params: CreateMessageRequest["params"]): Promise<CreateMessageResult> => {
      return new Promise((resolve, reject) => {
        const id = `sampling-${++requestIdCounter.current}`
        const request: PendingSamplingRequest = {
          id,
          params,
          timestamp: new Date(),
          resolve,
          reject,
        }
        setPendingSamplingRequests((prev) => [...prev, request])
      })
    },
    []
  )

  // Handler for elicitation requests from MCP servers
  const handleElicitationRequest = useCallback(
    (params: ElicitRequest["params"]): Promise<{ action: "accept" | "decline"; content?: Record<string, unknown> }> => {
      return new Promise((resolve, reject) => {
        const id = `elicitation-${++requestIdCounter.current}`
        const request: PendingElicitationRequest = {
          id,
          params,
          timestamp: new Date(),
          resolve,
          reject,
        }
        setPendingElicitationRequests((prev) => [...prev, request])
      })
    },
    []
  )

  // Callback to resolve a sampling request
  const resolveSamplingRequest = useCallback((id: string, result: CreateMessageResult) => {
    setPendingSamplingRequests((prev) => {
      const request = prev.find((r) => r.id === id)
      if (request) {
        request.resolve(result)
      }
      return prev.filter((r) => r.id !== id)
    })
  }, [])

  // Callback to reject a sampling request
  const rejectSamplingRequest = useCallback((id: string, error: string) => {
    setPendingSamplingRequests((prev) => {
      const request = prev.find((r) => r.id === id)
      if (request) {
        request.reject(new Error(error))
      }
      return prev.filter((r) => r.id !== id)
    })
  }, [])

  // Callback to resolve an elicitation request
  const resolveElicitationRequest = useCallback(
    (id: string, result: { action: "accept" | "decline"; content?: Record<string, unknown> }) => {
      setPendingElicitationRequests((prev) => {
        const request = prev.find((r) => r.id === id)
        if (request) {
          request.resolve(result)
        }
        return prev.filter((r) => r.id !== id)
      })
    },
    []
  )

  const handleConnect = async () => {
    if (!serverPort) return

    // Determine token based on mode
    const token = mode === "client" ? clientApiKey : internalTestToken
    if (!token) return

    // Disconnect existing client if any
    if (mcpClientRef.current) {
      await mcpClientRef.current.disconnect()
    }

    // Create new client with sampling/elicitation callbacks
    const client = createMcpClient(
      {
        serverPort,
        clientToken: token,
        serverId: mode === "direct" ? selectedServerId : undefined,
        transportType: "sse",
        // Only enable deferred loading for gateway modes (all / client)
        deferredLoading: isGatewayTarget ? deferredLoading : undefined,
      },
      {
        onStateChange: handleStateChange,
        onSamplingRequest: handleSamplingRequest,
        onElicitationRequest: handleElicitationRequest,
      }
    )

    mcpClientRef.current = client

    try {
      await client.connect()
    } catch (error) {
      console.error("Failed to connect:", error)
      // Error state is already set via handleStateChange
    }
  }

  const handleDisconnect = async () => {
    // Reject any pending requests before disconnecting
    pendingSamplingRequests.forEach((r) => r.reject(new Error("Disconnected")))
    pendingElicitationRequests.forEach((r) => r.reject(new Error("Disconnected")))

    if (mcpClientRef.current) {
      await mcpClientRef.current.disconnect()
      mcpClientRef.current = null
    }
    // Clear subscription state on disconnect
    setSubscribedUris(new Set())
    setResourceUpdates(new Map())
    setPendingSamplingRequests([])
    setPendingElicitationRequests([])
  }

  const getEndpointUrl = () => {
    if (!serverPort) return null
    if (mode === "direct") {
      return `http://localhost:${serverPort}/mcp/${selectedServerId}`
    }
    return `http://localhost:${serverPort}/`
  }

  const { isConnected, isConnecting, error: connectionError, capabilities } = connectionState

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Connection Settings */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base">MCP & Skill Connection</CardTitle>
              <p className="text-sm text-muted-foreground">
                Test MCP servers and skills through a client, the unified gateway, or individually
              </p>
            </div>
            <Badge variant={isConnected ? "success" : isConnecting ? "outline" : "secondary"}>
              {isConnected ? "Connected" : isConnecting ? "Connecting..." : "Disconnected"}
            </Badge>
          </div>
        </CardHeader>
        <CardContent>
          <div className="flex flex-col gap-4">
            <div className="flex items-center gap-4">
              <RadioGroup
                value={mode}
                onValueChange={(v: string) => setMode(v as McpTestMode)}
                className="flex gap-4"
                disabled={isConnected || isConnecting}
              >
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="client" id="mcp-mode-client" disabled={isConnected || isConnecting} />
                  <Label htmlFor="mcp-mode-client" className="flex items-center gap-2 cursor-pointer">
                    <Users className="h-4 w-4" />
                    Against Client
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="all" id="mcp-mode-all" disabled={isConnected || isConnecting} />
                  <Label htmlFor="mcp-mode-all" className="flex items-center gap-2 cursor-pointer">
                    <Globe className="h-4 w-4" />
                    All MCPs & Skills
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value="direct" id="mcp-mode-direct" disabled={isConnected || isConnecting} />
                  <Label htmlFor="mcp-mode-direct" className="flex items-center gap-2 cursor-pointer">
                    <Zap className="h-4 w-4" />
                    Direct MCP/Skill
                  </Label>
                </div>
              </RadioGroup>
            </div>

            <div className="flex items-center gap-4">
              {/* Mode-specific selector */}
              {mode === "client" && (
                <Select value={selectedClientId} onValueChange={setSelectedClientId} disabled={isConnected || isConnecting}>
                  <SelectTrigger className="w-[250px]">
                    <SelectValue placeholder="Select a client" />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((client) => (
                      <SelectItem key={client.id} value={client.id}>
                        {client.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

              {mode === "direct" && (
                <Select value={selectedServerId} onValueChange={setSelectedServerId} disabled={isConnected || isConnecting}>
                  <SelectTrigger className="w-[280px]">
                    <SelectValue placeholder="Select a server" />
                  </SelectTrigger>
                  <SelectContent>
                    {mcpServers.map((server) => (
                      <SelectItem key={server.id} value={server.id}>
                        {server.name} ({server.transport_type})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

              {/* Deferred Loading toggle - for gateway modes (all / client) */}
              {isGatewayTarget && (
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="deferred-loading"
                    checked={deferredLoading}
                    onCheckedChange={(checked) => setDeferredLoading(checked === true)}
                    disabled={isConnected || isConnecting}
                  />
                  <Label htmlFor="deferred-loading" className="text-sm cursor-pointer">
                    Deferred Loading
                  </Label>
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Info className="h-3.5 w-3.5 text-muted-foreground cursor-help" />
                      </TooltipTrigger>
                      <TooltipContent side="bottom" className="max-w-[300px]">
                        <p>
                          When enabled, tools and resources are loaded on-demand via search
                          instead of all at once. Reduces token consumption for large catalogs.
                        </p>
                      </TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                </div>
              )}

              {/* Connection buttons */}
              <div className="flex items-center gap-2 ml-auto">
                {getEndpointUrl() && (
                  <code className="text-xs text-muted-foreground bg-muted px-2 py-1 rounded">
                    {getEndpointUrl()}
                  </code>
                )}
                {!isConnected ? (
                  isConnecting ? (
                    <Button variant="outline" onClick={handleDisconnect}>
                      <X className="h-4 w-4 mr-1" />
                      Cancel
                    </Button>
                  ) : (
                    <Button
                      onClick={handleConnect}
                      disabled={
                        (mode === "client" && (!selectedClientId || !clientApiKey)) ||
                        (mode === "all" && !internalTestToken) ||
                        (mode === "direct" && (!internalTestToken || !selectedServerId || mcpServers.length === 0))
                      }
                    >
                      Connect
                    </Button>
                  )
                ) : (
                  <Button variant="outline" onClick={handleDisconnect}>
                    Disconnect
                  </Button>
                )}
              </div>
            </div>
          </div>

          {connectionError && (
            <div className="flex items-center gap-2 text-destructive text-sm mt-4">
              <AlertCircle className="h-4 w-4" />
              {connectionError}
            </div>
          )}
        </CardContent>
      </Card>

      {/* MCP Subtabs - only shown when connected */}
      {isConnected && (
      <Card className="flex flex-col flex-1 min-h-0">
        <Tabs
          value={activeSubtab}
          onValueChange={handleSubtabChange}
          className="flex flex-col flex-1 min-h-0"
        >
          <CardHeader className="pb-0 flex-shrink-0">
            <TooltipProvider>
              <TabsList className="w-fit">
                <TabsTrigger value="connection" className="flex items-center gap-1">
                  <Info className="h-3 w-3" />
                  Connection
                </TabsTrigger>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <TabsTrigger
                        value="tools"
                        className="flex items-center gap-1"
                        disabled={!capabilities?.tools}
                      >
                        <Wrench className="h-3 w-3" />
                        Tools
                      </TabsTrigger>
                    </span>
                  </TooltipTrigger>
                  {!capabilities?.tools && (
                    <TooltipContent>Server does not support tools</TooltipContent>
                  )}
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="relative">
                      <TabsTrigger
                        value="resources"
                        className="flex items-center gap-1"
                        disabled={!capabilities?.resources}
                      >
                        <FileText className="h-3 w-3" />
                        Resources
                        {resourceUpdates.size > 0 && (
                          <Circle className="h-2 w-2 fill-primary text-primary absolute -top-0.5 -right-0.5" />
                        )}
                      </TabsTrigger>
                    </span>
                  </TooltipTrigger>
                  {!capabilities?.resources && (
                    <TooltipContent>Server does not support resources</TooltipContent>
                  )}
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <TabsTrigger
                        value="prompts"
                        className="flex items-center gap-1"
                        disabled={!capabilities?.prompts}
                      >
                        <MessageSquare className="h-3 w-3" />
                        Prompts
                      </TabsTrigger>
                    </span>
                  </TooltipTrigger>
                  {!capabilities?.prompts && (
                    <TooltipContent>Server does not support prompts</TooltipContent>
                  )}
                </Tooltip>
                <TabsTrigger value="sampling" className="flex items-center gap-1 relative">
                  <Radio className="h-3 w-3" />
                  Sampling
                  {pendingSamplingRequests.length > 0 && (
                    <Circle className="h-2 w-2 fill-primary text-primary absolute -top-0.5 -right-0.5" />
                  )}
                </TabsTrigger>
                <TabsTrigger value="elicitation" className="flex items-center gap-1 relative">
                  <HelpCircle className="h-3 w-3" />
                  Elicitation
                  {pendingElicitationRequests.length > 0 && (
                    <Circle className="h-2 w-2 fill-primary text-primary absolute -top-0.5 -right-0.5" />
                  )}
                </TabsTrigger>
              </TabsList>
            </TooltipProvider>
          </CardHeader>

          <CardContent className="flex-1 min-h-0 pt-4">
            <TabsContent value="connection" className="h-full m-0">
              <ConnectionInfoPanel connectionState={connectionState} />
            </TabsContent>

            <TabsContent value="tools" className="h-full m-0">
              <ToolsPanel
                mcpClient={mcpClientRef.current}
                isConnected={isConnected}
                toolState={toolState}
                onToolStateChange={setToolState}
              />
            </TabsContent>

            <TabsContent value="resources" className="h-full m-0">
              <ResourcesPanel
                mcpClient={mcpClientRef.current}
                isConnected={isConnected}
                subscribedUris={subscribedUris}
                onSubscribedUrisChange={setSubscribedUris}
                resourceUpdates={resourceUpdates}
                onResourceUpdate={handleResourceUpdate}
                onResourceViewed={handleResourceViewed}
                resourceState={resourceState}
                onResourceStateChange={setResourceState}
              />
            </TabsContent>

            <TabsContent value="prompts" className="h-full m-0">
              <PromptsPanel
                mcpClient={mcpClientRef.current}
                isConnected={isConnected}
                promptState={promptState}
                onPromptStateChange={setPromptState}
              />
            </TabsContent>

            <TabsContent value="sampling" className="h-full m-0">
              <SamplingPanel
                isConnected={isConnected}
                pendingRequests={pendingSamplingRequests}
                onResolve={resolveSamplingRequest}
                onReject={rejectSamplingRequest}
                autoApprove={autoApproveSampling}
                onAutoApproveChange={setAutoApproveSampling}
                samplingState={samplingState}
                onSamplingStateChange={setSamplingState}
              />
            </TabsContent>

            <TabsContent value="elicitation" className="h-full m-0">
              <ElicitationPanel
                isConnected={isConnected}
                pendingRequests={pendingElicitationRequests}
                onResolve={resolveElicitationRequest}
                elicitationState={elicitationState}
                onElicitationStateChange={setElicitationState}
              />
            </TabsContent>
          </CardContent>
        </Tabs>
      </Card>
      )}
    </div>
  )
}
