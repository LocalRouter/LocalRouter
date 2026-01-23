
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info, FlaskConical } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Switch } from "@/components/ui/Toggle"
import { Label } from "@/components/ui/label"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
  mcp_deferred_loading: boolean
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
  proxy_url: string
  gateway_url: string
}

interface McpTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientMcpTab({ client, onUpdate }: McpTabProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  // Local state for UI
  const [includeAllServers, setIncludeAllServers] = useState(client.mcp_access_mode === "all")
  const [selectedServers, setSelectedServers] = useState<Set<string>>(
    new Set(client.mcp_servers)
  )
  const [deferredLoading, setDeferredLoading] = useState(client.mcp_deferred_loading)

  useEffect(() => {
    loadServers()
  }, [])

  // Sync local state when client prop changes
  useEffect(() => {
    setIncludeAllServers(client.mcp_access_mode === "all")
    setSelectedServers(new Set(client.mcp_servers))
    setDeferredLoading(client.mcp_deferred_loading)
  }, [client.mcp_access_mode, client.mcp_servers, client.mcp_deferred_loading])

  const loadServers = async () => {
    try {
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
    } finally {
      setLoading(false)
    }
  }

  const handleIncludeAllChange = async (checked: boolean) => {
    try {
      setSaving(true)
      const mode = checked ? "all" : (selectedServers.size > 0 ? "specific" : "none")
      const serverIds = checked ? [] : Array.from(selectedServers)

      await invoke("set_client_mcp_access", {
        clientId: client.client_id,
        mode,
        servers: serverIds,
      })

      setIncludeAllServers(checked)
      toast.success(checked ? "All MCP servers enabled" : "Switched to specific server selection")
      onUpdate()
    } catch (error) {
      console.error("Failed to update MCP access:", error)
      toast.error("Failed to update MCP settings")
    } finally {
      setSaving(false)
    }
  }

  const handleServerToggle = async (serverId: string) => {
    try {
      setSaving(true)
      const newSelected = new Set(selectedServers)

      if (newSelected.has(serverId)) {
        newSelected.delete(serverId)
      } else {
        newSelected.add(serverId)
      }

      const mode = newSelected.size > 0 ? "specific" : "none"
      const serverIds = Array.from(newSelected)

      await invoke("set_client_mcp_access", {
        clientId: client.client_id,
        mode,
        servers: serverIds,
      })

      setSelectedServers(newSelected)
      toast.success("MCP server access updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update MCP server:", error)
      toast.error("Failed to update server access")
    } finally {
      setSaving(false)
    }
  }

  const handleToggleDeferredLoading = async () => {
    try {
      setSaving(true)
      await invoke("toggle_client_deferred_loading", {
        clientId: client.client_id,
        enabled: !deferredLoading,
      })
      setDeferredLoading(!deferredLoading)
      toast.success("Deferred loading " + (!deferredLoading ? "enabled" : "disabled"))
      onUpdate()
    } catch (error) {
      console.error("Failed to update deferred loading:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  const enabledServerCount = servers.filter((s) => s.enabled).length
  const accessibleServerCount = includeAllServers
    ? enabledServerCount
    : Array.from(selectedServers).filter((id) =>
        servers.find((s) => s.id === id)?.enabled
      ).length

  return (
    <div className="space-y-6">
      {/* MCP Server Access */}
      <Card>
        <CardHeader>
          <CardTitle>MCP Server Access</CardTitle>
          <CardDescription>
            Select which MCP servers this client can access
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          {/* Include All Servers Toggle */}
          <div className="flex items-start justify-between gap-4 p-4 rounded-lg border bg-muted/50">
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Label htmlFor="include-all" className="font-medium">
                  Include All Servers
                </Label>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Info className="h-4 w-4 text-muted-foreground cursor-help" />
                    </TooltipTrigger>
                    <TooltipContent className="max-w-xs">
                      <p>
                        When enabled, this client will automatically have access to
                        all MCP servers, including any new servers added in the future.
                      </p>
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </div>
              <p className="text-sm text-muted-foreground">
                Automatically grant access to all current and future MCP servers
              </p>
            </div>
            <Switch
              id="include-all"
              checked={includeAllServers}
              onCheckedChange={handleIncludeAllChange}
              disabled={saving || loading}
            />
          </div>

          {/* Server Selection (only shown when Include All is off) */}
          {!includeAllServers && (
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <Label className="text-sm font-medium">Select Specific Servers</Label>
                <span className="text-xs text-muted-foreground">
                  {accessibleServerCount} of {enabledServerCount} servers selected
                </span>
              </div>

              {loading ? (
                <p className="text-sm text-muted-foreground py-4 text-center">
                  Loading servers...
                </p>
              ) : servers.length === 0 ? (
                <div className="py-8 text-center border rounded-lg bg-muted/30">
                  <p className="text-sm text-muted-foreground">
                    No MCP servers configured
                  </p>
                  <p className="text-xs text-muted-foreground mt-1">
                    Add MCP servers in the Resources tab
                  </p>
                </div>
              ) : (
                <div className="space-y-2 border rounded-lg divide-y">
                  {servers.map((server) => {
                    const isSelected = selectedServers.has(server.id)
                    const isDisabled = !server.enabled

                    return (
                      <label
                        key={server.id}
                        className={`flex items-center gap-3 p-3 cursor-pointer hover:bg-muted/50 transition-colors first:rounded-t-lg last:rounded-b-lg ${
                          isDisabled ? "opacity-50 cursor-not-allowed" : ""
                        }`}
                      >
                        <Checkbox
                          checked={isSelected}
                          onCheckedChange={() => !isDisabled && handleServerToggle(server.id)}
                          disabled={saving || isDisabled}
                        />
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="font-medium truncate">{server.name}</span>
                            {isDisabled && (
                              <Badge variant="secondary" className="text-xs">
                                Disabled
                              </Badge>
                            )}
                          </div>
                          <code className="text-xs text-muted-foreground block truncate">
                            {server.proxy_url}
                          </code>
                        </div>
                      </label>
                    )
                  })}
                </div>
              )}
            </div>
          )}

          {/* Access Summary */}
          {includeAllServers && (
            <div className="p-3 rounded-lg border border-green-500/30 bg-green-500/10">
              <p className="text-sm text-green-600 dark:text-green-400">
                This client has access to all {enabledServerCount} enabled MCP servers
                and will automatically gain access to new servers as they are added.
              </p>
            </div>
          )}

          {!includeAllServers && accessibleServerCount === 0 && (
            <div className="p-3 rounded-lg border border-amber-500/30 bg-amber-500/10">
              <p className="text-sm text-amber-600 dark:text-amber-400">
                This client has no MCP server access. Select servers above or enable
                &quot;Include All Servers&quot; to grant access.
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Deferred Loading */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <CardTitle>Deferred Loading</CardTitle>
            <Badge variant="outline" className="text-xs">
              <FlaskConical className="h-3 w-3 mr-1" />
              Experimental
            </Badge>
          </div>
          <CardDescription>
            Optimize token usage by loading MCP capabilities on-demand
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Toggle */}
          <div className="flex items-center justify-between p-4 rounded-lg border">
            <div className="space-y-0.5">
              <Label htmlFor="deferred-loading">Enable Deferred Loading</Label>
              <p className="text-sm text-muted-foreground">
                Load tool definitions only when needed
              </p>
            </div>
            <Switch
              id="deferred-loading"
              checked={deferredLoading}
              onCheckedChange={handleToggleDeferredLoading}
              disabled={saving}
            />
          </div>

          {/* Documentation */}
          <Alert>
            <Info className="h-4 w-4" />
            <AlertTitle>How Deferred Loading Works</AlertTitle>
            <AlertDescription className="space-y-3 mt-2">
              <p>
                Deferred loading reduces the initial token overhead by not sending all
                tool definitions upfront. Instead, a single search tool is provided
                that allows the LLM to discover and load tools on-demand.
              </p>

              <div className="space-y-2">
                <p className="font-medium text-sm">Requirements:</p>
                <ul className="list-disc list-inside text-sm space-y-1 ml-2">
                  <li>
                    Client must support the{" "}
                    <code className="px-1 py-0.5 rounded bg-muted text-xs">
                      tools/listChanged
                    </code>{" "}
                    notification
                  </li>
                  <li>Client must handle dynamic tool list updates mid-session</li>
                </ul>
              </div>

              <div className="space-y-2">
                <p className="font-medium text-sm">Behavior:</p>
                <ul className="list-disc list-inside text-sm space-y-1 ml-2">
                  <li>
                    A <code className="px-1 py-0.5 rounded bg-muted text-xs">search_tools</code>{" "}
                    tool is provided to discover available tools
                  </li>
                  <li>
                    When tools are searched, a{" "}
                    <code className="px-1 py-0.5 rounded bg-muted text-xs">
                      tools/listChanged
                    </code>{" "}
                    notification is sent
                  </li>
                  <li>Discovered tools remain available for the rest of the session</li>
                </ul>
              </div>
            </AlertDescription>
          </Alert>

          {deferredLoading && (
            <Alert variant="warning">
              <FlaskConical className="h-4 w-4" />
              <AlertTitle>Experimental Feature</AlertTitle>
              <AlertDescription>
                This feature is experimental and may not work with all clients.
                Ensure your client properly handles the{" "}
                <code className="px-1 py-0.5 rounded bg-muted text-xs">
                  tools/listChanged
                </code>{" "}
                notification before enabling.
              </AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
