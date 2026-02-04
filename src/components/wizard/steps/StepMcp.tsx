/**
 * Step 3: Select MCP Servers
 *
 * MCP server permission selection using Allow/Ask/Off states.
 * Supports hierarchical permissions for servers, tools, resources, and prompts.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2, Info, Plus, Grid, Store, ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/Modal"
import LegacySelect from "@/components/ui/Select"
import KeyValueInput from "@/components/ui/KeyValueInput"
import { PermissionTreeSelector } from "@/components/permissions/PermissionTreeSelector"
import { McpServerTemplates, McpServerTemplate } from "@/components/mcp/McpServerTemplates"
import { MarketplaceSearchPanel, McpServerListing } from "@/components/add-resource"
import ServiceIcon from "@/components/ServiceIcon"
import type { PermissionState, TreeNode, McpPermissions } from "@/components/permissions/types"

interface McpServer {
  id: string
  name: string
  enabled: boolean
  proxy_url: string
}

interface McpServerCapabilities {
  tools: Array<{ name: string; description: string | null }>
  resources: Array<{ uri: string; name: string; description: string | null }>
  prompts: Array<{ name: string; description: string | null }>
}

interface StepMcpProps {
  permissions: McpPermissions
  onChange: (permissions: McpPermissions) => void
}

export function StepMcp({ permissions, onChange }: StepMcpProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [capabilities, setCapabilities] = useState<Record<string, McpServerCapabilities>>({})
  const [loading, setLoading] = useState(true)

  // MCP server creation state
  const [showAddServer, setShowAddServer] = useState(false)
  const [dialogPage, setDialogPage] = useState<"select" | "configure">("select")
  const [dialogTab, setDialogTab] = useState<"templates" | "marketplace">("templates")
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

  const loadServers = useCallback(async () => {
    try {
      setLoading(true)
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      const enabledServers = serverList.filter((s) => s.enabled)
      setServers(enabledServers)

      // Eagerly load capabilities for all enabled servers
      for (const server of enabledServers) {
        try {
          const caps = await invoke<McpServerCapabilities>("get_mcp_server_capabilities", {
            serverId: server.id,
          })
          setCapabilities((prev) => ({ ...prev, [server.id]: caps }))
        } catch (error) {
          console.error(`Failed to load capabilities for ${server.id}:`, error)
        }
      }
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      setServers([])
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadServers()
  }, [loadServers])

  const resetForm = () => {
    setServerName("")
    setTransportType("Stdio")
    setCommand("")
    setEnvVars({})
    setUrl("")
    setHeaders({})
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

    setDialogPage("configure")
  }

  const handleSelectMarketplaceMcp = (listing: McpServerListing) => {
    setSelectedSource({ type: "marketplace", listing })
    setServerName(listing.name)

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

    setDialogPage("configure")
  }

  const handleBackToSelect = () => {
    setDialogPage("select")
  }

  const handleCreateServer = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsCreating(true)

    try {
      let transportConfig
      if (transportType === "Stdio") {
        transportConfig = {
          type: "stdio",
          command,
          env: envVars,
        }
      } else {
        transportConfig = {
          type: "http_sse",
          url,
          headers: headers,
        }
      }

      await invoke("create_mcp_server", {
        name: serverName || null,
        transport: transportType,
        transportConfig,
        authConfig: null,
      })

      toast.success("MCP server created")
      await loadServers()
      setShowAddServer(false)
      resetForm()
    } catch (error) {
      console.error("Failed to create MCP server:", error)
      toast.error(`Error creating MCP server: ${error}`)
    } finally {
      setIsCreating(false)
    }
  }

  // Handle permission changes
  const handlePermissionChange = (key: string, state: PermissionState, parentState: PermissionState) => {
    // If the new state matches the parent, remove the override (inherit from parent)
    // Otherwise, set an explicit override
    const shouldClear = state === parentState

    // Parse the key to determine the level
    // Format: server_id or server_id__type__name
    const parts = key.split("__")

    const newPermissions = { ...permissions }

    if (parts.length === 1) {
      // Server level
      const newServers = { ...permissions.servers }
      if (shouldClear) {
        delete newServers[key]
      } else {
        newServers[key] = state
      }
      newPermissions.servers = newServers
    } else if (parts.length === 3) {
      // Tool/resource/prompt level
      const [serverId, type, name] = parts
      const compositeKey = `${serverId}__${name}`

      if (type === "tool") {
        const newTools = { ...permissions.tools }
        if (shouldClear) {
          delete newTools[compositeKey]
        } else {
          newTools[compositeKey] = state
        }
        newPermissions.tools = newTools
      } else if (type === "resource") {
        const newResources = { ...permissions.resources }
        if (shouldClear) {
          delete newResources[compositeKey]
        } else {
          newResources[compositeKey] = state
        }
        newPermissions.resources = newResources
      } else if (type === "prompt") {
        const newPrompts = { ...permissions.prompts }
        if (shouldClear) {
          delete newPrompts[compositeKey]
        } else {
          newPrompts[compositeKey] = state
        }
        newPermissions.prompts = newPrompts
      }
    }

    onChange(newPermissions)
  }

  const handleGlobalChange = (state: PermissionState) => {
    // Clear all child customizations when global changes
    onChange({
      global: state,
      servers: {},
      tools: {},
      resources: {},
      prompts: {},
    })
  }

  // Build tree nodes from servers
  const buildTree = (): TreeNode[] => {
    return servers.map((server) => {
      const caps = capabilities[server.id]
      const children: TreeNode[] = []

      if (caps) {
        // Tools group
        if (caps.tools.length > 0) {
          children.push({
            id: `${server.id}__tools`,
            label: "Tools",
            isGroup: true,
            children: caps.tools.map((tool) => ({
              id: `${server.id}__tool__${tool.name}`,
              label: tool.name,
              description: tool.description || undefined,
            })),
          })
        }

        // Resources group
        if (caps.resources.length > 0) {
          children.push({
            id: `${server.id}__resources`,
            label: "Resources",
            isGroup: true,
            children: caps.resources.map((res) => ({
              id: `${server.id}__resource__${res.uri}`,
              label: res.name,
              description: res.description || undefined,
            })),
          })
        }

        // Prompts group
        if (caps.prompts.length > 0) {
          children.push({
            id: `${server.id}__prompts`,
            label: "Prompts",
            isGroup: true,
            children: caps.prompts.map((prompt) => ({
              id: `${server.id}__prompt__${prompt.name}`,
              label: prompt.name,
              description: prompt.description || undefined,
            })),
          })
        }
      }

      return {
        id: server.id,
        label: server.name,
        children: children.length > 0 ? children : undefined,
      }
    })
  }

  // Build flat permissions map for the tree
  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}

    // Server permissions
    for (const [serverId, state] of Object.entries(permissions.servers)) {
      map[serverId] = state
    }

    // Tool permissions
    for (const [key, state] of Object.entries(permissions.tools)) {
      const [serverId, toolName] = key.split("__")
      map[`${serverId}__tool__${toolName}`] = state
    }

    // Resource permissions
    for (const [key, state] of Object.entries(permissions.resources)) {
      const [serverId, uri] = key.split("__")
      map[`${serverId}__resource__${uri}`] = state
    }

    // Prompt permissions
    for (const [key, state] of Object.entries(permissions.prompts)) {
      const [serverId, promptName] = key.split("__")
      map[`${serverId}__prompt__${promptName}`] = state
    }

    return map
  }

  // Render the dialog content
  const renderAddMcpDialog = () => (
    <Dialog
      open={showAddServer}
      onOpenChange={(open) => {
        if (!open) {
          setShowAddServer(false)
          resetForm()
        }
      }}
    >
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add MCP</DialogTitle>
        </DialogHeader>

        {dialogPage === "select" ? (
          /* Page 1: Selection */
          <Tabs value={dialogTab} onValueChange={(v) => setDialogTab(v as typeof dialogTab)}>
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="templates" className="gap-2">
                <Grid className="h-4 w-4" />
                Templates
              </TabsTrigger>
              <TabsTrigger value="marketplace" className="gap-2">
                <Store className="h-4 w-4" />
                Marketplace
              </TabsTrigger>
            </TabsList>

            {/* Templates Tab */}
            <TabsContent value="templates" className="mt-4">
              <McpServerTemplates onSelectTemplate={handleSelectTemplate} />
            </TabsContent>

            {/* Marketplace Tab */}
            <TabsContent value="marketplace" className="mt-4">
              <MarketplaceSearchPanel
                type="mcp"
                onSelectMcp={handleSelectMarketplaceMcp}
                maxHeight="400px"
              />
            </TabsContent>
          </Tabs>
        ) : (
          /* Page 2: Configuration Form */
          <div className="space-y-4">
            {/* Back button and source header */}
            <div className="flex items-center gap-3 pb-2 border-b">
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
              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-400 dark:border-blue-800 rounded p-3">
                <p className="text-xs text-foreground">
                  {selectedSource.template.setupInstructions}
                </p>
              </div>
            )}
            {selectedSource?.type === "marketplace" && selectedSource.listing?.install_hint && (
              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-400 dark:border-blue-800 rounded p-3">
                <p className="text-xs text-foreground">
                  {selectedSource.listing.install_hint}
                </p>
              </div>
            )}

            <form onSubmit={handleCreateServer} className="space-y-4">
              <div>
                <Label className="mb-2 block">Server Name</Label>
                <Input
                  value={serverName}
                  onChange={(e) => setServerName(e.target.value)}
                  placeholder="My MCP Server"
                  required
                />
              </div>

              <div>
                <Label className="mb-2 block">Transport Type</Label>
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
                    <Label className="mb-2 block">Command</Label>
                    <Input
                      value={command}
                      onChange={(e) => setCommand(e.target.value)}
                      placeholder="npx -y @modelcontextprotocol/server-everything"
                      required
                    />
                    <p className="text-xs text-muted-foreground mt-1">
                      Full command with arguments
                    </p>
                  </div>

                  <div>
                    <Label className="mb-2 block">Environment Variables</Label>
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
                    <Label className="mb-2 block">URL</Label>
                    <Input
                      value={url}
                      onChange={(e) => setUrl(e.target.value)}
                      placeholder="https://api.example.com/mcp"
                      required
                    />
                  </div>

                  <div>
                    <Label className="mb-2 block">Headers (Optional)</Label>
                    <KeyValueInput
                      value={headers}
                      onChange={setHeaders}
                      keyPlaceholder="Header Name"
                      valuePlaceholder="Header Value"
                    />
                  </div>
                </>
              )}

              <div className="flex justify-end gap-2 pt-4">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setShowAddServer(false)
                    resetForm()
                  }}
                  disabled={isCreating}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={isCreating}>
                  {isCreating ? "Creating..." : "Create"}
                </Button>
              </div>
            </form>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (servers.length === 0) {
    return (
      <div className="space-y-4">
        <div className="rounded-lg border border-blue-600/50 bg-blue-500/10 p-4">
          <div className="flex items-start gap-3">
            <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-blue-900 dark:text-blue-300">
                No MCP servers configured
              </p>
              <p className="text-sm text-blue-900 dark:text-blue-400">
                MCP servers provide tools and resources to LLM applications.
                You can add servers now or configure access later.
              </p>
            </div>
          </div>
        </div>

        <Button onClick={() => setShowAddServer(true)} className="w-full">
          <Plus className="h-4 w-4 mr-2" />
          Add MCP
        </Button>

        <p className="text-xs text-muted-foreground text-center">
          You can skip this step and add MCP access later.
        </p>

        {renderAddMcpDialog()}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Configure MCP server access for this client. Use Allow, Ask, or Off for each server.
      </p>

      <PermissionTreeSelector
        nodes={buildTree()}
        permissions={buildPermissionsMap()}
        globalPermission={permissions.global}
        onPermissionChange={handlePermissionChange}
        onGlobalChange={handleGlobalChange}
        globalLabel="All MCP Servers"
        emptyMessage="No MCP servers configured"
      />

      <div className="flex items-center justify-between pt-2">
        <p className="text-xs text-muted-foreground">
          MCP servers provide tools and resources like filesystem access, database queries, and more.
        </p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setShowAddServer(true)}
        >
          <Plus className="h-3 w-3 mr-1" />
          Add Server
        </Button>
      </div>

      {renderAddMcpDialog()}
    </div>
  )
}
