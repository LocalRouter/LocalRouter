/**
 * Step 3: Select MCP Servers
 *
 * MCP server selection.
 * Shows empty state with guidance if no servers configured.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Loader2, Info } from "lucide-react"
import { McpServerSelector } from "@/components/mcp/McpServerSelector"

interface McpServer {
  id: string
  name: string
  enabled: boolean
  proxy_url: string
}

type McpAccessMode = "none" | "all" | "specific"

interface StepMcpProps {
  accessMode: McpAccessMode
  selectedServers: string[]
  onChange: (mode: McpAccessMode, servers: string[]) => void
}

export function StepMcp({ accessMode, selectedServers, onChange }: StepMcpProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadServers()
  }, [])

  const loadServers = async () => {
    try {
      setLoading(true)
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      setServers(serverList)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      setServers([])
    } finally {
      setLoading(false)
    }
  }

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
        <div className="rounded-lg border border-blue-500/30 bg-blue-500/10 p-4">
          <div className="flex items-start gap-3">
            <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-blue-700 dark:text-blue-300">
                No MCP servers configured
              </p>
              <p className="text-sm text-blue-600/90 dark:text-blue-400/90">
                MCP servers provide tools and resources to LLM applications.
                You can add servers in the MCP Servers tab and configure access later.
              </p>
            </div>
          </div>
        </div>
        <p className="text-xs text-muted-foreground text-center">
          You can skip this step and add MCP access later.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Select which MCP servers this client can access.
      </p>

      <McpServerSelector
        servers={servers}
        accessMode={accessMode}
        selectedServers={selectedServers}
        onChange={onChange}
      />

      <p className="text-xs text-muted-foreground">
        MCP servers provide tools and resources like filesystem access, database queries, and more.
      </p>
    </div>
  )
}
