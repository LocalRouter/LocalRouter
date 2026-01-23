import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Database, Plus } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { Input } from "@/components/ui/Input"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/Modal"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import LegacySelect from "@/components/ui/Select"
import KeyValueInput from "@/components/ui/KeyValueInput"
import {
  EntityActions,
  commonActions,
  createToggleAction,
} from "@/components/shared/entity-actions"
import { MetricsChart } from "@/components/shared/metrics-chart"
import { McpServerTemplates, McpServerTemplate } from "@/components/mcp/McpServerTemplates"
import { McpOAuthModal } from "@/components/mcp/McpOAuthModal"
import { cn } from "@/lib/utils"

interface McpAuthConfig {
  type: string
  [key: string]: unknown
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
  transport: string
  transport_config: unknown
  auth_config: McpAuthConfig | null
  proxy_url: string
  gateway_url: string
}

interface McpServersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  refreshTrigger?: number
}

export function McpServersPanel({
  selectedId,
  onSelect,
  refreshTrigger = 0,
}: McpServersPanelProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")

  // OAuth status state
  const [oauthStatus, setOauthStatus] = useState<Record<string, boolean>>({})
  const [showOAuthModal, setShowOAuthModal] = useState(false)

  // OAuth setup state
  const [showOAuthSetup, setShowOAuthSetup] = useState(false)
  const [oauthSetupClientId, setOauthSetupClientId] = useState("")
  const [oauthSetupClientSecret, setOauthSetupClientSecret] = useState("")
  const [oauthDiscovery, setOauthDiscovery] = useState<{ auth_url: string; token_url: string; scopes: string[] } | null>(null)
  const [isDiscovering, setIsDiscovering] = useState(false)
  const [isSavingOAuth, setIsSavingOAuth] = useState(false)

  // Delete confirmation state
  const [serverToDelete, setServerToDelete] = useState<McpServer | null>(null)

  // Create modal state
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [selectedTemplate, setSelectedTemplate] = useState<McpServerTemplate | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [createTab, setCreateTab] = useState<"templates" | "manual">("templates")

  // Form state
  const [serverName, setServerName] = useState("")
  const [transportType, setTransportType] = useState<"Stdio" | "Sse">("Stdio")
  const [command, setCommand] = useState("")
  const [args, setArgs] = useState("")
  const [envVars, setEnvVars] = useState<Record<string, string>>({})
  const [url, setUrl] = useState("")
  const [headers, setHeaders] = useState<Record<string, string>>({})

  // Auth config state
  const [authMethod, setAuthMethod] = useState<"none" | "bearer" | "oauth_browser">("none")
  const [bearerToken, setBearerToken] = useState("")

  useEffect(() => {
    loadServers()

    const unsubscribe = listen("mcp-servers-changed", () => {
      loadServers()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  const loadServers = async () => {
    try {
      setLoading(true)
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
    } finally {
      setLoading(false)
    }
  }

  const resetForm = () => {
    setServerName("")
    setTransportType("Stdio")
    setCommand("")
    setArgs("")
    setEnvVars({})
    setUrl("")
    setHeaders({})
    setAuthMethod("none")
    setBearerToken("")
    setSelectedTemplate(null)
    setCreateTab("templates")
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    setSelectedTemplate(template)
    setServerName(template.name)
    setTransportType(template.transport)

    if (template.transport === "Stdio" && template.command) {
      setCommand(template.command)
      if (template.args) {
        setArgs(template.args.join("\n"))
      }
    } else if (template.transport === "Sse" && template.url) {
      setUrl(template.url)
    }

    if (template.authMethod === "oauth_browser") {
      setAuthMethod("oauth_browser")
    } else if (template.authMethod === "none" || template.authMethod === "bearer") {
      setAuthMethod(template.authMethod)
    } else {
      setAuthMethod("none")
    }
  }

  const handleCreateServer = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsCreating(true)

    try {
      // Parse transport config based on type
      let transportConfig
      if (transportType === "Stdio") {
        const argsList = args.trim()
          ? args.split(/[\n,]/).map((a) => a.trim()).filter((a) => a)
          : []

        transportConfig = {
          type: "stdio",
          command,
          args: argsList,
          env: envVars,
        }
      } else {
        transportConfig = {
          type: "http_sse",
          url,
          headers: headers,
        }
      }

      // Build auth config based on auth method
      let authConfig = null
      if (authMethod === "bearer") {
        authConfig = {
          type: "bearer_token",
          token: bearerToken,
        }
      } else if (authMethod === "oauth_browser") {
        // Just mark as oauth_browser - credentials will be configured on detail page
        // after OAuth discovery from the MCP server
        authConfig = {
          type: "oauth_browser",
        }
      }

      await invoke("create_mcp_server", {
        name: serverName || null,
        transport: transportType,
        transportConfig,
        authConfig,
      })

      toast.success("MCP server created")
      await loadServers()
      setShowCreateModal(false)
      resetForm()
    } catch (error) {
      console.error("Failed to create MCP server:", error)
      toast.error(`Error creating MCP server: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  const handleToggle = async (server: McpServer) => {
    try {
      await invoke("toggle_mcp_server_enabled", {
        serverId: server.id,
        enabled: !server.enabled,
      })
      toast.success(`Server ${server.enabled ? "disabled" : "enabled"}`)
      loadServers()
    } catch (error) {
      toast.error("Failed to update server")
    }
  }

  const handleDelete = async () => {
    if (!serverToDelete) return
    try {
      await invoke("delete_mcp_server", { serverId: serverToDelete.id })
      toast.success("Server deleted")
      if (selectedId === serverToDelete.id) {
        onSelect(null)
      }
      loadServers()
    } catch (error) {
      toast.error("Failed to delete server")
    } finally {
      setServerToDelete(null)
    }
  }

  const checkOAuthStatus = async (serverId: string) => {
    try {
      const isValid = await invoke<boolean>("test_mcp_oauth_connection", { serverId })
      setOauthStatus((prev) => ({ ...prev, [serverId]: isValid }))
    } catch {
      setOauthStatus((prev) => ({ ...prev, [serverId]: false }))
    }
  }

  const handleOAuthSuccess = () => {
    if (selectedId) {
      checkOAuthStatus(selectedId)
    }
    setShowOAuthModal(false)
    toast.success("OAuth authentication successful")
  }

  const handleRevokeOAuth = async (serverId: string) => {
    try {
      await invoke("revoke_mcp_oauth_tokens", { serverId })
      setOauthStatus((prev) => ({ ...prev, [serverId]: false }))
      toast.success("OAuth tokens revoked")
    } catch (error) {
      toast.error("Failed to revoke OAuth tokens")
    }
  }

  // Check if OAuth is fully configured (has client_id)
  const isOAuthConfigured = (server: McpServer) => {
    if (server.auth_config?.type !== "oauth_browser") return false
    // Check if client_id is present (indicating OAuth is configured)
    return !!(server.auth_config as { client_id?: string }).client_id
  }

  // Start OAuth setup flow
  const handleStartOAuthSetup = async () => {
    if (!selectedServer) return

    setIsDiscovering(true)
    setOauthDiscovery(null)
    setOauthSetupClientId("")
    setOauthSetupClientSecret("")

    try {
      // Get the server's base URL from transport config
      const transportConfig = selectedServer.transport_config as { url?: string }
      if (!transportConfig.url) {
        toast.error("Server URL not found")
        return
      }

      // Extract base URL (remove any path)
      const url = new URL(transportConfig.url)
      const baseUrl = `${url.protocol}//${url.host}`

      // Discover OAuth endpoints
      const discovery = await invoke<{ auth_url: string; token_url: string; scopes: string[] } | null>(
        "discover_mcp_oauth_endpoints",
        { baseUrl }
      )

      if (discovery) {
        setOauthDiscovery(discovery)
        setShowOAuthSetup(true)
      } else {
        toast.error("This MCP server does not support OAuth")
      }
    } catch (error) {
      toast.error(`Failed to discover OAuth: ${error}`)
    } finally {
      setIsDiscovering(false)
    }
  }

  // Save OAuth credentials
  const handleSaveOAuthCredentials = async () => {
    if (!selectedServer || !oauthDiscovery) return

    setIsSavingOAuth(true)
    try {
      await invoke("update_mcp_server", {
        serverId: selectedServer.id,
        updates: {
          auth_config: {
            type: "oauth_browser",
            client_id: oauthSetupClientId,
            client_secret: oauthSetupClientSecret,
            auth_url: oauthDiscovery.auth_url,
            token_url: oauthDiscovery.token_url,
            scopes: oauthDiscovery.scopes,
            redirect_uri: "http://localhost:8080/callback",
          },
        },
      })

      toast.success("OAuth credentials saved")
      setShowOAuthSetup(false)
      await loadServers()
    } catch (error) {
      toast.error(`Failed to save OAuth credentials: ${error}`)
    } finally {
      setIsSavingOAuth(false)
    }
  }

  // Check OAuth status when a server with OAuth browser auth is selected
  useEffect(() => {
    if (selectedId) {
      const server = servers.find((s) => s.id === selectedId)
      if (server?.auth_config?.type === "oauth_browser") {
        checkOAuthStatus(selectedId)
      }
    }
  }, [selectedId, servers])

  const filteredServers = servers.filter((s) =>
    s.name.toLowerCase().includes(search.toLowerCase()) ||
    s.id.toLowerCase().includes(search.toLowerCase())
  )

  const selectedServer = servers.find((s) => s.id === selectedId)

  return (
    <>
    <ResizablePanelGroup direction="horizontal" className="h-full rounded-lg border">
      {/* List Panel */}
      <ResizablePanel defaultSize={35} minSize={25}>
        <div className="flex flex-col h-full">
          <div className="p-4 border-b">
            <div className="flex gap-2">
              <Input
                placeholder="Search MCP servers..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="flex-1"
              />
              <Button
                size="icon"
                onClick={() => setShowCreateModal(true)}
                title="Add MCP Server"
              >
                <Plus className="h-4 w-4" />
              </Button>
            </div>
          </div>
          <ScrollArea className="flex-1">
            <div className="p-2 space-y-1">
              {loading ? (
                <p className="text-sm text-muted-foreground p-4">Loading...</p>
              ) : filteredServers.length === 0 ? (
                <p className="text-sm text-muted-foreground p-4">No MCP servers found</p>
              ) : (
                filteredServers.map((server) => (
                  <div
                    key={server.id}
                    onClick={() => onSelect(server.id)}
                    className={cn(
                      "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                      selectedId === server.id ? "bg-accent" : "hover:bg-muted"
                    )}
                  >
                    <Database className="h-4 w-4 text-muted-foreground" />
                    <div className="flex-1 min-w-0">
                      <p className="font-medium truncate">{server.name}</p>
                      <p className="text-xs text-muted-foreground capitalize">
                        {server.transport === "Stdio" ? "STDIO" : "HTTP SSE"}
                      </p>
                    </div>
                    {!server.enabled && (
                      <Badge variant="secondary" className="text-xs">Off</Badge>
                    )}
                  </div>
                ))
              )}
            </div>
          </ScrollArea>
        </div>
      </ResizablePanel>

      <ResizableHandle withHandle />

      {/* Detail Panel */}
      <ResizablePanel defaultSize={65}>
        {selectedServer ? (
          <ScrollArea className="h-full">
            <div className="p-6 space-y-6">
              <div className="flex items-start justify-between">
                <div>
                  <h2 className="text-xl font-bold">{selectedServer.name}</h2>
                  <p className="text-sm text-muted-foreground capitalize">
                    {selectedServer.transport === "Stdio" ? "STDIO" : "HTTP SSE"} transport
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <Badge variant={selectedServer.enabled ? "success" : "secondary"}>
                    {selectedServer.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                  <EntityActions
                    actions={[
                      createToggleAction(selectedServer.enabled, () =>
                        handleToggle(selectedServer)
                      ),
                      commonActions.delete(() => setServerToDelete(selectedServer)),
                    ]}
                  />
                </div>
              </div>

              {/* Connection Info */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-sm">Connection Details</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div>
                    <p className="text-sm text-muted-foreground">Proxy URL</p>
                    <code className="text-sm break-all">{selectedServer.proxy_url}</code>
                  </div>
                  <div>
                    <p className="text-sm text-muted-foreground">Gateway URL</p>
                    <code className="text-sm break-all">{selectedServer.gateway_url}</code>
                  </div>
                  {selectedServer.auth_config?.type && (
                    <div>
                      <p className="text-sm text-muted-foreground">Authentication</p>
                      <p className="text-sm capitalize">{selectedServer.auth_config?.type.replace(/_/g, " ")}</p>
                    </div>
                  )}
                </CardContent>
              </Card>

              {/* OAuth Status */}
              {selectedServer.auth_config?.type === "oauth_browser" && (
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm">OAuth Authentication</CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-4">
                    {!isOAuthConfigured(selectedServer) ? (
                      // OAuth not yet configured - show setup button
                      <>
                        <p className="text-sm text-muted-foreground">
                          OAuth credentials are not configured. Click Setup to discover OAuth
                          endpoints and enter your credentials.
                        </p>
                        <Button
                          size="sm"
                          onClick={handleStartOAuthSetup}
                          disabled={isDiscovering}
                        >
                          {isDiscovering ? "Discovering..." : "Setup OAuth"}
                        </Button>
                      </>
                    ) : (
                      // OAuth configured - show status and authenticate button
                      <>
                        <div className="flex items-center justify-between">
                          <div>
                            <p className="text-sm font-medium">
                              {oauthStatus[selectedServer.id]
                                ? "Authenticated"
                                : "Not authenticated"}
                            </p>
                            <p className="text-xs text-muted-foreground">
                              {oauthStatus[selectedServer.id]
                                ? "OAuth tokens are valid and ready to use"
                                : "Click Authenticate to complete browser login"}
                            </p>
                          </div>
                          <Badge variant={oauthStatus[selectedServer.id] ? "success" : "secondary"}>
                            {oauthStatus[selectedServer.id] ? "Active" : "Inactive"}
                          </Badge>
                        </div>
                        <div className="flex gap-2">
                          <Button
                            size="sm"
                            variant={oauthStatus[selectedServer.id] ? "secondary" : "default"}
                            onClick={() => setShowOAuthModal(true)}
                          >
                            {oauthStatus[selectedServer.id] ? "Re-authenticate" : "Authenticate"}
                          </Button>
                          {oauthStatus[selectedServer.id] && (
                            <>
                              <Button
                                size="sm"
                                variant="secondary"
                                onClick={() => checkOAuthStatus(selectedServer.id)}
                              >
                                Test
                              </Button>
                              <Button
                                size="sm"
                                variant="destructive"
                                onClick={() => handleRevokeOAuth(selectedServer.id)}
                              >
                                Revoke
                              </Button>
                            </>
                          )}
                        </div>
                      </>
                    )}
                  </CardContent>
                </Card>
              )}

              {/* Metrics */}
              <MetricsChart
                title="MCP Metrics"
                scope="server"
                scopeId={selectedServer.id}
                defaultMetricType="requests"
                metricOptions={[
                  { id: "requests", label: "Requests" },
                  { id: "latency", label: "Latency" },
                  { id: "successrate", label: "Success" },
                ]}
                refreshTrigger={refreshTrigger}
                height={250}
                dataSource="mcp"
              />
            </div>
          </ScrollArea>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select an MCP server to view details</p>
          </div>
        )}
      </ResizablePanel>
    </ResizablePanelGroup>

    {/* Create MCP Server Modal */}
    <Dialog
      open={showCreateModal}
      onOpenChange={(open) => {
        if (!open) {
          setShowCreateModal(false)
          resetForm()
        }
      }}
    >
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add MCP Server</DialogTitle>
        </DialogHeader>

        <Tabs value={createTab} onValueChange={(v) => setCreateTab(v as "templates" | "manual")}>
          <TabsList className="grid w-full grid-cols-2">
            <TabsTrigger value="templates">Templates</TabsTrigger>
            <TabsTrigger value="manual">Manual</TabsTrigger>
          </TabsList>

          {/* Templates Tab */}
          <TabsContent value="templates" className="mt-4">
            <McpServerTemplates
              onSelectTemplate={(template) => {
                handleSelectTemplate(template)
                setCreateTab("manual")
              }}
            />
          </TabsContent>

          {/* Manual Tab */}
          <TabsContent value="manual" className="mt-4">
            <form onSubmit={handleCreateServer} className="space-y-4">
              {/* Show selected template info */}
              {selectedTemplate && (
                <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="text-2xl">{selectedTemplate.icon}</span>
                      <div>
                        <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                          Using template: {selectedTemplate.name}
                        </p>
                        <p className="text-xs text-blue-700 dark:text-blue-300">
                          Customize the settings below
                        </p>
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      onClick={() => {
                        setSelectedTemplate(null)
                        setServerName("")
                        setCommand("")
                        setArgs("")
                        setUrl("")
                        setAuthMethod("none")
                      }}
                    >
                      Clear
                    </Button>
                  </div>
                  {selectedTemplate.setupInstructions && (
                    <p className="text-xs text-blue-700 dark:text-blue-300 mt-2">
                      {selectedTemplate.setupInstructions}
                    </p>
                  )}
                </div>
              )}

              <div>
                <label className="block text-sm font-medium mb-2">Server Name</label>
                <Input
                  value={serverName}
                  onChange={(e) => setServerName(e.target.value)}
                  placeholder="My MCP Server"
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium mb-2">Transport Type</label>
                <LegacySelect
                  value={transportType}
                  onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}
                >
                  <option value="Stdio">STDIO (Subprocess)</option>
                  <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                </LegacySelect>
              </div>

              {/* STDIO Config */}
              {transportType === "Stdio" && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-2">Command</label>
                    <Input
                      value={command}
                      onChange={(e) => setCommand(e.target.value)}
                      placeholder="npx -y @modelcontextprotocol/server-everything"
                      required
                    />
                    <p className="text-xs text-muted-foreground mt-1">
                      Example: npx -y &lt;command&gt;
                    </p>
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">
                      Arguments (one per line)
                    </label>
                    <textarea
                      value={args}
                      onChange={(e) => setArgs(e.target.value)}
                      placeholder={"-y\n@modelcontextprotocol/server-everything"}
                      className="w-full px-3 py-2 bg-background border border-input rounded-md text-sm min-h-[80px]"
                      rows={3}
                    />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">
                      Environment Variables
                    </label>
                    <KeyValueInput
                      value={envVars}
                      onChange={setEnvVars}
                      keyPlaceholder="KEY"
                      valuePlaceholder="VALUE"
                    />
                  </div>
                </>
              )}

              {/* HTTP-SSE Config */}
              {transportType === "Sse" && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-2">URL</label>
                    <Input
                      value={url}
                      onChange={(e) => setUrl(e.target.value)}
                      placeholder="https://mcp.example.com/sse"
                      required
                    />
                  </div>

                  <div>
                    <label className="block text-sm font-medium mb-2">Headers</label>
                    <KeyValueInput
                      value={headers}
                      onChange={setHeaders}
                      keyPlaceholder="Header Name"
                      valuePlaceholder="Header Value"
                    />
                  </div>
                </>
              )}

              {/* Authentication Configuration */}
              {transportType === "Sse" && (
                <div className="border-t pt-4 mt-4">
                  <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
                  <p className="text-sm text-muted-foreground mb-3">
                    Configure how LocalRouter authenticates to this MCP server
                  </p>

                  <div>
                    <label className="block text-sm font-medium mb-2">Authentication</label>
                    <LegacySelect
                      value={authMethod}
                      onChange={(e) => setAuthMethod(e.target.value as typeof authMethod)}
                    >
                      <option value="none">None / Via headers</option>
                      <option value="bearer">Bearer Token</option>
                      <option value="oauth_browser">OAuth (Browser)</option>
                    </LegacySelect>
                  </div>

                  {/* Bearer Token Auth */}
                  {authMethod === "bearer" && (
                    <div className="mt-3">
                      <label className="block text-sm font-medium mb-2">Bearer Token</label>
                      <Input
                        type="password"
                        value={bearerToken}
                        onChange={(e) => setBearerToken(e.target.value)}
                        placeholder="your-bearer-token"
                        required
                      />
                      <p className="text-xs text-muted-foreground mt-1">
                        Token will be stored securely in system keychain
                      </p>
                    </div>
                  )}

                  {/* OAuth Browser Flow */}
                  {authMethod === "oauth_browser" && (
                    <div className="mt-3">
                      <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded p-3">
                        <p className="text-blue-800 dark:text-blue-200 text-sm">
                          OAuth will be configured after creation. The app will discover OAuth
                          endpoints from the MCP server and guide you through authentication.
                        </p>
                      </div>
                    </div>
                  )}
                </div>
              )}

              <div className="flex justify-end gap-2 pt-4">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setShowCreateModal(false)
                    resetForm()
                  }}
                  disabled={isCreating}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={isCreating}>
                  {isCreating ? "Creating..." : "Create Server"}
                </Button>
              </div>
            </form>
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>

    {/* OAuth Modal */}
    {selectedServer && selectedServer.auth_config?.type === "oauth_browser" && isOAuthConfigured(selectedServer) && (
      <McpOAuthModal
        isOpen={showOAuthModal}
        onClose={() => setShowOAuthModal(false)}
        serverId={selectedServer.id}
        serverName={selectedServer.name}
        onSuccess={handleOAuthSuccess}
      />
    )}

    {/* OAuth Setup Dialog */}
    <Dialog open={showOAuthSetup} onOpenChange={setShowOAuthSetup}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Setup OAuth</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          {oauthDiscovery && (
            <div className="bg-muted rounded p-3 text-sm">
              <p className="font-medium mb-2">Discovered OAuth Endpoints:</p>
              <p className="text-xs text-muted-foreground truncate">
                Auth: {oauthDiscovery.auth_url}
              </p>
              <p className="text-xs text-muted-foreground truncate">
                Token: {oauthDiscovery.token_url}
              </p>
              {oauthDiscovery.scopes.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  Scopes: {oauthDiscovery.scopes.join(", ")}
                </p>
              )}
            </div>
          )}

          <div>
            <label className="block text-sm font-medium mb-2">Client ID</label>
            <Input
              value={oauthSetupClientId}
              onChange={(e) => setOauthSetupClientId(e.target.value)}
              placeholder="your-oauth-app-client-id"
            />
            <p className="text-xs text-muted-foreground mt-1">
              Create an OAuth app in your provider's settings
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">Client Secret</label>
            <Input
              type="password"
              value={oauthSetupClientSecret}
              onChange={(e) => setOauthSetupClientSecret(e.target.value)}
              placeholder="your-oauth-app-client-secret"
            />
          </div>

          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              onClick={() => setShowOAuthSetup(false)}
              disabled={isSavingOAuth}
            >
              Cancel
            </Button>
            <Button
              onClick={handleSaveOAuthCredentials}
              disabled={!oauthSetupClientId || !oauthSetupClientSecret || isSavingOAuth}
            >
              {isSavingOAuth ? "Saving..." : "Save & Continue"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>

    {/* Delete Confirmation Dialog */}
    <AlertDialog open={!!serverToDelete} onOpenChange={(open) => !open && setServerToDelete(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete MCP Server</AlertDialogTitle>
          <AlertDialogDescription>
            Are you sure you want to delete "{serverToDelete?.name}"? This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            onClick={handleDelete}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    </>
  )
}
