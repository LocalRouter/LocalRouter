import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { IndexingStateButton, IndexingStateButtonWithDefault } from "./IndexingStateButton"
import type { TreeNode } from "./types"
import type {
  IndexingState,
  ClientToolsIndexingPermissions,
  KnownToolEntry,
} from "@/types/tauri-commands"

interface ClientToolsIndexingTreeProps {
  clientId: string
  templateId: string | null
  globalDefault: IndexingState
  onUpdate: () => void
}

export function ClientToolsIndexingTree({
  clientId,
  templateId,
  globalDefault,
  onUpdate,
}: ClientToolsIndexingTreeProps) {
  const [permissions, setPermissions] = useState<ClientToolsIndexingPermissions | null>(null)
  const [knownTools, setKnownTools] = useState<KnownToolEntry[]>([])
  const [seenTools, setSeenTools] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadData = useCallback(async () => {
    try {
      const [perms, known, seen] = await Promise.all([
        invoke<ClientToolsIndexingPermissions | null>("get_client_tools_indexing", {
          clientId,
        }),
        templateId
          ? invoke<KnownToolEntry[]>("get_known_client_tools", {
              templateId,
            })
          : Promise.resolve([]),
        invoke<string[]>("get_seen_client_tools", {
          clientId,
        }),
      ])
      setPermissions(perms)
      setKnownTools(known)
      setSeenTools(seen)
    } catch (error) {
      console.error("Failed to load client tools indexing:", error)
    } finally {
      setLoading(false)
    }
  }, [clientId, templateId])

  useEffect(() => {
    loadData()
  }, [loadData])

  // Merge known tools with seen tools (discovered at runtime)
  const knownNames = new Set(knownTools.map((t) => t.name))
  const allToolNames = [
    ...knownTools.map((t) => t.name),
    ...seenTools.filter((name) => !knownNames.has(name)),
  ]

  const knownToolMap = new Map(knownTools.map((t) => [t.name, t]))

  // Build tree nodes (flat list of tools)
  const nodes: TreeNode[] = allToolNames.map((name) => {
    const known = knownToolMap.get(name)
    const isIndexable = known ? known.indexable : true
    return {
      id: name,
      label: name,
      description: !isIndexable ? "Action tool — not indexable" : undefined,
      disabled: !isIndexable,
    }
  })

  // Build flat permissions from the per-client overrides
  const flatPermissions: Record<string, IndexingState> = {}
  if (permissions) {
    for (const [key, value] of Object.entries(permissions.tools ?? {})) {
      flatPermissions[key] = value
    }
  }

  // Effective global: client override → config default
  const effectiveGlobal: IndexingState = permissions?.global ?? globalDefault

  const handlePermissionChange = async (key: string, state: IndexingState, parentState: IndexingState) => {
    setSaving(true)
    try {
      const shouldClear = state === parentState
      const params = shouldClear
        ? { clientId, level: "tool_clear", key, state: null }
        : { clientId, level: "tool", key, state }

      await invoke("set_client_tools_indexing", params )
      await loadData()
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const handleGlobalChange = async (state: IndexingState | null) => {
    setSaving(true)
    try {
      const params = state === null
        ? { clientId, level: "global_clear" }
        : { clientId, level: "global", state }

      await invoke("set_client_tools_indexing", params )
      await loadData()
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-2">
      <PermissionTreeSelector<IndexingState>
        nodes={nodes}
        permissions={flatPermissions}
        globalPermission={effectiveGlobal}
        onPermissionChange={handlePermissionChange}
        onGlobalChange={handleGlobalChange}
        renderButton={(props) => <IndexingStateButton {...props} />}
        renderGlobalButton={(childRollupStates) => (
          <IndexingStateButtonWithDefault
            value={permissions?.global ?? null}
            globalDefault={globalDefault}
            onChange={handleGlobalChange}
            disabled={saving}
            childRollupStates={childRollupStates}
          />
        )}
        disabled={saving}
        globalLabel="All Client Tools"
        emptyMessage="Specific client tools will appear here when client connects and reports its available tools."
        loading={loading}
        defaultExpanded
      />
      {allToolNames.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Tools are auto-discovered when the client connects.
        </p>
      )}
    </div>
  )
}
