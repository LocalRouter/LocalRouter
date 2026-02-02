/**
 * McpServerSelector Component
 *
 * Reusable checkbox selector for MCP server access control.
 * Supports three modes:
 * - All: Access to all servers (including future ones)
 * - Specific: Access only to selected servers
 * - None: No MCP access
 */

import { Checkbox } from "@/components/ui/checkbox"
import { cn } from "@/lib/utils"

interface McpServer {
  id: string
  name: string
  enabled: boolean
  proxy_url?: string
}

type McpAccessMode = "none" | "all" | "specific"

interface McpServerSelectorProps {
  servers: McpServer[]
  accessMode: McpAccessMode
  selectedServers: string[]
  onChange: (mode: McpAccessMode, servers: string[]) => void
  loading?: boolean
  disabled?: boolean
  className?: string
}

export function McpServerSelector({
  servers,
  accessMode,
  selectedServers,
  onChange,
  loading = false,
  disabled = false,
  className,
}: McpServerSelectorProps) {
  const includeAllServers = accessMode === "all"
  const enabledServerCount = servers.filter((s) => s.enabled).length
  const selectedCount = includeAllServers
    ? enabledServerCount
    : selectedServers.filter((id) =>
        servers.find((s) => s.id === id)?.enabled
      ).length

  // Check if indeterminate (some but not all selected)
  const isIndeterminate = !includeAllServers && selectedCount > 0 && selectedCount < enabledServerCount

  const isServerSelected = (serverId: string): boolean => {
    if (includeAllServers) return true
    return selectedServers.includes(serverId)
  }

  const handleAllServersToggle = () => {
    if (disabled) return

    if (includeAllServers) {
      // Switch to specific mode with current selections
      const mode = selectedServers.length > 0 ? "specific" : "none"
      onChange(mode, selectedServers)
    } else {
      // Enable all servers mode
      onChange("all", [])
    }
  }

  const handleServerToggle = (serverId: string) => {
    if (disabled) return

    // If includeAllServers is true, we need to demote to specific mode minus this server
    if (includeAllServers) {
      const otherServers = servers
        .filter(s => s.id !== serverId && s.enabled)
        .map(s => s.id)

      const mode = otherServers.length > 0 ? "specific" : "none"
      onChange(mode, otherServers)
      return
    }

    const newSelected = new Set(selectedServers)

    if (newSelected.has(serverId)) {
      newSelected.delete(serverId)
    } else {
      newSelected.add(serverId)
    }

    const selectedArray = Array.from(newSelected)
    const mode = selectedArray.length > 0 ? "specific" : "none"
    onChange(mode, selectedArray)
  }

  if (loading) {
    return (
      <div className={cn("border rounded-lg p-8 text-center text-muted-foreground text-sm", className)}>
        Loading servers...
      </div>
    )
  }

  if (servers.length === 0) {
    return (
      <div className={cn("border rounded-lg p-8 text-center text-muted-foreground text-sm", className)}>
        No MCP servers configured.
      </div>
    )
  }

  return (
    <div className={cn("border rounded-lg", className)}>
      <div className="max-h-[400px] overflow-y-auto">
        {/* All MCP Servers row */}
        <div
          className="flex items-center gap-3 px-4 py-3 border-b bg-background sticky top-0 z-10 cursor-pointer hover:bg-muted/50 transition-colors"
          onClick={handleAllServersToggle}
        >
          <Checkbox
            checked={includeAllServers || isIndeterminate}
            onCheckedChange={handleAllServersToggle}
            disabled={disabled}
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
          const canToggle = !disabled && !isDisabled

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
              {server.proxy_url && (
                <code className="text-xs text-muted-foreground truncate max-w-[200px]">
                  {server.proxy_url}
                </code>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
