/**
 * MCP Servers View
 *
 * Standalone view for managing MCP (Model Context Protocol) servers.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { McpIcon } from "@/components/icons/category-icons"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { McpServersPanel, McpHealthStatus, McpHealthCheckEvent } from "../resources/mcp-servers-panel"
import { McpSettingsPanel } from "./mcp-settings-panel"

interface McpServersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function McpServersView({ activeSubTab, onTabChange }: McpServersViewProps) {
  // Parse subTab to determine server selection
  // Format: "server-id" or "add/template-id"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { selectedId: null, addTemplateId: null }
    if (subTab.startsWith("add/")) {
      return { selectedId: null, addTemplateId: subTab.slice(4) }
    }
    return { selectedId: subTab, addTemplateId: null }
  }

  const { selectedId, addTemplateId } = parseSubTab(activeSubTab)

  // Lifted health status state - persists across interactions
  const [healthStatus, setHealthStatus] = useState<Record<string, McpHealthStatus>>({})
  const [healthInitialized, setHealthInitialized] = useState(false)

  // Start health checks for all servers (called once on mount)
  const startHealthChecks = useCallback(async (serverIds: string[]) => {
    setHealthStatus((prev) => {
      const updated = { ...prev }
      for (const id of serverIds) {
        if (!updated[id]) {
          updated[id] = { status: "pending" }
        }
      }
      return updated
    })

    try {
      await invoke("start_mcp_health_checks")
    } catch (error) {
      console.error("Failed to start MCP health checks:", error)
    }
  }, [])

  // Refresh health for a single server
  const refreshHealth = useCallback(async (serverId: string) => {
    setHealthStatus((prev) => ({
      ...prev,
      [serverId]: { status: "pending" },
    }))
    await invoke("check_single_mcp_health", { serverId })
  }, [])

  // Listen for health check events
  useEffect(() => {
    const lHealth = listenSafe<McpHealthCheckEvent>("mcp-health-check", (event) => {
      const { server_id, status, latency_ms, error } = event.payload
      setHealthStatus((prev) => ({
        ...prev,
        [server_id]: {
          status: status as McpHealthStatus["status"],
          latency_ms,
          error,
        },
      }))
    })

    interface ItemHealth {
      name: string
      status: string
      latency_ms?: number
      error?: string
    }
    interface HealthCacheState {
      mcp_servers: Record<string, ItemHealth>
    }
    const lCacheChanged = listenSafe<HealthCacheState>("health-status-changed", (event) => {
      const { mcp_servers } = event.payload
      if (!mcp_servers) return
      setHealthStatus((prev) => {
        const updated = { ...prev }
        for (const [id, health] of Object.entries(mcp_servers)) {
          updated[id] = {
            status: health.status as McpHealthStatus["status"],
            latency_ms: health.latency_ms,
            error: health.error,
          }
        }
        return updated
      })
    })

    return () => {
      lHealth.cleanup()
      lCacheChanged.cleanup()
    }
  }, [])

  const handleSelect = (id: string | null) => {
    onTabChange("mcp-servers", id)
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      {selectedId ? (
        <div className="flex-shrink-0 pb-2">
          <Button variant="ghost" size="sm" className="gap-1 -ml-2" onClick={() => handleSelect(null)}>
            <ArrowLeft className="h-3 w-3" />
            Back to MCPs
          </Button>
        </div>
      ) : (
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><McpIcon className="h-6 w-6" />MCP</h1>
          <p className="text-sm text-muted-foreground">
            Connect to external MCP servers and aggregate their tools, prompts, and resources into the unified MCP gateway that clients connect to.
          </p>
        </div>
      )}

      {selectedId ? (
        <McpServersPanel
          selectedId={selectedId}
          onSelect={handleSelect}
          healthStatus={healthStatus}
          onHealthInit={(serverIds) => {
            if (!healthInitialized) {
              setHealthInitialized(true)
              startHealthChecks(serverIds)
            }
          }}
          onRefreshHealth={refreshHealth}
          initialAddTemplateId={addTemplateId}
          onViewChange={onTabChange}
        />
      ) : (
        <Tabs defaultValue="servers" className="flex flex-col flex-1 min-h-0">
          <TabsList className="flex-shrink-0 w-fit">
            <TabsTrigger value="servers"><TAB_ICONS.browse className={TAB_ICON_CLASS} />Servers</TabsTrigger>
            <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
          </TabsList>

          <TabsContent value="servers" className="flex-1 min-h-0 mt-4">
            <McpServersPanel
              selectedId={null}
              onSelect={handleSelect}
              healthStatus={healthStatus}
              onHealthInit={(serverIds) => {
                if (!healthInitialized) {
                  setHealthInitialized(true)
                  startHealthChecks(serverIds)
                }
              }}
              onRefreshHealth={refreshHealth}
              initialAddTemplateId={addTemplateId}
              onViewChange={onTabChange}
            />
          </TabsContent>

          <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-auto">
            <McpSettingsPanel />
          </TabsContent>
        </Tabs>
      )}
    </div>
  )
}
