import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import type { PermissionState, TreeNode, ModelPermissions, PermissionTreeProps } from "./types"

interface StrategyModel {
  provider: string
  model_id: string
  display_name: string
}

interface ModelsPermissionTreeProps extends PermissionTreeProps {
  permissions: ModelPermissions
  allowedModels: StrategyModel[]
}

export function ModelsPermissionTree({
  clientId,
  permissions,
  allowedModels,
  onUpdate,
}: ModelsPermissionTreeProps) {
  const [saving, setSaving] = useState(false)

  // Group models by provider
  const modelsByProvider = allowedModels.reduce(
    (acc, model) => {
      if (!acc[model.provider]) {
        acc[model.provider] = []
      }
      acc[model.provider].push(model)
      return acc
    },
    {} as Record<string, StrategyModel[]>
  )

  const handlePermissionChange = async (key: string, state: PermissionState) => {
    setSaving(true)
    try {
      // Parse the key to determine the level
      // Format: provider_name or provider_name__model_id
      const parts = key.split("__")

      if (parts.length === 1) {
        // Provider level
        await invoke("set_client_model_permission", {
          clientId,
          level: "provider",
          key,
          state,
        })
      } else {
        // Model level
        await invoke("set_client_model_permission", {
          clientId,
          level: "model",
          key,
          state,
        })
      }
      onUpdate()
    } catch (error) {
      console.error("Failed to set permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  const handleGlobalChange = async (state: PermissionState) => {
    setSaving(true)
    try {
      // First clear all child customizations so they inherit the new global value
      await invoke("clear_client_model_child_permissions", { clientId })
      // Then set the global permission
      await invoke("set_client_model_permission", {
        clientId,
        level: "global",
        key: null,
        state,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to set global permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  // Build tree nodes from models grouped by provider
  const buildTree = (): TreeNode[] => {
    return Object.entries(modelsByProvider).map(([provider, models]) => ({
      id: provider,
      label: provider,
      children: models.map((model) => ({
        id: `${provider}__${model.model_id}`,
        label: model.display_name || model.model_id,
      })),
    }))
  }

  // Build flat permissions map for the tree
  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}

    // Provider permissions
    if (permissions.providers) {
      for (const [provider, state] of Object.entries(permissions.providers)) {
        map[provider] = state
      }
    }

    // Model permissions
    if (permissions.models) {
      for (const [key, state] of Object.entries(permissions.models)) {
        map[key] = state
      }
    }

    return map
  }

  if (allowedModels.length === 0) {
    return (
      <div className="p-4 text-center text-muted-foreground text-sm">
        No models configured in the strategy's allowed list.
      </div>
    )
  }

  return (
    <PermissionTreeSelector
      nodes={buildTree()}
      permissions={buildPermissionsMap()}
      globalPermission={permissions.global}
      onPermissionChange={handlePermissionChange}
      onGlobalChange={handleGlobalChange}
      disabled={saving}
      globalLabel="All Models"
      emptyMessage="No models available"
    />
  )
}
