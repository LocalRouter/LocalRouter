/**
 * PrioritizedModelSelector Component
 *
 * Allows selecting and reordering models for prioritization.
 * Used for:
 * - Auto-routing prioritized models (strong models)
 * - Strong/Weak weak models
 *
 * Features:
 * - Select models from available list
 * - Reorder selected models with up/down buttons
 * - Handles models that are no longer available (shows warning)
 * - Expandable "Add Models" section grouped by provider
 */

import { useState, useMemo } from "react"
import { ChevronUp, ChevronDown, X, Plus, AlertCircle, GripVertical } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { cn } from "@/lib/utils"

export interface Model {
  id: string
  provider: string
}

interface PrioritizedModelSelectorProps {
  /**
   * All available models that can be selected
   */
  availableModels: Model[]
  /**
   * Currently selected and prioritized models as [provider, model_id] tuples
   */
  selectedModels: [string, string][]
  /**
   * Callback when selection changes
   */
  onChange: (models: [string, string][]) => void
  /**
   * Whether the component is disabled
   */
  disabled?: boolean
  /**
   * Optional title for the component
   */
  title?: string
  /**
   * Optional description
   */
  description?: string
  /**
   * Placeholder text when no models are selected
   */
  emptyText?: string
  /**
   * Additional CSS classes
   */
  className?: string
}

