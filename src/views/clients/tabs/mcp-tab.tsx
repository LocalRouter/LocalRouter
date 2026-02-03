
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Checkbox } from "@/components/ui/checkbox"
import { cn } from "@/lib/utils"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
  mcp_deferred_loading: boolean
  marketplace_enabled: boolean
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
  const [marketplaceEnabled, setMarketplaceEnabled] = useState(client.marketplace_enabled)

  useEffect(() => {
    loadServers()
  }, [])

  // Sync local state when client prop changes
  useEffect(() => {
    setIncludeAllServers(client.mcp_access_mode === "all")
    setSelectedServers(new Set(client.mcp_servers))
    setDeferredLoading(client.mcp_deferred_loading)
    setMarketplaceEnabled(client.marketplace_enabled)
  }, [client.mcp_access_mode, client.mcp_servers, client.mcp_deferred_loading, client.marketplace_enabled])

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

  const handleAllServersToggle = async () => {
    try {
      setSaving(true)
      const newIncludeAll = !includeAllServers

      if (newIncludeAll) {
        // Enable all servers mode
        await invoke("set_client_mcp_access", {
          clientId: client.client_id,
          mode: "all",
          servers: [],
        })
        setIncludeAllServers(true)
        toast.success("All MCP servers enabled")
      } else {
        // Switch to specific mode with current selections
        const mode = selectedServers.size > 0 ? "specific" : "none"
        await invoke("set_client_mcp_access", {
          clientId: client.client_id,
          mode,
          servers: Array.from(selectedServers),
        })
        setIncludeAllServers(false)
        toast.success("Switched to specific server selection")
      }
      onUpdate()
    } catch (error) {
      console.error("Failed to update MCP access:", error)
      toast.error("Failed to update MCP settings")
    } finally {
      setSaving(false)
    }
  }

  const handleServerToggle = async (serverId: string) => {
    // If includeAllServers is true, we need to demote to specific mode minus this server
    if (includeAllServers) {
      try {
        setSaving(true)
        const otherServers = servers
          .filter(s => s.id !== serverId && s.enabled)
          .map(s => s.id)

        await invoke("set_client_mcp_access", {
          clientId: client.client_id,
          mode: otherServers.length > 0 ? "specific" : "none",
          servers: otherServers,
        })

        setIncludeAllServers(false)
        setSelectedServers(new Set(otherServers))
        toast.success("MCP server access updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update MCP server:", error)
        toast.error("Failed to update server access")
      } finally {
        setSaving(false)
      }
      return
    }

    try {
      setSaving(true)
      const newSelected = new Set(selectedServers)

      if (newSelected.has(serverId)) {
        newSelected.delete(serverId)
      } else {
        newSelected.add(serverId)
      }

      const mode = newSelected.size > 0 ? "specific" : "none"
      await invoke("set_client_mcp_access", {
        clientId: client.client_id,
        mode,
        servers: Array.from(newSelected),
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

  const handleToggleMarketplace = async () => {
    try {
      setSaving(true)
      await invoke("set_client_marketplace_enabled", {
        clientId: client.client_id,
        enabled: !marketplaceEnabled,
      })
      setMarketplaceEnabled(!marketplaceEnabled)
      toast.success("Marketplace " + (!marketplaceEnabled ? "enabled" : "disabled"))
      onUpdate()
    } catch (error) {
      console.error("Failed to update marketplace access:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  const enabledServerCount = servers.filter((s) => s.enabled).length
  const selectedCount = includeAllServers
    ? enabledServerCount
    : Array.from(selectedServers).filter((id) =>
        servers.find((s) => s.id === id)?.enabled
      ).length

  // Check if indeterminate (some but not all selected)
  const isIndeterminate = !includeAllServers && selectedCount > 0 && selectedCount < enabledServerCount

  const isServerSelected = (serverId: string): boolean => {
    if (includeAllServers) return true
    return selectedServers.has(serverId)
  }

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
        <CardContent>
          {loading ? (
            <div className="p-8 text-center text-muted-foreground text-sm">
              Loading servers...
            </div>
          ) : servers.length === 0 ? (
            <div className="p-8 text-center text-muted-foreground text-sm">
              No MCP servers configured. Add MCP servers in the Resources tab.
            </div>
          ) : (
            <div className="border rounded-lg">
              <div className="max-h-[400px] overflow-y-auto">
                {/* All MCP Servers row */}
                <div
                  className="flex items-center gap-3 px-4 py-3 border-b bg-background sticky top-0 z-10 cursor-pointer hover:bg-muted/50 transition-colors"
                  onClick={() => !saving && handleAllServersToggle()}
                >
                  <Checkbox
                    checked={includeAllServers || isIndeterminate}
                    onCheckedChange={handleAllServersToggle}
                    disabled={saving}
                    className={cn(
                      "data-[state=checked]:bg-primary",
                      isIndeterminate && "data-[state=checked]:bg-primary/60"
                    )}
                  />
                  <span className="font-semibold text-sm">
                    All MCP Servers
                  </span>
                  <span className="text-xs text-muted-foreground ml-auto">
                    {includeAllServers ? (
                      <span className="text-primary">All (including future servers)</span>
                    ) : (
                      `${selectedCount} / ${enabledServerCount} selected`
                    )}
                  </span>
                </div>

                {/* Individual server rows */}
                {servers.map((server) => {
                  const isSelected = isServerSelected(server.id)
                  const isDisabled = !server.enabled
                  // Server row is clickable when all servers mode is not active
                  const canToggle = !saving && !isDisabled

                  return (
                    <div
                      key={server.id}
                      className={cn(
                        "flex items-center gap-3 px-4 py-2.5 border-b border-border/50",
                        "hover:bg-muted/30 transition-colors",
                        canToggle ? "cursor-pointer" : "",
                        isDisabled && "opacity-50",
                        includeAllServers && !isDisabled && "opacity-60"
                      )}
                      style={{ paddingLeft: "2rem" }}
                      onClick={() => canToggle && handleServerToggle(server.id)}
                    >
                      <Checkbox
                        checked={isSelected}
                        onCheckedChange={() => handleServerToggle(server.id)}
                        disabled={!canToggle}
                      />
                      <div className="flex-1 min-w-0">
                        <span className="text-sm font-medium">{server.name}</span>
                        {isDisabled && (
                          <span className="ml-2 text-xs text-muted-foreground">(Disabled)</span>
                        )}
                      </div>
                      <code className="text-xs text-muted-foreground truncate max-w-[200px]">
                        {server.proxy_url}
                      </code>
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Deferred Loading */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <CardTitle className="text-base">Deferred Loading</CardTitle>
              <span className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-700 dark:text-purple-300 font-medium">
                EXPERIMENTAL
              </span>
            </div>
            <Switch
              checked={deferredLoading}
              onCheckedChange={handleToggleDeferredLoading}
              disabled={saving}
            />
          </div>
          <CardDescription>
            Optimize token usage by loading MCP capabilities on-demand
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* How it works - Blue info panel */}
          <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-500/20">
            <div className="flex items-start gap-3">
              <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
              <div className="space-y-2">
                <p className="text-sm font-medium text-blue-700 dark:text-blue-300">
                  How Deferred Loading Works
                </p>
                <p className="text-sm text-blue-600/90 dark:text-blue-400/90">
                  Deferred loading reduces the initial token overhead by not sending all
                  tool definitions upfront. Instead, a single search tool is provided
                  that allows the LLM to discover and load tools on-demand.
                </p>
                <p className="text-sm text-blue-600/90 dark:text-blue-400/90">
                  If client does not support dynamic tool loading via{" "}
                  <code className="px-1 py-0.5 rounded bg-blue-500/20 text-xs">
                    tools/listChanged
                  </code>, deferred loading is automatically disabled.
                </p>
              </div>
            </div>
          </div>

          {deferredLoading && (
            <div className="p-3 rounded-lg border border-amber-500/30 bg-amber-500/10">
              <p className="text-sm text-amber-600 dark:text-amber-400">
                This feature is experimental and may not work with all clients.
                Ensure your client properly handles the{" "}
                <code className="px-1 py-0.5 rounded bg-muted text-xs">
                  tools/listChanged
                </code>{" "}
                notification before enabling.
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Marketplace Access */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">Marketplace Access</CardTitle>
            <Switch
              checked={marketplaceEnabled}
              onCheckedChange={handleToggleMarketplace}
              disabled={saving}
            />
          </div>
          <CardDescription>
            Allow this client to search and install MCP servers and skills from the marketplace
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="p-4 rounded-lg bg-muted/50 border">
            <p className="text-sm text-muted-foreground">
              When enabled, this client will have access to 4 marketplace tools:
            </p>
            <ul className="list-disc list-inside mt-2 text-sm text-muted-foreground space-y-1">
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__search_mcp_servers</code> - Search the MCP registry</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__install_mcp_server</code> - Install an MCP server</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__search_skills</code> - Browse skill repositories</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__install_skill</code> - Install a skill</li>
            </ul>
          </div>
          {marketplaceEnabled && (
            <div className="p-3 rounded-lg border border-blue-500/30 bg-blue-500/10">
              <p className="text-sm text-blue-600 dark:text-blue-400">
                Install requests from AI clients will show a confirmation dialog before proceeding.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
