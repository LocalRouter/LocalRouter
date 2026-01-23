/**
 * AllowedModelsSelector Component
 *
 * Hierarchical checkbox selector for model permissions with three levels:
 * - All (selects everything including future models)
 * - Provider (selects all models from a provider)
 * - Individual Model
 *
 * Uses explicit fields for selection state:
 * - selected_all: boolean - when true, all models are allowed
 * - selected_providers: string[] - providers where all models are allowed
 * - selected_models: [string, string][] - individual model selections
 */

import { useMemo } from "react"
import { Checkbox } from "@/components/ui/checkbox"
import { cn } from "@/lib/utils"

export interface Model {
  id: string
  provider: string
}

export interface AllowedModelsSelection {
  selected_all: boolean
  selected_providers: string[]
  selected_models: [string, string][]
}

interface AllowedModelsSelectorProps {
  models: Model[]
  value: AllowedModelsSelection
  onChange: (selection: AllowedModelsSelection) => void
  disabled?: boolean
  className?: string
}

export function AllowedModelsSelector({
  models,
  value,
  onChange,
  disabled = false,
  className,
}: AllowedModelsSelectorProps) {
  // Normalize value with defaults to handle potentially missing fields
  const normalizedValue = useMemo((): AllowedModelsSelection => ({
    selected_all: value?.selected_all ?? true,
    selected_providers: value?.selected_providers || [],
    selected_models: value?.selected_models || [],
  }), [value])

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

  // Check if a model is allowed using the three explicit fields
  const isModelAllowed = (provider: string, modelId: string): boolean => {
    if (normalizedValue.selected_all) return true
    if (normalizedValue.selected_providers.some(p => p.toLowerCase() === provider.toLowerCase())) return true
    return normalizedValue.selected_models.some(([p, m]) =>
      p.toLowerCase() === provider.toLowerCase() && m.toLowerCase() === modelId.toLowerCase()
    )
  }

  // Check if a provider has all its models selected
  const isProviderSelected = (provider: string): boolean => {
    if (normalizedValue.selected_all) return true
    return normalizedValue.selected_providers.some(p => p.toLowerCase() === provider.toLowerCase())
  }

  // Check if provider is in "indeterminate" state (some but not all models selected)
  const isProviderIndeterminate = (provider: string): boolean => {
    if (normalizedValue.selected_all || isProviderSelected(provider)) return false
    const providerModels = groupedModels[provider] || []
    const selectedCount = providerModels.filter(m =>
      normalizedValue.selected_models.some(([p, id]) =>
        p.toLowerCase() === provider.toLowerCase() && id.toLowerCase() === m.id.toLowerCase()
      )
    ).length
    return selectedCount > 0 && selectedCount < providerModels.length
  }

  // Handle "All" checkbox toggle
  const handleAllToggle = () => {
    if (disabled) return

    if (normalizedValue.selected_all) {
      // Uncheck "All" - keep the current selections but turn off selected_all
      // If there were no explicit selections, start with none
      onChange({
        selected_all: false,
        selected_providers: normalizedValue.selected_providers,
        selected_models: normalizedValue.selected_models,
      })
    } else {
      // Check "All" - enable selected_all (keep existing selections for reference)
      onChange({
        selected_all: true,
        selected_providers: normalizedValue.selected_providers,
        selected_models: normalizedValue.selected_models,
      })
    }
  }

  // Handle provider checkbox toggle
  const handleProviderToggle = (provider: string) => {
    if (disabled) return

    // If selected_all is true, we need to turn it off and select everything except this provider
    if (normalizedValue.selected_all) {
      onChange({
        selected_all: false,
        selected_providers: providers.filter(p => p.toLowerCase() !== provider.toLowerCase()),
        selected_models: [], // No individual models needed when all other providers are selected
      })
      return
    }

    const providerLower = provider.toLowerCase()
    const isCurrentlySelected = normalizedValue.selected_providers.some(p => p.toLowerCase() === providerLower)

    if (isCurrentlySelected) {
      // Uncheck provider - remove from selected_providers
      onChange({
        selected_all: false,
        selected_providers: normalizedValue.selected_providers.filter(p => p.toLowerCase() !== providerLower),
        selected_models: normalizedValue.selected_models,
      })
    } else {
      // Check provider - add to selected_providers, remove individual models from this provider
      onChange({
        selected_all: false,
        selected_providers: [...normalizedValue.selected_providers, provider],
        selected_models: normalizedValue.selected_models.filter(([p]) => p.toLowerCase() !== providerLower),
      })
    }
  }

  // Handle individual model checkbox toggle
  const handleModelToggle = (provider: string, modelId: string) => {
    if (disabled) return

    const providerLower = provider.toLowerCase()
    const modelLower = modelId.toLowerCase()

    // If selected_all is true, turn it off and select everything except this model
    if (normalizedValue.selected_all) {
      const otherProviders = providers.filter(p => p.toLowerCase() !== providerLower)
      const providerModels = groupedModels[provider] || []
      const otherModels = providerModels
        .filter(m => m.id.toLowerCase() !== modelLower)
        .map(m => [provider, m.id] as [string, string])

      onChange({
        selected_all: false,
        selected_providers: otherProviders,
        selected_models: otherModels,
      })
      return
    }

    // If provider is selected, we need to "demote" it to individual models minus this one
    if (isProviderSelected(provider)) {
      const providerModels = groupedModels[provider] || []
      const otherModels = providerModels
        .filter(m => m.id.toLowerCase() !== modelLower)
        .map(m => [provider, m.id] as [string, string])

      onChange({
        selected_all: false,
        selected_providers: normalizedValue.selected_providers.filter(p => p.toLowerCase() !== providerLower),
        selected_models: [...normalizedValue.selected_models, ...otherModels],
      })
      return
    }

    // Toggle individual model
    const isCurrentlySelected = normalizedValue.selected_models.some(
      ([p, m]) => p.toLowerCase() === providerLower && m.toLowerCase() === modelLower
    )

    if (isCurrentlySelected) {
      // Uncheck model
      onChange({
        selected_all: false,
        selected_providers: normalizedValue.selected_providers,
        selected_models: normalizedValue.selected_models.filter(
          ([p, m]) => !(p.toLowerCase() === providerLower && m.toLowerCase() === modelLower)
        ),
      })
    } else {
      // Check model
      const newSelectedModels = [...normalizedValue.selected_models, [provider, modelId] as [string, string]]

      // Check if all models from this provider are now selected - promote to provider level
      const providerModels = groupedModels[provider] || []
      const selectedFromProvider = newSelectedModels.filter(
        ([p]) => p.toLowerCase() === providerLower
      ).length

      if (selectedFromProvider === providerModels.length) {
        // Promote to provider-level selection
        onChange({
          selected_all: false,
          selected_providers: [...normalizedValue.selected_providers, provider],
          selected_models: newSelectedModels.filter(([p]) => p.toLowerCase() !== providerLower),
        })
      } else {
        onChange({
          selected_all: false,
          selected_providers: normalizedValue.selected_providers,
          selected_models: newSelectedModels,
        })
      }
    }
  }

  // Count selected models for display
  const getSelectedCount = (): number => {
    if (normalizedValue.selected_all) return models.length

    let count = 0
    for (const provider of normalizedValue.selected_providers) {
      const providerModels = groupedModels[provider]
      if (providerModels) {
        count += providerModels.length
      }
    }
    count += normalizedValue.selected_models.length
    return count
  }

  return (
    <div className={cn("border rounded-lg", className)}>
      <div className="max-h-[400px] overflow-y-auto">
        {/* All row */}
        <div
          className="flex items-center gap-3 px-4 py-3 border-b bg-background sticky top-0 z-10 cursor-pointer hover:bg-muted/50 transition-colors"
          onClick={() => !disabled && handleAllToggle()}
        >
          <Checkbox
            checked={normalizedValue.selected_all}
            onCheckedChange={handleAllToggle}
            disabled={disabled}
            className="data-[state=checked]:bg-primary"
          />
          <span className="font-semibold text-sm">
            All Providers & Models
          </span>
          <span className="text-xs text-muted-foreground ml-auto">
            {normalizedValue.selected_all ? (
              <span className="text-primary">All (including future models)</span>
            ) : (
              `${getSelectedCount()} / ${models.length} selected`
            )}
          </span>
        </div>

        {/* Provider and model rows */}
        {providers.map((provider) => {
          const providerSelected = isProviderSelected(provider)
          const providerModels = groupedModels[provider]
          const indeterminate = isProviderIndeterminate(provider)

          return (
            <div key={provider}>
              {/* Provider row */}
              <div
                className={cn(
                  "flex items-center gap-3 px-4 py-2.5 border-b",
                  "hover:bg-muted/50 transition-colors cursor-pointer",
                  normalizedValue.selected_all && "opacity-60"
                )}
                style={{ paddingLeft: "2rem" }}
                onClick={() => !disabled && handleProviderToggle(provider)}
              >
                <Checkbox
                  checked={providerSelected || indeterminate}
                  onCheckedChange={() => handleProviderToggle(provider)}
                  disabled={disabled}
                  className={cn(
                    indeterminate && "data-[state=checked]:bg-primary/60"
                  )}
                />
                <span className="font-medium text-sm">{provider}</span>
                <span className="text-xs text-muted-foreground ml-auto">
                  {providerModels.length} models
                </span>
              </div>

              {/* Model rows */}
              {providerModels.map((model) => {
                const modelSelected = isModelAllowed(provider, model.id)
                // Model row is clickable when provider is not explicitly selected
                const canToggleModel = !disabled && !providerSelected && !normalizedValue.selected_all

                return (
                  <div
                    key={`${provider}/${model.id}`}
                    className={cn(
                      "flex items-center gap-3 px-4 py-2 border-b border-border/50",
                      "hover:bg-muted/30 transition-colors",
                      canToggleModel ? "cursor-pointer" : "opacity-50"
                    )}
                    style={{ paddingLeft: "3.5rem" }}
                    onClick={() => canToggleModel && handleModelToggle(provider, model.id)}
                  >
                    <Checkbox
                      checked={modelSelected}
                      onCheckedChange={() => handleModelToggle(provider, model.id)}
                      disabled={!canToggleModel}
                    />
                    <span className="text-sm text-muted-foreground font-mono">
                      {model.id}
                    </span>
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
