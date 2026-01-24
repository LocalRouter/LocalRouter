/**
 * MCP Servers View
 *
 * Standalone view for managing MCP (Model Context Protocol) servers.
 */

import { McpServersPanel } from "../resources/mcp-servers-panel"

interface McpServersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function McpServersView({ activeSubTab, onTabChange }: McpServersViewProps) {
  // Parse subTab to get selected server ID
  const selectedId = activeSubTab || null

  const handleSelect = (id: string | null) => {
    onTabChange("mcp-servers", id)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight">MCP Servers</h1>
        <p className="text-sm text-muted-foreground">
          Manage Model Context Protocol server connections
        </p>
      </div>

      <div className="flex-1 min-h-0">
        <McpServersPanel
          selectedId={selectedId}
          onSelect={handleSelect}
        />
      </div>
    </div>
  )
}
