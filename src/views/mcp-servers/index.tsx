/**
 * MCP Servers View
 *
 * Standalone view for managing MCP (Model Context Protocol) servers.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { McpIcon } from "@/components/icons/category-icons"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { McpServersPanel, McpHealthStatus, McpHealthCheckEvent } from "../resources/mcp-servers-panel"
import { McpTab } from "@/views/try-it-out/mcp-tab"

interface McpServersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function McpServersView({ activeSubTab, onTabChange }: McpServersViewProps) {
  // Parse subTab to determine top-level tab and server selection
  // Format: "try-it-out" or "try-it-out/init/..." or "server-id" or "add/template-id"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { topTab: "servers", selectedId: null, addTemplateId: null, tryItOutInit: null as string | null }
    if (subTab === "try-it-out") return { topTab: "try-it-out", selectedId: null, addTemplateId: null, tryItOutInit: null as string | null }
    if (subTab.startsWith("try-it-out/")) return { topTab: "try-it-out", selectedId: null, addTemplateId: null, tryItOutInit: subTab.slice(11) }
    if (subTab.startsWith("add/")) {
      return { topTab: "servers", selectedId: null, addTemplateId: subTab.slice(4), tryItOutInit: null as string | null }
    }
    return { topTab: "servers", selectedId: subTab, addTemplateId: null, tryItOutInit: null as string | null }
  }

  const { topTab, selectedId, addTemplateId, tryItOutInit } = parseSubTab(activeSubTab)

  // Lifted health status state - persists across interactions
  const [healthStatus, setHealthStatus] = useState<Record<string, McpHealthStatus>>({})
  const [healthInitialized, setHealthInitialized] = useState(false)

  // Start health checks for all servers (called once on mount)
  const startHealthChecks = useCallback(async (serverIds: string[]) => {
    // Set servers to pending state (only for new servers)
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

  // Listen for health check events (individual MCP checks)
  useEffect(() => {
    const unsubHealth = listen<McpHealthCheckEvent>("mcp-health-check", (event) => {
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

    // Listen for global health cache updates (e.g. from sidebar refresh button)
    interface ItemHealth {
      name: string
      status: string
      latency_ms?: number
      error?: string
    }
    interface HealthCacheState {
      mcp_servers: Record<string, ItemHealth>
    }
    const unsubCacheChanged = listen<HealthCacheState>("health-status-changed", (event) => {
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
      unsubHealth.then((fn) => fn())
      unsubCacheChanged.then((fn) => fn())
    }
  }, [])

  const handleSelect = (id: string | null) => {
    onTabChange("mcp-servers", id)
  }

  const handleTopTabChange = (tab: string) => {
    onTabChange("mcp-servers", tab === "servers" ? null : tab)
  }

  // Parse init path for try-it-out
  const parseTryItOutInit = () => {
    if (!tryItOutInit || !tryItOutInit.startsWith("init/")) return {}
    const parts = tryItOutInit.slice(5).split("/")
    const mode = parts[0] as "client" | "all" | "direct" | undefined
    const target = parts.slice(1).join("/") || undefined
    if (mode === "client" && target) return { initialMode: mode, initialClientId: target }
    if (mode === "direct" && target) return { initialMode: mode, initialDirectTarget: target }
    return {}
  }

  const tryItOutInitProps = parseTryItOutInit()

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><McpIcon className="h-6 w-6" />MCP</h1>
        <div className="flex items-center gap-2">
          <p className="text-sm text-muted-foreground">
            Connect to external MCP servers and aggregate their tools, prompts, and resources into the unified MCP gateway that clients connect to.
          </p>
          <SamplePopupButton popupType="mcp_tool" />
        </div>
      </div>

      <Tabs
        value={topTab}
        onValueChange={handleTopTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="servers">Servers</TabsTrigger>
          <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>
        </TabsList>

        <TabsContent value="servers" className="flex-1 min-h-0 mt-4">
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
        </TabsContent>

        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <McpTab
            innerPath={null}
            onPathChange={() => {}}
            {...tryItOutInitProps}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
