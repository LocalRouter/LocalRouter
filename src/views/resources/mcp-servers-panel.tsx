import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Plus, CheckCircle, XCircle, Loader2, RefreshCw, FlaskConical, Blocks, Grid, Store, ArrowLeft, Settings2 } from "lucide-react"
import McpServerIcon from "@/components/McpServerIcon"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
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
import LegacySelect from "@/components/ui/Select"
import KeyValueInput from "@/components/ui/KeyValueInput"
import { McpServerTemplates, McpServerTemplate } from "@/components/mcp/McpServerTemplates"
import { McpOAuthModal } from "@/components/mcp/McpOAuthModal"
import { MarketplaceSearchPanel, McpServerListing } from "@/components/add-resource"
import ServiceIcon from "@/components/ServiceIcon"
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

export interface McpHealthStatus {
  status: "pending" | "healthy" | "ready" | "unhealthy" | "unknown" | "disabled"
  latency_ms?: number
  error?: string
}

export interface McpHealthCheckEvent {
  server_id: string
  server_name: string
  status: string
  latency_ms?: number
  error?: string
}

interface McpServersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  healthStatus: Record<string, McpHealthStatus>
  onHealthInit: (serverIds: string[]) => void
  onRefreshHealth: (serverId: string) => Promise<void>
  initialAddTemplateId?: string | null
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function McpServersPanel({
  selectedId,
  onSelect,
  healthStatus,
  onHealthInit,
  onRefreshHealth,
  initialAddTemplateId,
  onViewChange,
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

  // Detail tab state
  const [detailTab, setDetailTab] = useState("info")

  // Inline edit state
  const [isEditing, setIsEditing] = useState(false)

  // Create modal state
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [dialogPage, setDialogPage] = useState<"select" | "configure">("select")
  const [dialogTab, setDialogTab] = useState<"templates" | "marketplace" | "custom">("templates")
  const [selectedSource, setSelectedSource] = useState<{
    type: "template" | "marketplace"
    template?: McpServerTemplate
    listing?: McpServerListing
  } | null>(null)
  const [isCreating, setIsCreating] = useState(false)

  // Form state
  const [serverName, setServerName] = useState("")
  const [transportType, setTransportType] = useState<"Stdio" | "Sse">("Stdio")
  const [command, setCommand] = useState("")
  const [envVars, setEnvVars] = useState<Record<string, string>>({})
  const [url, setUrl] = useState("")
  const [headers, setHeaders] = useState<Record<string, string>>({})

  // Auth config state
  const [authMethod, setAuthMethod] = useState<"none" | "bearer" | "oauth_pregenerated" | "oauth_browser">("none")
  const [bearerToken, setBearerToken] = useState("")

  // OAuth credentials state (for pregenerated flow)
  const [oauthClientId, setOauthClientId] = useState("")
  const [oauthClientSecret, setOauthClientSecret] = useState("")

  useEffect(() => {
    loadServers()

    const unsubscribe = listen("mcp-servers-changed", () => {
      loadServersOnly()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  // Handle initial add template from navigation
  useEffect(() => {
    if (initialAddTemplateId) {
      const { MCP_SERVER_TEMPLATES } = require("@/components/mcp/McpServerTemplates")
      const template = MCP_SERVER_TEMPLATES.find((t: McpServerTemplate) => t.id === initialAddTemplateId)
      if (template) {
        setShowCreateModal(true)
        setSelectedSource({ type: "template", template })
        setDialogPage("configure")
        setServerName(template.name)
        setTransportType(template.transport)
        if (template.transport === "Stdio") {
          setCommand([template.command, ...(template.args || [])].join(" "))
          setUrl("")
        } else {
          setUrl(template.url || "")
          setCommand("")
        }
        setAuthMethod(template.authMethod)
      }
    }
  }, [initialAddTemplateId])

  // Reset detail tab when selection changes
  useEffect(() => {
    setDetailTab("info")
  }, [selectedId])

  // Load servers and initialize health checks (only on first load)
  const loadServers = async () => {
    try {
      setLoading(true)
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)

      // Initialize health checks (parent will only do this once)
      onHealthInit(serverList.map(s => s.id))
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
    } finally {
      setLoading(false)
    }
  }

  // Load servers without triggering health checks (for refreshes/updates)
  const loadServersOnly = async () => {
    try {
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
    }
  }

  const resetForm = () => {
    setServerName("")
    setTransportType("Stdio")
    setCommand("")
    setEnvVars({})
    setUrl("")
    setHeaders({})
    setAuthMethod("none")
    setBearerToken("")
    setOauthClientId("")
    setOauthClientSecret("")
    setSelectedSource(null)
    setDialogPage("select")
    setDialogTab("templates")
  }

  const handleSelectTemplate = (template: McpServerTemplate) => {
    setSelectedSource({ type: "template", template })
    setServerName(template.name)
    setTransportType(template.transport)

    if (template.transport === "Stdio" && template.command) {
      const fullCommand = template.args
        ? [template.command, ...template.args].join(" ")
        : template.command
      setCommand(fullCommand)
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

    // Switch to configure page
    setDialogPage("configure")
  }

  const handleSelectMarketplaceMcp = (listing: McpServerListing) => {
    setSelectedSource({ type: "marketplace", listing })
    setServerName(listing.name)

    // Pre-populate form based on listing
    if (listing.available_transports.includes("stdio")) {
      setTransportType("Stdio")
      if (listing.packages.length > 0) {
        const pkg = listing.packages[0]
        if (pkg.runtime === "node" || pkg.registry === "npm") {
          setCommand(`npx -y ${pkg.name}`)
        } else if (pkg.runtime === "python" || pkg.registry === "pypi") {
          setCommand(`uvx ${pkg.name}`)
        }
      }
      setUrl("")
    } else if (listing.remotes.length > 0) {
      setTransportType("Sse")
      setUrl(listing.remotes[0].url)
      setCommand("")
    }

    setAuthMethod("none")
    setDialogPage("configure")
  }

  const handleBackToSelect = () => {
    setDialogPage("select")
    // Keep form data in case user wants to go back
  }

  const handleCreateServer = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsCreating(true)

    try {
      let transportConfig
      if (transportType === "Stdio") {
        transportConfig = { type: "stdio", command, env: envVars }
      } else {
        transportConfig = { type: "http_sse", url, headers: headers }
      }

      let authConfig = null
      if (authMethod === "bearer") {
        authConfig = { type: "bearer_token", token: bearerToken }
      } else if (authMethod === "oauth_pregenerated") {
        if (!oauthClientId || !oauthClientSecret) {
          toast.error("Client ID and Client Secret are required for OAuth")
          setIsCreating(false)
          return
        }
        const discovery = await invoke<{
          auth_url: string; token_url: string; scopes_supported: string[]
        } | null>("discover_mcp_oauth_endpoints", { baseUrl: url })
        if (!discovery) {
          toast.error("This MCP server does not support OAuth")
          setIsCreating(false)
          return
        }
        authConfig = {
          type: "oauth", client_id: oauthClientId, client_secret: oauthClientSecret,
          auth_url: discovery.auth_url, token_url: discovery.token_url, scopes: discovery.scopes_supported,
        }
      } else if (authMethod === "oauth_browser") {
        authConfig = { type: "oauth_browser" }
      }

      const newServer = await invoke<{ id: string }>("create_mcp_server", {
        name: serverName || null, transport: transportType, transportConfig, authConfig,
      })

      toast.success("MCP server created")
      await loadServersOnly()
      setShowCreateModal(false)
      resetForm()
      onRefreshHealth(newServer.id)
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
        serverId: server.id, enabled: !server.enabled,
      })
      toast.success(`Server ${server.enabled ? "disabled" : "enabled"}`)
      loadServersOnly()
      onRefreshHealth(server.id)
    } catch (error) {
      toast.error("Failed to update server")
    }
  }

  const populateFormFromServer = (server: McpServer) => {
    setServerName(server.name)
    setTransportType(server.transport === "Stdio" ? "Stdio" : "Sse")
    setSelectedSource(null)

    const tc = server.transport_config as Record<string, unknown>
    if (server.transport === "Stdio") {
      const cmd = (tc.command as string) || ""
      const args = (tc.args as string[]) || []
      const fullCommand = args.length > 0 ? [cmd, ...args].join(" ") : cmd
      setCommand(fullCommand)
      setEnvVars((tc.env as Record<string, string>) || {})
      setUrl("")
      setHeaders({})
    } else {
      setUrl((tc.url as string) || "")
      setHeaders((tc.headers as Record<string, string>) || {})
      setCommand("")
      setEnvVars({})
    }

    if (!server.auth_config || server.auth_config.type === "none") {
      setAuthMethod("none"); setBearerToken(""); setOauthClientId(""); setOauthClientSecret("")
    } else if (server.auth_config.type === "bearer_token") {
      setAuthMethod("bearer"); setBearerToken(""); setOauthClientId(""); setOauthClientSecret("")
    } else if (server.auth_config.type === "oauth") {
      setAuthMethod("oauth_pregenerated"); setBearerToken("")
      setOauthClientId((server.auth_config as { client_id?: string }).client_id || "")
      setOauthClientSecret("")
    } else if (server.auth_config.type === "oauth_browser") {
      setAuthMethod("oauth_browser"); setBearerToken("")
      setOauthClientId((server.auth_config as { client_id?: string }).client_id || "")
      setOauthClientSecret("")
    } else {
      setAuthMethod("none"); setBearerToken(""); setOauthClientId(""); setOauthClientSecret("")
    }
  }

  const handleEditServer = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!selectedServer) return

    setIsEditing(true)
    try {
      let transportConfig
      if (transportType === "Stdio") {
        transportConfig = { type: "stdio", command, env: envVars }
      } else {
        transportConfig = { type: "http_sse", url, headers: headers }
      }

      let authConfig = null
      if (authMethod === "bearer" && bearerToken) {
        authConfig = { type: "bearer_token", token: bearerToken }
      } else if (authMethod === "oauth_browser") {
        authConfig = { type: "oauth_browser" }
      } else if (authMethod === "none") {
        authConfig = null
      }

      const updates: Record<string, unknown> = {
        name: serverName, transport_config: transportConfig,
      }

      if (authMethod === "bearer" && bearerToken) {
        updates.auth_config = authConfig
      } else if (authMethod === "none" && selectedServer.auth_config?.type !== "none" && selectedServer.auth_config !== null) {
        updates.auth_config = null
      }

      await invoke("update_mcp_server", { serverId: selectedServer.id, updates })
      toast.success("MCP server updated")
      await loadServersOnly()
    } catch (error) {
      console.error("Failed to update MCP server:", error)
      toast.error(`Error updating MCP server: ${error}`)
    } finally {
      setIsEditing(false)
    }
  }

  const handleDelete = async () => {
    if (!serverToDelete) return
    try {
      await invoke("delete_mcp_server", { serverId: serverToDelete.id })
      toast.success("Server deleted")
      if (selectedId === serverToDelete.id) { onSelect(null) }
      loadServersOnly()
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
    if (selectedId) { checkOAuthStatus(selectedId) }
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

  const isOAuthConfigured = (server: McpServer) => {
    if (server.auth_config?.type !== "oauth_browser") return false
    return !!(server.auth_config as { client_id?: string }).client_id
  }

  const handleStartOAuthSetup = async () => {
    if (!selectedServer) return
    setIsDiscovering(true)
    setOauthDiscovery(null)
    setOauthSetupClientId("")
    setOauthSetupClientSecret("")

    try {
      const transportConfig = selectedServer.transport_config as { url?: string }
      if (!transportConfig.url) { toast.error("Server URL not found"); return }
      const baseUrl = transportConfig.url.replace(/\/+$/, "")
      const discovery = await invoke<{ auth_url: string; token_url: string; scopes: string[] } | null>(
        "discover_mcp_oauth_endpoints", { baseUrl }
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

  const handleSaveOAuthCredentials = async () => {
    if (!selectedServer || !oauthDiscovery) return
    setIsSavingOAuth(true)
    try {
      await invoke("update_mcp_server", {
        serverId: selectedServer.id,
        updates: {
          auth_config: {
            type: "oauth_browser", client_id: oauthSetupClientId, client_secret: oauthSetupClientSecret,
            auth_url: oauthDiscovery.auth_url, token_url: oauthDiscovery.token_url,
            scopes: oauthDiscovery.scopes, redirect_uri: "http://localhost:8080/callback",
          },
        },
      })
      toast.success("OAuth credentials saved")
      setShowOAuthSetup(false)
      await loadServersOnly()
    } catch (error) {
      toast.error(`Failed to save OAuth credentials: ${error}`)
    } finally {
      setIsSavingOAuth(false)
    }
  }

  useEffect(() => {
    if (selectedId) {
      const server = servers.find((s) => s.id === selectedId)
      if (server?.auth_config?.type === "oauth_browser") {
        checkOAuthStatus(selectedId)
      }
    }
  }, [selectedId, servers])

  // Populate edit form when switching to settings tab
  useEffect(() => {
    if (detailTab === "settings" && selectedServer) {
      populateFormFromServer(selectedServer)
    }
  }, [detailTab, selectedId])

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
                placeholder="Search MCP..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="flex-1"
              />
              <Button size="icon" onClick={() => setShowCreateModal(true)} title="Add MCP">
                <Plus className="h-4 w-4" />
              </Button>
            </div>
          </div>
          <ScrollArea className="flex-1">
            <div className="p-2 space-y-1">
              {loading ? (
                <p className="text-sm text-muted-foreground p-4">Loading...</p>
              ) : filteredServers.length === 0 ? (
                <p className="text-sm text-muted-foreground p-4">No MCP found</p>
              ) : (
                filteredServers.map((server) => {
                  const health = healthStatus[server.id]
                  const formatLatency = (ms?: number) => {
                    if (ms == null) return ""
                    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
                  }
                  return (
                    <div
                      key={server.id}
                      onClick={() => onSelect(server.id)}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedId === server.id ? "bg-accent" : "hover:bg-muted"
                      )}
                    >
                      <McpServerIcon serverName={server.name} size={20} />
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate">{server.name}</p>
                        <p className="text-xs text-muted-foreground capitalize">
                          {server.transport === "Stdio" ? "STDIO" : "HTTP SSE"}
                        </p>
                      </div>
                      <div className="flex items-center gap-2">
                        {health && health.latency_ms != null && health.status !== "pending" && (
                          <span className="text-xs text-muted-foreground">
                            {formatLatency(health.latency_ms)}
                          </span>
                        )}
                        {(!health || health.status === "pending") ? (
                          <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
                        ) : (
                          <div
                            className={cn(
                              "h-2 w-2 rounded-full",
                              (health.status === "healthy" || health.status === "ready") && "bg-green-500",
                              (health.status === "unhealthy" || health.status === "unknown") && "bg-red-500",
                              health.status === "disabled" && "bg-gray-400"
                            )}
                            title={
                              health.status === "healthy"
                                ? health.latency_ms != null
                                  ? `Running (${formatLatency(health.latency_ms)})`
                                  : "Running"
                                : health.status === "ready"
                                ? "Ready to start"
                                : health.status === "disabled"
                                ? "Disabled"
                                : health.error || "Unhealthy"
                            }
                          />
                        )}
                      </div>
                    </div>
                  )
                })
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
                  {onViewChange && selectedServer.enabled && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => onViewChange("try-it-out", `mcp/init/direct/server:${selectedServer.id}`)}
                    >
                      <FlaskConical className="h-4 w-4 mr-1" />
                      Try It Out
                    </Button>
                  )}
                </div>
              </div>

              <Tabs value={detailTab} onValueChange={setDetailTab}>
                <TabsList>
                  <TabsTrigger value="info">Info</TabsTrigger>
                  <TabsTrigger value="settings">Settings</TabsTrigger>
                </TabsList>

                <TabsContent value="info">
                  <div className="space-y-6">
                    {/* Health Status */}
                    <Card>
                      <CardHeader className="pb-3">
                        <div className="flex items-center justify-between">
                          <CardTitle className="text-sm">Health Status</CardTitle>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6"
                            onClick={() => onRefreshHealth(selectedServer.id)}
                            disabled={healthStatus[selectedServer.id]?.status === "pending"}
                          >
                            <RefreshCw className={cn(
                              "h-3 w-3",
                              healthStatus[selectedServer.id]?.status === "pending" && "animate-spin"
                            )} />
                          </Button>
                        </div>
                      </CardHeader>
                      <CardContent>
                        {(() => {
                          const health = healthStatus[selectedServer.id]
                          const formatLatency = (ms?: number) => {
                            if (!ms) return ""
                            return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
                          }

                          if (!health || health.status === "pending") {
                            return (
                              <div className="flex items-center gap-2 text-muted-foreground">
                                <Loader2 className="h-4 w-4 animate-spin" />
                                <span>Checking health...</span>
                              </div>
                            )
                          }
                          if (health.status === "healthy") {
                            return (
                              <div className="flex items-center gap-2 text-green-600">
                                <CheckCircle className="h-4 w-4" />
                                <span>Running</span>
                                {health.latency_ms != null && (
                                  <span className="text-muted-foreground">({formatLatency(health.latency_ms)})</span>
                                )}
                              </div>
                            )
                          }
                          if (health.status === "ready") {
                            return (
                              <div className="flex items-center gap-2 text-green-600">
                                <CheckCircle className="h-4 w-4" />
                                <span>Ready</span>
                                {health.error && <span className="text-muted-foreground">- {health.error}</span>}
                              </div>
                            )
                          }
                          if (health.status === "disabled") {
                            return (
                              <div className="flex items-center gap-2 text-muted-foreground">
                                <XCircle className="h-4 w-4" />
                                <span>Disabled</span>
                              </div>
                            )
                          }
                          return (
                            <div className="flex items-center gap-2 text-red-600">
                              <XCircle className="h-4 w-4" />
                              <span>Unhealthy</span>
                              {health.error && <span className="text-muted-foreground">- {health.error}</span>}
                            </div>
                          )
                        })()}
                      </CardContent>
                    </Card>

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

                    {/* Transport Configuration */}
                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">Transport Configuration</CardTitle>
                      </CardHeader>
                      <CardContent className="space-y-4">
                        {selectedServer.transport === "Stdio" && (() => {
                          const tc = selectedServer.transport_config as { command?: string; args?: string[]; env?: Record<string, string> }
                          return (
                            <>
                              <div>
                                <p className="text-sm text-muted-foreground">Command</p>
                                <code className="text-sm break-all">{tc.command || "N/A"}</code>
                              </div>
                              {tc.args && tc.args.length > 0 && (
                                <div>
                                  <p className="text-sm text-muted-foreground">Arguments</p>
                                  <code className="text-sm break-all">{tc.args.join(" ")}</code>
                                </div>
                              )}
                              {tc.env && Object.keys(tc.env).length > 0 && (
                                <div>
                                  <p className="text-sm text-muted-foreground">Environment Variables</p>
                                  <div className="space-y-1">
                                    {Object.entries(tc.env).map(([key, value]) => (
                                      <div key={key} className="text-sm">
                                        <code>{key}</code>=<code className="text-muted-foreground">{value}</code>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              )}
                            </>
                          )
                        })()}
                        {selectedServer.transport !== "Stdio" && (() => {
                          const tc = selectedServer.transport_config as { url?: string; headers?: Record<string, string> }
                          return (
                            <>
                              <div>
                                <p className="text-sm text-muted-foreground">URL</p>
                                <code className="text-sm break-all">{tc.url || "N/A"}</code>
                              </div>
                              {tc.headers && Object.keys(tc.headers).length > 0 && (
                                <div>
                                  <p className="text-sm text-muted-foreground">Headers</p>
                                  <div className="space-y-1">
                                    {Object.entries(tc.headers).map(([key, value]) => (
                                      <div key={key} className="text-sm">
                                        <code>{key}</code>: <code className="text-muted-foreground">{value}</code>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              )}
                            </>
                          )
                        })()}
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
                            <>
                              <p className="text-sm text-muted-foreground">
                                OAuth credentials are not configured. Click Setup to discover OAuth
                                endpoints and enter your credentials.
                              </p>
                              <Button size="sm" onClick={handleStartOAuthSetup} disabled={isDiscovering}>
                                {isDiscovering ? "Discovering..." : "Setup OAuth"}
                              </Button>
                            </>
                          ) : (
                            <>
                              <div className="flex items-center justify-between">
                                <div>
                                  <p className="text-sm font-medium">
                                    {oauthStatus[selectedServer.id] ? "Authenticated" : "Not authenticated"}
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
                                    <Button size="sm" variant="secondary" onClick={() => checkOAuthStatus(selectedServer.id)}>
                                      Test
                                    </Button>
                                    <Button size="sm" variant="destructive" onClick={() => handleRevokeOAuth(selectedServer.id)}>
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
                  </div>
                </TabsContent>

                <TabsContent value="settings">
                  <div className="space-y-6">
                    {/* Inline Edit Form */}
                    <Card>
                      <CardHeader>
                        <CardTitle>Server Configuration</CardTitle>
                        <CardDescription>Update the configuration for this MCP server</CardDescription>
                      </CardHeader>
                      <CardContent>
                        <form onSubmit={handleEditServer} className="space-y-4">
                          <div>
                            <label className="block text-sm font-medium mb-2">Server Name</label>
                            <Input value={serverName} onChange={(e) => setServerName(e.target.value)} placeholder="My MCP Server" required />
                          </div>
                          <div>
                            <label className="block text-sm font-medium mb-2">Transport Type</label>
                            <LegacySelect value={transportType} onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}>
                              <option value="Stdio">STDIO (Subprocess)</option>
                              <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                            </LegacySelect>
                          </div>
                          {transportType === "Stdio" && (
                            <>
                              <div>
                                <label className="block text-sm font-medium mb-2">Command</label>
                                <Input value={command} onChange={(e) => setCommand(e.target.value)} placeholder="npx -y @modelcontextprotocol/server-everything" required />
                                <p className="text-xs text-muted-foreground mt-1">Full command with arguments</p>
                              </div>
                              <div>
                                <label className="block text-sm font-medium mb-2">Environment Variables</label>
                                <KeyValueInput value={envVars} onChange={setEnvVars} keyPlaceholder="KEY" valuePlaceholder="VALUE" />
                              </div>
                            </>
                          )}
                          {transportType === "Sse" && (
                            <div>
                              <label className="block text-sm font-medium mb-2">URL</label>
                              <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://api.example.com/mcp" required />
                            </div>
                          )}
                          {transportType === "Sse" && (
                            <div className="border-t pt-4 mt-4">
                              <h3 className="text-md font-semibold mb-3">Authentication</h3>
                              <p className="text-sm text-muted-foreground mb-3">Configure how LocalRouter authenticates to this MCP server</p>
                              <div>
                                <label className="block text-sm font-medium mb-2">Authentication Method</label>
                                <LegacySelect value={authMethod} onChange={(e) => setAuthMethod(e.target.value as typeof authMethod)}>
                                  <option value="none">None / Via headers</option>
                                  <option value="bearer">Bearer Token</option>
                                  <option value="oauth_pregenerated">OAuth (Pre-generated credentials)</option>
                                </LegacySelect>
                              </div>
                              {authMethod === "bearer" && (
                                <div className="mt-3">
                                  <label className="block text-sm font-medium mb-2">Bearer Token</label>
                                  <Input type="password" value={bearerToken} onChange={(e) => setBearerToken(e.target.value)} placeholder="Enter new token to update (leave empty to keep existing)" />
                                  <p className="text-xs text-muted-foreground mt-1">Leave empty to keep the existing token. Token will be stored securely in system keychain.</p>
                                </div>
                              )}
                              {authMethod === "oauth_pregenerated" && (
                                <div className="mt-3 space-y-3">
                                  <div>
                                    <label className="block text-sm font-medium mb-2">Client ID</label>
                                    <Input value={oauthClientId} onChange={(e) => setOauthClientId(e.target.value)} placeholder="your-oauth-client-id" />
                                    <p className="text-xs text-muted-foreground mt-1">Leave empty to keep the existing client ID</p>
                                  </div>
                                  <div>
                                    <label className="block text-sm font-medium mb-2">Client Secret</label>
                                    <Input type="password" value={oauthClientSecret} onChange={(e) => setOauthClientSecret(e.target.value)} placeholder="Enter new secret to update (leave empty to keep existing)" />
                                    <p className="text-xs text-muted-foreground mt-1">Leave empty to keep the existing secret. Stored securely in system keychain.</p>
                                  </div>
                                </div>
                              )}
                            </div>
                          )}
                          {transportType === "Sse" && (
                            <div>
                              <label className="block text-sm font-medium mb-2">Headers (Optional)</label>
                              <KeyValueInput value={headers} onChange={setHeaders} keyPlaceholder="Header Name" valuePlaceholder="Header Value" />
                            </div>
                          )}
                          <div className="flex justify-end gap-2 pt-4">
                            <Button type="submit" disabled={isEditing}>
                              {isEditing ? "Saving..." : "Save Changes"}
                            </Button>
                          </div>
                        </form>
                      </CardContent>
                    </Card>

                    {/* Enable/Disable */}
                    <Card>
                      <CardHeader>
                        <CardTitle>Enable Server</CardTitle>
                        <CardDescription>When disabled, this MCP server will not be available to clients</CardDescription>
                      </CardHeader>
                      <CardContent>
                        <div className="flex items-center gap-3">
                          <Switch checked={selectedServer.enabled} onCheckedChange={() => handleToggle(selectedServer)} />
                          <span className="text-sm">{selectedServer.enabled ? "Enabled" : "Disabled"}</span>
                        </div>
                      </CardContent>
                    </Card>

                    {/* Danger Zone */}
                    <Card className="border-red-200 dark:border-red-900">
                      <CardHeader>
                        <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                        <CardDescription>Irreversible actions for this server</CardDescription>
                      </CardHeader>
                      <CardContent>
                        <div className="flex items-center justify-between">
                          <div>
                            <p className="text-sm font-medium">Delete this server</p>
                            <p className="text-sm text-muted-foreground">Permanently delete "{selectedServer.name}" and its configuration</p>
                          </div>
                          <Button variant="destructive" onClick={() => setServerToDelete(selectedServer)}>
                            Delete Server
                          </Button>
                        </div>
                      </CardContent>
                    </Card>
                  </div>
                </TabsContent>
              </Tabs>

            </div>
          </ScrollArea>
        ) : (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
            <Blocks className="h-12 w-12 opacity-30" />
            <div className="text-center">
              <p className="font-medium">Select an MCP to view details</p>
            </div>
          </div>
        )}
      </ResizablePanel>
    </ResizablePanelGroup>

    {/* Create MCP Modal */}
    <Dialog
      open={showCreateModal}
      onOpenChange={(open) => {
        if (!open) { setShowCreateModal(false); resetForm() }
      }}
    >
      <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col overflow-hidden">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Add MCP</DialogTitle>
        </DialogHeader>

        {dialogPage === "select" ? (
          /* Page 1: Selection */
          <Tabs value={dialogTab} onValueChange={(v) => setDialogTab(v as typeof dialogTab)} className="flex-1 flex flex-col min-h-0">
            <TabsList className="grid w-full grid-cols-3 flex-shrink-0">
              <TabsTrigger value="templates" className="gap-2">
                <Grid className="h-4 w-4" />
                Templates
              </TabsTrigger>
              <TabsTrigger value="marketplace" className="gap-2">
                <Store className="h-4 w-4" />
                Marketplace
              </TabsTrigger>
              <TabsTrigger value="custom" className="gap-2">
                <Settings2 className="h-4 w-4" />
                Custom
              </TabsTrigger>
            </TabsList>

            {/* Templates Tab */}
            <TabsContent value="templates" className="mt-4 flex-1 overflow-y-auto">
              <McpServerTemplates onSelectTemplate={handleSelectTemplate} />
            </TabsContent>

            {/* Marketplace Tab */}
            <TabsContent value="marketplace" className="mt-4 flex-1 min-h-0 overflow-hidden">
              <MarketplaceSearchPanel
                type="mcp"
                onSelectMcp={handleSelectMarketplaceMcp}
                maxHeight="calc(85vh - 180px)"
              />
            </TabsContent>

            {/* Custom Tab */}
            <TabsContent value="custom" className="mt-4 flex-1 overflow-y-auto">
              <form onSubmit={handleCreateServer} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium mb-2">Server Name</label>
                  <Input value={serverName} onChange={(e) => setServerName(e.target.value)} placeholder="My MCP Server" required />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">Transport Type</label>
                  <LegacySelect value={transportType} onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}>
                    <option value="Stdio">STDIO (Subprocess)</option>
                    <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                  </LegacySelect>
                </div>
                {transportType === "Stdio" && (
                  <>
                    <div>
                      <label className="block text-sm font-medium mb-2">Command</label>
                      <Input value={command} onChange={(e) => setCommand(e.target.value)} placeholder="npx -y @modelcontextprotocol/server-everything" required />
                      <p className="text-xs text-muted-foreground mt-1">Full command with arguments</p>
                    </div>
                    <div>
                      <label className="block text-sm font-medium mb-2">Environment Variables</label>
                      <KeyValueInput value={envVars} onChange={setEnvVars} keyPlaceholder="KEY" valuePlaceholder="VALUE" />
                    </div>
                  </>
                )}
                {transportType === "Sse" && (
                  <>
                    <div>
                      <label className="block text-sm font-medium mb-2">URL</label>
                      <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://api.example.com/mcp" required />
                    </div>
                    <div className="border-t pt-4 mt-4">
                      <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
                      <div>
                        <label className="block text-sm font-medium mb-2">Authentication Method</label>
                        <LegacySelect value={authMethod} onChange={(e) => setAuthMethod(e.target.value as typeof authMethod)}>
                          <option value="none">None / Via headers</option>
                          <option value="bearer">Bearer Token</option>
                          <option value="oauth_pregenerated">OAuth (Pre-generated credentials)</option>
                        </LegacySelect>
                      </div>
                      {authMethod === "bearer" && (
                        <div className="mt-3">
                          <label className="block text-sm font-medium mb-2">Bearer Token</label>
                          <Input type="password" value={bearerToken} onChange={(e) => setBearerToken(e.target.value)} placeholder="your-bearer-token" required />
                        </div>
                      )}
                      {authMethod === "oauth_pregenerated" && (
                        <div className="mt-3 space-y-3">
                          <div>
                            <label className="block text-sm font-medium mb-2">Client ID</label>
                            <Input value={oauthClientId} onChange={(e) => setOauthClientId(e.target.value)} placeholder="your-oauth-client-id" required />
                          </div>
                          <div>
                            <label className="block text-sm font-medium mb-2">Client Secret</label>
                            <Input type="password" value={oauthClientSecret} onChange={(e) => setOauthClientSecret(e.target.value)} placeholder="your-oauth-client-secret" required />
                          </div>
                        </div>
                      )}
                    </div>
                    <div>
                      <label className="block text-sm font-medium mb-2">Headers (Optional)</label>
                      <KeyValueInput value={headers} onChange={setHeaders} keyPlaceholder="Header Name" valuePlaceholder="Header Value" />
                    </div>
                  </>
                )}
                <div className="flex justify-end gap-2 pt-4">
                  <Button type="button" variant="secondary" onClick={() => { setShowCreateModal(false); resetForm() }} disabled={isCreating}>Cancel</Button>
                  <Button type="submit" disabled={isCreating}>{isCreating ? "Creating..." : "Create"}</Button>
                </div>
              </form>
            </TabsContent>
          </Tabs>
        ) : (
          /* Page 2: Configuration Form */
          <div className="flex-1 flex flex-col min-h-0 overflow-y-auto space-y-4">
            {/* Back button and source header */}
            <div className="flex items-center gap-3 pb-2 border-b flex-shrink-0">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={handleBackToSelect}
                className="h-8 px-2"
              >
                <ArrowLeft className="h-4 w-4 mr-1" />
                Back
              </Button>
              {selectedSource && (
                <div className="flex items-center gap-2">
                  {selectedSource.type === "template" && selectedSource.template && (
                    <>
                      <ServiceIcon service={selectedSource.template.id} size={24} fallbackToServerIcon />
                      <div>
                        <p className="text-sm font-medium">{selectedSource.template.name}</p>
                        <p className="text-xs text-muted-foreground">{selectedSource.template.description}</p>
                      </div>
                    </>
                  )}
                  {selectedSource.type === "marketplace" && selectedSource.listing && (
                    <>
                      <ServiceIcon service={selectedSource.listing.name.toLowerCase().replace(/[^a-z0-9]/g, "")} size={24} fallbackToServerIcon />
                      <div>
                        <p className="text-sm font-medium">{selectedSource.listing.name}</p>
                        <p className="text-xs text-muted-foreground">{selectedSource.listing.description}</p>
                      </div>
                    </>
                  )}
                </div>
              )}
            </div>

            {/* Setup instructions */}
            {selectedSource?.type === "template" && selectedSource.template?.setupInstructions && (
              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                <p className="text-xs text-blue-700 dark:text-blue-300">
                  {selectedSource.template.setupInstructions}
                </p>
              </div>
            )}
            {selectedSource?.type === "marketplace" && selectedSource.listing?.install_hint && (
              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                <p className="text-xs text-blue-700 dark:text-blue-300">
                  {selectedSource.listing.install_hint}
                </p>
              </div>
            )}

            <form onSubmit={handleCreateServer} className="space-y-4">
              <div>
                <label className="block text-sm font-medium mb-2">Server Name</label>
                <Input value={serverName} onChange={(e) => setServerName(e.target.value)} placeholder="My MCP Server" required />
              </div>
              <div>
                <label className="block text-sm font-medium mb-2">Transport Type</label>
                <LegacySelect value={transportType} onChange={(e) => setTransportType(e.target.value as "Stdio" | "Sse")}>
                  <option value="Stdio">STDIO (Subprocess)</option>
                  <option value="Sse">HTTP-SSE (Server-Sent Events)</option>
                </LegacySelect>
              </div>
              {transportType === "Stdio" && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-2">Command</label>
                    <Input value={command} onChange={(e) => setCommand(e.target.value)} placeholder="npx -y @modelcontextprotocol/server-everything" required />
                    <p className="text-xs text-muted-foreground mt-1">Full command with arguments (e.g., npx -y @modelcontextprotocol/server-filesystem /tmp)</p>
                  </div>
                  <div>
                    <label className="block text-sm font-medium mb-2">Environment Variables</label>
                    <KeyValueInput value={envVars} onChange={setEnvVars} keyPlaceholder="KEY" valuePlaceholder="VALUE" />
                  </div>
                </>
              )}
              {transportType === "Sse" && (
                <div>
                  <label className="block text-sm font-medium mb-2">URL</label>
                  <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://api.example.com/mcp" required />
                </div>
              )}
              {transportType === "Sse" && (
                <div className="border-t pt-4 mt-4">
                  <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
                  <p className="text-sm text-muted-foreground mb-3">Configure how LocalRouter authenticates to this MCP server</p>
                  <div>
                    <label className="block text-sm font-medium mb-2">Authentication Method</label>
                    <LegacySelect value={authMethod} onChange={(e) => setAuthMethod(e.target.value as typeof authMethod)}>
                      <option value="none">None / Via headers</option>
                      <option value="bearer">Bearer Token</option>
                      <option value="oauth_pregenerated">OAuth (Pre-generated credentials)</option>
                    </LegacySelect>
                  </div>
                  {authMethod === "bearer" && (
                    <div className="mt-3">
                      <label className="block text-sm font-medium mb-2">Bearer Token</label>
                      <Input type="password" value={bearerToken} onChange={(e) => setBearerToken(e.target.value)} placeholder="your-bearer-token" required />
                      <p className="text-xs text-muted-foreground mt-1">Token will be stored securely in system keychain</p>
                    </div>
                  )}
                  {authMethod === "oauth_pregenerated" && (
                    <div className="mt-3 space-y-3">
                      <div>
                        <label className="block text-sm font-medium mb-2">Client ID</label>
                        <Input value={oauthClientId} onChange={(e) => setOauthClientId(e.target.value)} placeholder="your-oauth-client-id" required />
                      </div>
                      <div>
                        <label className="block text-sm font-medium mb-2">Client Secret</label>
                        <Input type="password" value={oauthClientSecret} onChange={(e) => setOauthClientSecret(e.target.value)} placeholder="your-oauth-client-secret" required />
                        <p className="text-xs text-muted-foreground mt-1">Stored securely in system keychain</p>
                      </div>
                    </div>
                  )}
                </div>
              )}
              {transportType === "Sse" && (
                <div>
                  <label className="block text-sm font-medium mb-2">Headers (Optional)</label>
                  <KeyValueInput value={headers} onChange={setHeaders} keyPlaceholder="Header Name" valuePlaceholder="Header Value" />
                </div>
              )}
              <div className="flex justify-end gap-2 pt-4">
                <Button type="button" variant="secondary" onClick={() => { setShowCreateModal(false); resetForm() }} disabled={isCreating}>Cancel</Button>
                <Button type="submit" disabled={isCreating}>{isCreating ? "Creating..." : "Create"}</Button>
              </div>
            </form>
          </div>
        )}
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
              <p className="text-xs text-muted-foreground truncate">Auth: {oauthDiscovery.auth_url}</p>
              <p className="text-xs text-muted-foreground truncate">Token: {oauthDiscovery.token_url}</p>
              {oauthDiscovery.scopes && oauthDiscovery.scopes.length > 0 && (
                <p className="text-xs text-muted-foreground">Scopes: {oauthDiscovery.scopes.join(", ")}</p>
              )}
            </div>
          )}
          <div>
            <label className="block text-sm font-medium mb-2">Client ID</label>
            <Input value={oauthSetupClientId} onChange={(e) => setOauthSetupClientId(e.target.value)} placeholder="your-oauth-app-client-id" />
            <p className="text-xs text-muted-foreground mt-1">Create an OAuth app in your provider's settings</p>
          </div>
          <div>
            <label className="block text-sm font-medium mb-2">Client Secret</label>
            <Input type="password" value={oauthSetupClientSecret} onChange={(e) => setOauthSetupClientSecret(e.target.value)} placeholder="your-oauth-app-client-secret" />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setShowOAuthSetup(false)} disabled={isSavingOAuth}>Cancel</Button>
            <Button onClick={handleSaveOAuthCredentials} disabled={!oauthSetupClientId || !oauthSetupClientSecret || isSavingOAuth}>
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
          <AlertDialogAction onClick={handleDelete} className="bg-destructive text-destructive-foreground hover:bg-destructive/90">
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    </>
  )
}
