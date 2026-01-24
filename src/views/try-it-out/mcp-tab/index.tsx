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
import { Wrench, FileText, MessageSquare, Radio, HelpCircle, AlertCircle, X, Circle, Info } from "lucide-react"
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

interface McpServer {
  id: string
  name: string
  transport_type: string
  enabled: boolean
  status?: string
}

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

// Target can be "gateway" for unified or a server ID for individual
type McpTarget = "gateway" | string

export function McpTab({ innerPath, onPathChange }: McpTabProps) {
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [selectedTarget, setSelectedTarget] = useState<McpTarget>("gateway")
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
  const isGatewayTarget = selectedTarget === "gateway"
  const selectedServerId = isGatewayTarget ? "" : selectedTarget

  // Initialize data
  useEffect(() => {
    const init = async () => {
      try {
        const [config, servers, testToken] = await Promise.all([
          invoke<ServerConfig>("get_server_config"),
          invoke<McpServer[]>("list_mcp_servers"),
          invoke<string>("get_internal_test_token"),
        ])

        setServerPort(config.actual_port ?? config.port)
        setMcpServers(servers.filter((s) => s.enabled))
        setInternalTestToken(testToken)
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
    if (!serverPort || !internalTestToken) return

    // Disconnect existing client if any
    if (mcpClientRef.current) {
      await mcpClientRef.current.disconnect()
    }

    // Create new client with sampling/elicitation callbacks
    const client = createMcpClient(
      {
        serverPort,
        clientToken: internalTestToken,
        serverId: isGatewayTarget ? undefined : selectedServerId,
        transportType: "sse",
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
    if (isGatewayTarget) {
      return `http://localhost:${serverPort}/`
    } else {
      return `http://localhost:${serverPort}/mcp/${selectedServerId}`
    }
  }

  const { isConnected, isConnecting, error: connectionError, capabilities } = connectionState

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Connection Settings */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base">MCP Connection</CardTitle>
              <p className="text-sm text-muted-foreground">
                Test MCP servers through the unified gateway or individually
              </p>
            </div>
            <Badge variant={isConnected ? "success" : isConnecting ? "outline" : "secondary"}>
              {isConnected ? "Connected" : isConnecting ? "Connecting..." : "Disconnected"}
            </Badge>
          </div>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2">
              <Label>Target:</Label>
              <Select
                value={selectedTarget}
                onValueChange={setSelectedTarget}
                disabled={isConnected || isConnecting}
              >
                <SelectTrigger className="w-[280px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="gateway">
                    Unified Gateway (all servers)
                  </SelectItem>
                  {mcpServers.map((server) => (
                    <SelectItem key={server.id} value={server.id}>
                      {server.name} ({server.transport_type})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

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
                    disabled={!internalTestToken || (mcpServers.length === 0 && !isGatewayTarget)}
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
              />
            </TabsContent>

            <TabsContent value="prompts" className="h-full m-0">
              <PromptsPanel
                mcpClient={mcpClientRef.current}
                isConnected={isConnected}
              />
            </TabsContent>

            <TabsContent value="sampling" className="h-full m-0">
              <SamplingPanel
                isConnected={isConnected}
                pendingRequests={pendingSamplingRequests}
                onResolve={resolveSamplingRequest}
                onReject={rejectSamplingRequest}
              />
            </TabsContent>

            <TabsContent value="elicitation" className="h-full m-0">
              <ElicitationPanel
                isConnected={isConnected}
                pendingRequests={pendingElicitationRequests}
                onResolve={resolveElicitationRequest}
              />
            </TabsContent>
          </CardContent>
        </Tabs>
      </Card>
      )}
    </div>
  )
}
