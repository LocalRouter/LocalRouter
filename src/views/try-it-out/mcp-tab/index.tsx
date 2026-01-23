import { useState, useEffect } from "react"
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
import { Wrench, FileText, MessageSquare, Radio, HelpCircle, RefreshCw, AlertCircle } from "lucide-react"
import { ToolsPanel } from "./tools-panel"
import { ResourcesPanel } from "./resources-panel"
import { PromptsPanel } from "./prompts-panel"
import { SamplingPanel } from "./sampling-panel"
import { ElicitationPanel } from "./elicitation-panel"

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
  const [isConnected, setIsConnected] = useState(false)
  const [isConnecting, setIsConnecting] = useState(false)
  const [connectionError, setConnectionError] = useState<string | null>(null)
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // Parse inner path to get subtab
  const parseInnerPath = (path: string | null) => {
    if (!path) return "tools"
    const parts = path.split("/")
    return parts[0] || "tools"
  }

  const activeSubtab = parseInnerPath(innerPath)

  const handleSubtabChange = (tab: string) => {
    onPathChange(tab)
  }

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
    }
  }, [])

  const handleConnect = async () => {
    setIsConnecting(true)
    setConnectionError(null)

    try {
      // For now, just validate the connection
      // The actual MCP SDK integration would happen here
      await new Promise((resolve) => setTimeout(resolve, 500))
      setIsConnected(true)
    } catch (error) {
      setConnectionError(error instanceof Error ? error.message : "Connection failed")
    } finally {
      setIsConnecting(false)
    }
  }

  const handleDisconnect = () => {
    setIsConnected(false)
    setConnectionError(null)
  }

  const getEndpointUrl = () => {
    if (!serverPort) return null
    if (isGatewayTarget) {
      return `http://localhost:${serverPort}/`
    } else {
      return `http://localhost:${serverPort}/mcp/${selectedServerId}`
    }
  }

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
            <Badge variant={isConnected ? "success" : "secondary"}>
              {isConnected ? "Connected" : "Disconnected"}
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
                disabled={isConnected}
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
                <Button
                  onClick={handleConnect}
                  disabled={isConnecting || !internalTestToken || (mcpServers.length === 0 && !isGatewayTarget)}
                >
                  {isConnecting ? (
                    <>
                      <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                      Connecting...
                    </>
                  ) : (
                    "Connect"
                  )}
                </Button>
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

      {/* MCP Subtabs */}
      <Card className="flex flex-col flex-1 min-h-0">
        <Tabs
          value={activeSubtab}
          onValueChange={handleSubtabChange}
          className="flex flex-col flex-1 min-h-0"
        >
          <CardHeader className="pb-0 flex-shrink-0">
            <TabsList className="w-fit">
              <TabsTrigger value="tools" className="flex items-center gap-1">
                <Wrench className="h-3 w-3" />
                Tools
              </TabsTrigger>
              <TabsTrigger value="resources" className="flex items-center gap-1">
                <FileText className="h-3 w-3" />
                Resources
              </TabsTrigger>
              <TabsTrigger value="prompts" className="flex items-center gap-1">
                <MessageSquare className="h-3 w-3" />
                Prompts
              </TabsTrigger>
              <TabsTrigger value="sampling" className="flex items-center gap-1">
                <Radio className="h-3 w-3" />
                Sampling
              </TabsTrigger>
              <TabsTrigger value="elicitation" className="flex items-center gap-1">
                <HelpCircle className="h-3 w-3" />
                Elicitation
              </TabsTrigger>
            </TabsList>
          </CardHeader>

          <CardContent className="flex-1 min-h-0 pt-4">
            <TabsContent value="tools" className="h-full m-0">
              <ToolsPanel
                serverPort={serverPort}
                clientToken={internalTestToken}
                isGateway={isGatewayTarget}
                selectedServer={selectedServerId}
                isConnected={isConnected}
              />
            </TabsContent>

            <TabsContent value="resources" className="h-full m-0">
              <ResourcesPanel
                serverPort={serverPort}
                clientToken={internalTestToken}
                isGateway={isGatewayTarget}
                selectedServer={selectedServerId}
                isConnected={isConnected}
              />
            </TabsContent>

            <TabsContent value="prompts" className="h-full m-0">
              <PromptsPanel
                serverPort={serverPort}
                clientToken={internalTestToken}
                isGateway={isGatewayTarget}
                selectedServer={selectedServerId}
                isConnected={isConnected}
              />
            </TabsContent>

            <TabsContent value="sampling" className="h-full m-0">
              <SamplingPanel
                serverPort={serverPort}
                clientToken={internalTestToken}
                isGateway={isGatewayTarget}
                selectedServer={selectedServerId}
                isConnected={isConnected}
              />
            </TabsContent>

            <TabsContent value="elicitation" className="h-full m-0">
              <ElicitationPanel
                serverPort={serverPort}
                clientToken={internalTestToken}
                isGateway={isGatewayTarget}
                selectedServer={selectedServerId}
                isConnected={isConnected}
              />
            </TabsContent>
          </CardContent>
        </Tabs>
      </Card>
    </div>
  )
}