export function PrioritizedModelSelector({
  availableModels,
  selectedModels,
  onChange,
  disabled = false,
  title,
  description,
  emptyText = "No models selected. Add models below.",
  className,
}: PrioritizedModelSelectorProps) {
  const [addSectionOpen, setAddSectionOpen] = useState(false)

  // Group available models by provider
  const groupedAvailable = useMemo(() => {
    const groups: Record<string, Model[]> = {}
    for (const model of availableModels) {
      if (!groups[model.provider]) {
        groups[model.provider] = []
      }
      groups[model.provider].push(model)
    }
    return groups
  }, [availableModels])

  const providers = useMemo(
    () => Object.keys(groupedAvailable).sort(),
    [groupedAvailable]
  )

  // Check which selected models are still available
  const availableSet = useMemo(
    () => new Set(availableModels.map((m) => `${m.provider}/${m.id}`)),
    [availableModels]
  )

  // Check if a model is in the selected list
  const isModelSelected = (provider: string, modelId: string): boolean => {
    return selectedModels.some(([p, m]) => p === provider && m === modelId)
  }

  // Handle adding a model
  const handleAddModel = (provider: string, modelId: string) => {
    if (disabled || isModelSelected(provider, modelId)) return
    onChange([...selectedModels, [provider, modelId]])
  }

  // Handle removing a model
  const handleRemoveModel = (index: number) => {
    if (disabled) return
    const newModels = [...selectedModels]
    newModels.splice(index, 1)
    onChange(newModels)
  }

  // Handle moving a model up
  const handleMoveUp = (index: number) => {
    if (disabled || index === 0) return
    const newModels = [...selectedModels]
    ;[newModels[index - 1], newModels[index]] = [newModels[index], newModels[index - 1]]
    onChange(newModels)
  }

  // Handle moving a model down
  const handleMoveDown = (index: number) => {
    if (disabled || index === selectedModels.length - 1) return
    const newModels = [...selectedModels]
    ;[newModels[index], newModels[index + 1]] = [newModels[index + 1], newModels[index]]
    onChange(newModels)
  }

  // Count unavailable models
  const unavailableCount = selectedModels.filter(
    ([p, m]) => !availableSet.has(`${p}/${m}`)
  ).length

  return (
    <div className={cn("space-y-4", className)}>
      {/* Header */}
      {(title || description) && (
        <div>
          {title && <h4 className="font-medium text-sm">{title}</h4>}
          {description && (
            <p className="text-xs text-muted-foreground mt-1">{description}</p>
          )}
        </div>
      )}

      {/* Unavailable models warning */}
      {unavailableCount > 0 && (
        <div className="flex items-start gap-2 p-3 rounded-lg bg-amber-500/10 border border-amber-500/30 text-amber-600 dark:text-amber-400">
          <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
          <div className="text-xs">
            <p className="font-medium">
              {unavailableCount} model{unavailableCount > 1 ? "s" : ""} no longer available
            </p>
            <p className="text-muted-foreground mt-0.5">
              These models are configured but not currently offered by their providers.
              They will be skipped during selection.
            </p>
          </div>
        </div>
      )}

      {/* Selected models list */}
      <div className="border rounded-lg overflow-hidden">
        <div className="bg-muted/30 px-4 py-2 border-b">
          <span className="text-xs font-medium text-muted-foreground">
            Priority Order ({selectedModels.length} model{selectedModels.length !== 1 ? "s" : ""})
          </span>
        </div>

        <div className="divide-y">
          {selectedModels.length === 0 ? (
            <div className="p-4 text-center text-sm text-muted-foreground">
              {emptyText}
            </div>
          ) : (
            selectedModels.map(([provider, modelId], index) => {
              const isAvailable = availableSet.has(`${provider}/${modelId}`)

              return (
                <div
                  key={`${provider}/${modelId}/${index}`}
                  className={cn(
                    "flex items-center gap-2 px-3 py-2 group",
                    "hover:bg-muted/50 transition-colors",
                    !isAvailable && "bg-amber-500/5"
                  )}
                >
                  {/* Drag handle / index */}
                  <div className="flex items-center gap-1 text-muted-foreground">
                    <GripVertical className="h-4 w-4 opacity-0 group-hover:opacity-50" />
                    <span className="text-xs font-mono w-5 text-right">
                      {index + 1}.
                    </span>
                  </div>

                  {/* Model info */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          "text-sm font-mono truncate",
                          !isAvailable && "text-muted-foreground line-through"
                        )}
                      >
                        {modelId}
                      </span>
                      {!isAvailable && (
                        <span className="text-xs text-amber-600 dark:text-amber-400 shrink-0">
                          (unavailable)
                        </span>
                      )}
                    </div>
                    <span className="text-xs text-muted-foreground">
                      {provider}
                    </span>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7"
                      onClick={() => handleMoveUp(index)}
                      disabled={disabled || index === 0}
                    >
                      <ChevronUp className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7"
                      onClick={() => handleMoveDown(index)}
                      disabled={disabled || index === selectedModels.length - 1}
                    >
                      <ChevronDown className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7 text-destructive hover:text-destructive hover:bg-destructive/10"
                      onClick={() => handleRemoveModel(index)}
                      disabled={disabled}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              )
            })
          )}
        </div>
      </div>

      {/* Add models section */}
      <Collapsible open={addSectionOpen} onOpenChange={setAddSectionOpen}>
        <CollapsibleTrigger asChild>
          <Button
            variant="outline"
            className="w-full justify-between"
            disabled={disabled}
          >
            <span className="flex items-center gap-2">
              <Plus className="h-4 w-4" />
              Add Models
            </span>
            <ChevronDown
              className={cn(
                "h-4 w-4 transition-transform",
                addSectionOpen && "rotate-180"
              )}
            />
          </Button>
        </CollapsibleTrigger>

        <CollapsibleContent className="mt-2">
          <div className="border rounded-lg max-h-[300px] overflow-y-auto">
            {providers.length === 0 ? (
              <div className="p-4 text-center text-sm text-muted-foreground">
                No models available
              </div>
            ) : (
              providers.map((provider) => {
                const providerModels = groupedAvailable[provider]
                const unselectedModels = providerModels.filter(
                  (m) => !isModelSelected(provider, m.id)
                )

                if (unselectedModels.length === 0) return null

                return (
                  <div key={provider}>
                    {/* Provider header */}
                    <div className="px-4 py-2 bg-muted/30 border-b sticky top-0">
                      <span className="text-xs font-medium">{provider}</span>
                      <span className="text-xs text-muted-foreground ml-2">
                        ({unselectedModels.length} available)
                      </span>
                    </div>

                    {/* Models */}
                    {unselectedModels.map((model) => (
                      <div
                        key={`${provider}/${model.id}`}
                        className="flex items-center gap-3 px-4 py-2 hover:bg-muted/30 transition-colors cursor-pointer border-b border-border/50"
                        onClick={() => handleAddModel(provider, model.id)}
                      >
                        <Checkbox
                          checked={false}
                          disabled={disabled}
                          className="pointer-events-none"
                        />
                        <span className="text-sm font-mono text-muted-foreground">
                          {model.id}
                        </span>
                      </div>
                    ))}
                  </div>
                )
              })
            )}

            {/* All models selected message */}
            {providers.every(
              (p) =>
                groupedAvailable[p].filter((m) => !isModelSelected(p, m.id))
                  .length === 0
            ) && (
              <div className="p-4 text-center text-sm text-muted-foreground">
                All available models have been selected
              </div>
            )}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}
