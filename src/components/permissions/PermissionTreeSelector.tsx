import { useState, useMemo, type ReactNode } from "react"
import { ChevronRight, ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"
import type { TreeNode } from "./types"

export interface TreeButtonProps<S extends string> {
  value: S
  onChange: (state: S) => void
  disabled?: boolean
  size?: "sm" | "md"
  inherited?: boolean
  childRollupStates?: Set<S>
}

export interface PermissionTreeSelectorProps<S extends string> {
  nodes: TreeNode[]
  permissions: Record<string, S>
  globalPermission: S
  /** Called when a node's permission changes. parentState is the inherited value from the parent. */
  onPermissionChange: (key: string, state: S, parentState: S) => void
  onGlobalChange: (state: S) => void
  /** Render the state button for each node */
  renderButton: (props: TreeButtonProps<S>) => ReactNode
  disabled?: boolean
  globalLabel?: string
  emptyMessage?: string
  loading?: boolean
  /** Start with all nodes expanded (default: false) */
  defaultExpanded?: boolean
}

export function PermissionTreeSelector<S extends string>({
  nodes,
  permissions,
  globalPermission,
  onPermissionChange,
  onGlobalChange,
  renderButton,
  disabled = false,
  globalLabel = "All",
  emptyMessage = "No items found",
  loading = false,
  defaultExpanded = false,
}: PermissionTreeSelectorProps<S>) {
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(() => {
    if (!defaultExpanded) return new Set<string>()
    // Collect all node IDs that have children
    const ids = new Set<string>()
    const collect = (nodes: TreeNode[]) => {
      for (const node of nodes) {
        if (node.children && node.children.length > 0) {
          ids.add(node.id)
          collect(node.children)
        }
      }
    }
    collect(nodes)
    return ids
  })

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

  const getEffectivePermission = (nodeId: string, parentPermission: S): S => {
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

  // Compute child rollup states for a node - collect all explicit permissions from descendants
  const getChildRollupStates = useMemo(() => {
    const cache = new Map<string, Set<S>>()

    const computeForNode = (node: TreeNode): Set<S> => {
      if (cache.has(node.id)) {
        return cache.get(node.id)!
      }

      const states = new Set<S>()

      // Check this node's explicit permission (only if it has children - we're computing for parent)
      if (node.children) {
        for (const child of node.children) {
          // Add child's explicit permission if set
          if (permissions[child.id] !== undefined) {
            states.add(permissions[child.id])
          }
          // Recursively get grandchildren's states
          const grandchildStates = computeForNode(child)
          grandchildStates.forEach(s => states.add(s))
        }
      }

      cache.set(node.id, states)
      return states
    }

    // Build the lookup function
    return (nodeId: string): Set<S> => {
      const node = findNode(nodes, nodeId)
      if (!node) return new Set()
      return computeForNode(node)
    }
  }, [nodes, permissions])

  // Helper to find a node by ID
  const findNode = (nodes: TreeNode[], id: string): TreeNode | undefined => {
    for (const node of nodes) {
      if (node.id === id) return node
      if (node.children) {
        const found = findNode(node.children, id)
        if (found) return found
      }
    }
    return undefined
  }

  // Compute global-level child rollup states
  const globalChildRollupStates = useMemo(() => {
    const states = new Set<S>()

    // Collect all explicit permissions at any level
    for (const node of nodes) {
      // Check node's explicit permission
      if (permissions[node.id] !== undefined) {
        states.add(permissions[node.id])
      }
      // Get descendants' states
      const childStates = getChildRollupStates(node.id)
      childStates.forEach(s => states.add(s))
    }

    return states
  }, [nodes, permissions, getChildRollupStates])

  const renderNode = (node: TreeNode, parentPermission: S, depth: number = 0) => {
    const isExpanded = expandedNodes.has(node.id)
    const hasChildren = node.children && node.children.length > 0
    const effectivePermission = getEffectivePermission(node.id, parentPermission)
    const inherited = isInherited(node.id)
    const canExpand = hasChildren
    const childRollupStates = hasChildren ? getChildRollupStates(node.id) : undefined

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
          {!node.isGroup && renderButton({
            value: effectivePermission,
            onChange: (state) => onPermissionChange(node.id, state, parentPermission),
            disabled,
            size: "sm",
            inherited,
            childRollupStates,
          })}
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
          {renderButton({
            value: globalPermission,
            onChange: onGlobalChange,
            disabled,
            size: "sm",
            childRollupStates: globalChildRollupStates,
          })}
        </div>

        {/* Tree nodes */}
        {nodes.map((node) => renderNode(node, globalPermission, 0))}
      </div>
    </div>
  )
}
