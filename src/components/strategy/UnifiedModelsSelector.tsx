/**
 * UnifiedModelsSelector Component
 *
 * Combines model selection with firewall permissions in a single unified view.
 * Uses Allow/Ask/Off states instead of checkboxes:
 * - Allow = model is selected in strategy AND client can use without approval
 * - Ask = model is selected in strategy AND client needs approval before use
 * - Off = model is not selected/not accessible
 *
 * Updates both:
 * - Strategy's allowed_models (determines what models appear in /v1/models)
 * - Client's model_permissions (determines firewall behavior)
 */

import { useMemo, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ChevronRight, ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"
import { PermissionStateButton } from "@/components/permissions/PermissionStateButton"
import type { PermissionState, ModelPermissions } from "@/components/permissions"
import { useState } from "react"

export interface Model {
  id: string
  provider: string
}

export interface AllowedModelsSelection {
  selected_all: boolean
  selected_providers: string[]
  selected_models: [string, string][]
}

interface UnifiedModelsSelectorProps {
  /** All available models */
  models: Model[]
  /** Current strategy's allowed models selection */
  strategySelection: AllowedModelsSelection
  /** Client ID for permission updates */
  clientId: string
  /** Client's current model permissions */
  clientPermissions: ModelPermissions
  /** Callback when strategy selection changes */
  onStrategyChange: (selection: AllowedModelsSelection) => void
  /** Callback after client permissions update */
  onClientUpdate: () => void
  disabled?: boolean
  className?: string
}

export function UnifiedModelsSelector({
  models,
  strategySelection,
  clientId,
  clientPermissions,
  onStrategyChange,
  onClientUpdate,
  disabled = false,
  className,
}: UnifiedModelsSelectorProps) {
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set())
  const [saving, setSaving] = useState(false)

  // Normalize selections with defaults
  const normalizedSelection = useMemo((): AllowedModelsSelection => ({
    selected_all: strategySelection?.selected_all ?? true,
    selected_providers: strategySelection?.selected_providers || [],
    selected_models: strategySelection?.selected_models || [],
  }), [strategySelection])

  // Group models by provider
  const groupedModels = useMemo(() => {
    const groups: Record<string, Model[]> = {}
    for (const model of models) {
      if (!groups[model.provider]) {
        groups[model.provider] = []
      }
      groups[model.provider].push(model)
    }
    return groups
  }, [models])

  const providers = useMemo(() => Object.keys(groupedModels).sort(), [groupedModels])

  const toggleProvider = (provider: string) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev)
      if (next.has(provider)) {
        next.delete(provider)
      } else {
        next.add(provider)
      }
      return next
    })
  }

  // Check if a model is in the strategy's allowed list
  const isModelInStrategy = useCallback((provider: string, modelId: string): boolean => {
    if (normalizedSelection.selected_all) return true
    if (normalizedSelection.selected_providers.some(p => p.toLowerCase() === provider.toLowerCase())) return true
    return normalizedSelection.selected_models.some(([p, m]) =>
      p.toLowerCase() === provider.toLowerCase() && m.toLowerCase() === modelId.toLowerCase()
    )
  }, [normalizedSelection])

  // Check if provider has all models in strategy
  const isProviderInStrategy = useCallback((provider: string): boolean => {
    if (normalizedSelection.selected_all) return true
    return normalizedSelection.selected_providers.some(p => p.toLowerCase() === provider.toLowerCase())
  }, [normalizedSelection])

  // Get client permission for a model (inherited from provider/global if not set)
  const getModelPermission = useCallback((provider: string, modelId: string): PermissionState => {
    const modelKey = `${provider}__${modelId}`

    // Check model-level override
    if (clientPermissions?.models?.[modelKey]) {
      return clientPermissions.models[modelKey]
    }

    // Check provider-level
    if (clientPermissions?.providers?.[provider]) {
      return clientPermissions.providers[provider]
    }

    // Fall back to global
    return clientPermissions?.global || "allow"
  }, [clientPermissions])

  // Get effective permission for display (considering strategy + client)
  const getEffectivePermission = useCallback((provider: string, modelId: string): PermissionState => {
    const inStrategy = isModelInStrategy(provider, modelId)
    if (!inStrategy) return "off"
    return getModelPermission(provider, modelId)
  }, [isModelInStrategy, getModelPermission])

  // Get provider-level effective permission
  const getProviderEffectivePermission = useCallback((provider: string): PermissionState => {
    const inStrategy = isProviderInStrategy(provider)
    if (!inStrategy) {
      // Check if any models from this provider are in the strategy
      const providerModels = groupedModels[provider] || []
      const anyInStrategy = providerModels.some(m => isModelInStrategy(provider, m.id))
      if (!anyInStrategy) return "off"

      // Mixed state - check individual model permissions
      const permissions = providerModels
        .filter(m => isModelInStrategy(provider, m.id))
        .map(m => getModelPermission(provider, m.id))

      if (permissions.every(p => p === "off")) return "off"
      if (permissions.every(p => p === "allow")) return "allow"
      if (permissions.every(p => p === "ask")) return "ask"
      return "ask" // Mixed permissions default to ask
    }

    // Provider is in strategy, check permission
    if (clientPermissions?.providers?.[provider]) {
      return clientPermissions.providers[provider]
    }
    return clientPermissions?.global || "allow"
  }, [isProviderInStrategy, isModelInStrategy, groupedModels, getModelPermission, clientPermissions])

  // Get global effective permission
  const getGlobalEffectivePermission = useCallback((): PermissionState => {
    if (normalizedSelection.selected_all) {
      return clientPermissions?.global || "allow"
    }
    // Not all selected - check if everything is off
    const hasAnySelected = normalizedSelection.selected_providers.length > 0 ||
      normalizedSelection.selected_models.length > 0
    if (!hasAnySelected) return "off"
    return clientPermissions?.global || "allow"
  }, [normalizedSelection, clientPermissions])

  // Update client permission via backend
  const updateClientPermission = useCallback(async (
    level: "global" | "provider" | "model",
    key: string | null,
    state: PermissionState
  ) => {
    setSaving(true)
    try {
      await invoke("set_client_model_permission", {
        clientId,
        level,
        key,
        state,
      })
      onClientUpdate()
    } catch (error) {
      console.error("Failed to set permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }, [clientId, onClientUpdate])

  // Handle global permission change
  const handleGlobalChange = useCallback(async (state: PermissionState) => {
    if (disabled || saving) return

    setSaving(true)
    try {
      // First clear all child customizations so they inherit the new global value
      await invoke("clear_client_model_child_permissions", { clientId })

      if (state === "off") {
        // Turn off all - update strategy to select nothing
        onStrategyChange({
          selected_all: false,
          selected_providers: [],
          selected_models: [],
        })
      } else {
        // Turn on all - update strategy to select all
        onStrategyChange({
          selected_all: true,
          selected_providers: normalizedSelection.selected_providers,
          selected_models: normalizedSelection.selected_models,
        })
      }

      // Set the global permission
      await invoke("set_client_model_permission", {
        clientId,
        level: "global",
        key: null,
        state,
      })
      onClientUpdate()
    } catch (error) {
      console.error("Failed to set global permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }, [disabled, saving, clientId, normalizedSelection, onStrategyChange, onClientUpdate])

  // Handle provider permission change
  const handleProviderChange = useCallback(async (provider: string, state: PermissionState) => {
    if (disabled || saving) return

    const providerLower = provider.toLowerCase()

    if (state === "off") {
      // Remove provider from strategy
      if (normalizedSelection.selected_all) {
        // Was "all", now select all providers except this one
        onStrategyChange({
          selected_all: false,
          selected_providers: providers.filter(p => p.toLowerCase() !== providerLower),
          selected_models: [],
        })
      } else {
        // Remove from selected providers and models
        onStrategyChange({
          selected_all: false,
          selected_providers: normalizedSelection.selected_providers.filter(
            p => p.toLowerCase() !== providerLower
          ),
          selected_models: normalizedSelection.selected_models.filter(
            ([p]) => p.toLowerCase() !== providerLower
          ),
        })
      }
      await updateClientPermission("provider", provider, "off")
    } else {
      // Add provider to strategy
      if (!isProviderInStrategy(provider)) {
        onStrategyChange({
          selected_all: false,
          selected_providers: [...normalizedSelection.selected_providers, provider],
          selected_models: normalizedSelection.selected_models.filter(
            ([p]) => p.toLowerCase() !== providerLower
          ),
        })
      }
      await updateClientPermission("provider", provider, state)
    }
  }, [disabled, saving, normalizedSelection, providers, isProviderInStrategy, onStrategyChange, updateClientPermission])

  // Handle individual model permission change
  const handleModelChange = useCallback(async (provider: string, modelId: string, state: PermissionState) => {
    if (disabled || saving) return

    const providerLower = provider.toLowerCase()
    const modelLower = modelId.toLowerCase()
    const modelKey = `${provider}__${modelId}`

    if (state === "off") {
      // Remove model from strategy
      if (normalizedSelection.selected_all) {
        // Was "all", select all except this model
        const otherProviders = providers.filter(p => p.toLowerCase() !== providerLower)
        const providerModels = groupedModels[provider] || []
        const otherModels = providerModels
          .filter(m => m.id.toLowerCase() !== modelLower)
          .map(m => [provider, m.id] as [string, string])

        onStrategyChange({
          selected_all: false,
          selected_providers: otherProviders,
          selected_models: otherModels,
        })
      } else if (isProviderInStrategy(provider)) {
        // Provider was selected, demote to individual models minus this one
        const providerModels = groupedModels[provider] || []
        const otherModels = providerModels
          .filter(m => m.id.toLowerCase() !== modelLower)
          .map(m => [provider, m.id] as [string, string])

        onStrategyChange({
          selected_all: false,
          selected_providers: normalizedSelection.selected_providers.filter(
            p => p.toLowerCase() !== providerLower
          ),
          selected_models: [
            ...normalizedSelection.selected_models.filter(([p]) => p.toLowerCase() !== providerLower),
            ...otherModels,
          ],
        })
      } else {
        // Just remove from selected models
        onStrategyChange({
          selected_all: false,
          selected_providers: normalizedSelection.selected_providers,
          selected_models: normalizedSelection.selected_models.filter(
            ([p, m]) => !(p.toLowerCase() === providerLower && m.toLowerCase() === modelLower)
          ),
        })
      }
      await updateClientPermission("model", modelKey, "off")
    } else {
      // Add model to strategy
      if (!isModelInStrategy(provider, modelId)) {
        const newSelectedModels = [
          ...normalizedSelection.selected_models,
          [provider, modelId] as [string, string],
        ]

        // Check if all models from this provider are now selected - promote to provider level
        const providerModels = groupedModels[provider] || []
        const selectedFromProvider = newSelectedModels.filter(
          ([p]) => p.toLowerCase() === providerLower
        ).length

        if (selectedFromProvider === providerModels.length) {
          // Promote to provider-level selection
          onStrategyChange({
            selected_all: false,
            selected_providers: [...normalizedSelection.selected_providers, provider],
            selected_models: newSelectedModels.filter(([p]) => p.toLowerCase() !== providerLower),
          })
        } else {
          onStrategyChange({
            selected_all: false,
            selected_providers: normalizedSelection.selected_providers,
            selected_models: newSelectedModels,
          })
        }
      }
      await updateClientPermission("model", modelKey, state)
    }
  }, [
    disabled, saving, normalizedSelection, providers, groupedModels,
    isProviderInStrategy, isModelInStrategy, onStrategyChange, updateClientPermission
  ])

  // Check if permission is inherited
  const isModelInherited = useCallback((provider: string, modelId: string): boolean => {
    const modelKey = `${provider}__${modelId}`
    return !clientPermissions?.models?.[modelKey]
  }, [clientPermissions])

  const isProviderInherited = useCallback((provider: string): boolean => {
    return !clientPermissions?.providers?.[provider]
  }, [clientPermissions])

  // Count selected models
  const getSelectedCount = (): number => {
    if (normalizedSelection.selected_all) return models.length

    let count = 0
    for (const provider of normalizedSelection.selected_providers) {
      const providerModels = groupedModels[provider]
      if (providerModels) {
        count += providerModels.length
      }
    }
    count += normalizedSelection.selected_models.length
    return count
  }

  return (
    <div className={cn("border rounded-lg", className)}>
      <div className="max-h-[500px] overflow-y-auto">
        {/* Global row - sticky header */}
        <div className="flex items-center gap-2 px-3 py-3 border-b bg-background sticky top-0 z-10">
          <div className="w-5" /> {/* Spacer for alignment */}
          <span className="font-semibold text-sm flex-1">All Providers & Models</span>
          <span className="text-xs text-muted-foreground mr-2">
            {normalizedSelection.selected_all ? (
              <span className="text-primary">All (including future models)</span>
            ) : (
              `${getSelectedCount()} / ${models.length} selected`
            )}
          </span>
          <PermissionStateButton
            value={getGlobalEffectivePermission()}
            onChange={handleGlobalChange}
            disabled={disabled || saving}
            size="sm"
          />
        </div>

        {/* Provider and model rows */}
        {providers.map((provider) => {
          const providerModels = groupedModels[provider]
          const isExpanded = expandedProviders.has(provider)
          const effectivePermission = getProviderEffectivePermission(provider)

          return (
            <div key={provider}>
              {/* Provider row */}
              <div
                className={cn(
                  "flex items-center gap-2 py-2.5 border-b",
                  "hover:bg-muted/30 transition-colors"
                )}
                style={{ paddingLeft: "12px", paddingRight: "12px" }}
              >
                {/* Expand/collapse button */}
                <button
                  type="button"
                  onClick={() => toggleProvider(provider)}
                  className="p-0.5 rounded hover:bg-muted"
                >
                  {isExpanded ? (
                    <ChevronDown className="h-4 w-4 text-muted-foreground" />
                  ) : (
                    <ChevronRight className="h-4 w-4 text-muted-foreground" />
                  )}
                </button>

                <span className={cn(
                  "font-medium text-sm flex-1",
                  isProviderInherited(provider) && effectivePermission !== "off" && "text-muted-foreground"
                )}>
                  {provider}
                </span>
                <span className="text-xs text-muted-foreground mr-2">
                  {providerModels.length} models
                </span>
                <PermissionStateButton
                  value={effectivePermission}
                  onChange={(state) => handleProviderChange(provider, state)}
                  disabled={disabled || saving}
                  size="sm"
                  inherited={isProviderInherited(provider) && effectivePermission !== "off"}
                />
              </div>

              {/* Model rows */}
              {isExpanded && providerModels.map((model) => {
                const modelPermission = getEffectivePermission(provider, model.id)
                const inherited = isModelInherited(provider, model.id)

                return (
                  <div
                    key={`${provider}/${model.id}`}
                    className={cn(
                      "flex items-center gap-2 py-2 border-b border-border/50",
                      "hover:bg-muted/30 transition-colors text-sm"
                    )}
                    style={{ paddingLeft: "28px", paddingRight: "12px" }}
                  >
                    <div className="w-5" /> {/* Spacer for alignment */}
                    <span className={cn(
                      "text-muted-foreground font-mono flex-1 truncate",
                      inherited && modelPermission !== "off" && "opacity-70"
                    )}>
                      {model.id}
                    </span>
                    <PermissionStateButton
                      value={modelPermission}
                      onChange={(state) => handleModelChange(provider, model.id, state)}
                      disabled={disabled || saving}
                      size="sm"
                      inherited={inherited && modelPermission !== "off"}
                    />
                  </div>
                )
              })}
            </div>
          )
        })}

        {providers.length === 0 && (
          <div className="p-8 text-center text-muted-foreground text-sm">
            No models available
          </div>
        )}
      </div>
    </div>
  )
}
