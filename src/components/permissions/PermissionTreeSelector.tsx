import { useState } from "react"
import { ChevronRight, ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"
import { PermissionStateButton } from "./PermissionStateButton"
import type { PermissionState, TreeNode } from "./types"

interface PermissionTreeSelectorProps {
  nodes: TreeNode[]
  permissions: Record<string, PermissionState>
  globalPermission: PermissionState
  /** Called when a node's permission changes. parentState is the inherited value from the parent. */
  onPermissionChange: (key: string, state: PermissionState, parentState: PermissionState) => void
  onGlobalChange: (state: PermissionState) => void
  disabled?: boolean
  globalLabel?: string
  emptyMessage?: string
  loading?: boolean
}

export function PermissionTreeSelector({
  nodes,
  permissions,
  globalPermission,
  onPermissionChange,
  onGlobalChange,
  disabled = false,
  globalLabel = "All",
  emptyMessage = "No items found",
  loading = false,
}: PermissionTreeSelectorProps) {
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())

  const toggleNode = (nodeId: string) => {
    setExpandedNodes((prev) => {
      const next = new Set(prev)
      if (next.has(nodeId)) {
        next.delete(nodeId)
      } else {
        next.add(nodeId)
      }
      return next
    })
  }

  const getEffectivePermission = (nodeId: string, parentPermission: PermissionState): PermissionState => {
    // Check if there's an explicit override
    if (permissions[nodeId] !== undefined) {
      return permissions[nodeId]
    }
    // Otherwise inherit from parent
    return parentPermission
  }

  const isInherited = (nodeId: string): boolean => {
    return permissions[nodeId] === undefined
  }

  const renderNode = (node: TreeNode, parentPermission: PermissionState, depth: number = 0) => {
    const isExpanded = expandedNodes.has(node.id)
    const hasChildren = node.children && node.children.length > 0
    const effectivePermission = getEffectivePermission(node.id, parentPermission)
    const inherited = isInherited(node.id)
    const canExpand = hasChildren && effectivePermission !== "off"

    // For group nodes (Tools/Resources/Prompts), make the whole row clickable
    const handleRowClick = () => {
      if (node.isGroup && canExpand) {
        toggleNode(node.id)
      }
    }

    return (
      <div key={node.id}>
        <div
          className={cn(
            "flex items-center gap-2 py-2 border-b border-border/50",
            "hover:bg-muted/30 transition-colors",
            depth > 0 && "text-sm",
            node.isGroup && canExpand && "cursor-pointer"
          )}
          style={{ paddingLeft: `${12 + depth * 16}px`, paddingRight: "12px" }}
          onClick={handleRowClick}
        >
          {/* Expand/collapse button */}
          {hasChildren ? (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                canExpand && toggleNode(node.id)
              }}
              className={cn(
                "p-0.5 rounded hover:bg-muted",
                !canExpand && "opacity-30 cursor-not-allowed"
              )}
              disabled={!canExpand}
            >
              {isExpanded ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
            </button>
          ) : (
            <div className="w-5" /> // Spacer for alignment
          )}

          {/* Label */}
          <div className="flex-1 min-w-0">
            <span className={cn("font-medium", inherited && "text-muted-foreground")}>
              {node.label}
            </span>
            {node.description && (
              <p className="text-xs text-muted-foreground break-words">{node.description}</p>
            )}
          </div>

          {/* Permission selector - not shown for group nodes (Tools/Resources/Prompts headers) */}
          {!node.isGroup && (
            <PermissionStateButton
              value={effectivePermission}
              onChange={(state) => onPermissionChange(node.id, state, parentPermission)}
              disabled={disabled}
              size="sm"
              inherited={inherited}
            />
          )}
        </div>

        {/* Children */}
        {isExpanded && hasChildren && (
          <div>
            {node.children!.map((child) =>
              renderNode(child, effectivePermission, depth + 1)
            )}
          </div>
        )}
      </div>
    )
  }

  if (loading) {
    return (
      <div className="p-8 text-center text-muted-foreground text-sm">
        Loading...
      </div>
    )
  }

  if (nodes.length === 0) {
    return (
      <div className="p-8 text-center text-muted-foreground text-sm">
        {emptyMessage}
      </div>
    )
  }

  return (
    <div className="border rounded-lg">
      <div className="max-h-[500px] overflow-y-auto">
        {/* Global row - sticky header */}
        <div
          className="flex items-center gap-2 px-3 py-3 border-b bg-background sticky top-0 z-10"
        >
          <span className="font-semibold text-sm flex-1">{globalLabel}</span>
          <PermissionStateButton
            value={globalPermission}
            onChange={onGlobalChange}
            disabled={disabled}
            size="sm"
          />
        </div>

        {/* Tree nodes */}
        {nodes.map((node) => renderNode(node, globalPermission, 0))}
      </div>
    </div>
  )
}
