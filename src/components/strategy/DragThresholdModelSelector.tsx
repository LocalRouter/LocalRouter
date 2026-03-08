/**
 * DragThresholdModelSelector - Experimental Component
 *
 * A table-based model selector where:
 * - All models are shown in a single draggable table
 * - A threshold divider separates enabled (above) from disabled (below)
 * - Drag models to reorder or move across the threshold
 * - Models above the threshold are prioritized in order
 */

import { useState, useMemo } from "react"
import {
  DndContext,
  DragOverlay,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  DragStartEvent,
  DragEndEvent,
  DragOverEvent,
  useDroppable,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { GripVertical, Zap, Ban, Search, ChevronRight, ChevronDown, ArrowUpDown } from "lucide-react"
import { cn } from "@/lib/utils"
import { ModelPricingBadge } from "@/components/shared/model-pricing-badge"
import type { FreeTierKind } from "@/types/tauri-commands"

export interface Model {
  id: string
  provider: string
}

type SortOption = 'name' | 'provider' | 'price-asc' | 'price-desc' | 'params-asc' | 'params-desc'

const SORT_OPTIONS: { value: SortOption; label: string }[] = [
  { value: 'name', label: 'Name' },
  { value: 'provider', label: 'Provider' },
  { value: 'price-asc', label: 'Price: Low → High' },
  { value: 'price-desc', label: 'Price: High → Low' },
  { value: 'params-asc', label: 'Params: Small → Large' },
  { value: 'params-desc', label: 'Params: Large → Small' },
]

/** Parse formatted parameter count string (e.g. "7.0B", "13.5M") to a numeric value for sorting */
const parseParamCount = (s: string): number => {
  const match = s.match(/^([\d.]+)\s*([BMK]?)$/i)
  if (!match) return 0
  const num = parseFloat(match[1])
  switch (match[2].toUpperCase()) {
    case 'B': return num * 1e9
    case 'M': return num * 1e6
    case 'K': return num * 1e3
    default: return num
  }
}

export interface ModelPricingInfo {
  input?: number | null
  output?: number | null
}

interface DragThresholdModelSelectorProps {
  availableModels: Model[]
  enabledModels: [string, string][]
  onChange: (models: [string, string][]) => void
  disabled?: boolean
  title?: string
  description?: string
  className?: string
  /** Disable DragOverlay - useful in modals/dialogs where transforms cause offset issues */
  disableDragOverlay?: boolean
  /** Optional pricing data keyed by "provider/modelId" */
  modelPricing?: Record<string, ModelPricingInfo>
  /** Optional parameter count strings keyed by "provider/modelId" (e.g. "7.0B") */
  modelParamCounts?: Record<string, string>
  /** Optional free tier kinds keyed by provider instance name */
  freeTierKinds?: Record<string, FreeTierKind>
}

// Unique ID for each model
const getModelKey = (provider: string, modelId: string) => `${provider}::${modelId}`
const parseModelKey = (key: string): [string, string] => {
  const [provider, modelId] = key.split("::")
  return [provider, modelId]
}

// Sortable row component
function SortableRow({
  id,
  provider,
  modelId,
  index,
  isEnabled,
  disabled,
  onToggle,
  pricing,
  freeTierKind,
  showProvider = true,
}: {
  id: string
  provider: string
  modelId: string
  index: number
  isEnabled: boolean
  disabled: boolean
  onToggle: () => void
  pricing?: ModelPricingInfo
  freeTierKind?: FreeTierKind
  showProvider?: boolean
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id, disabled })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
  }

  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      onClick={(e) => {
        // Only toggle if not dragging (click without movement)
        if (!isDragging) {
          e.stopPropagation()
          onToggle()
        }
      }}
      className={cn(
        "flex items-center gap-3 px-3 py-2 border-b border-border/50 transition-colors",
        "cursor-grab active:cursor-grabbing touch-none select-none",
        isEnabled
          ? "bg-background hover:bg-muted/30"
          : "bg-muted/20 hover:bg-muted/40 text-muted-foreground",
        isDragging && "opacity-50 bg-primary/10 z-50",
        disabled && "opacity-60 cursor-default"
      )}
    >
      {/* Drag handle icon */}
      <GripVertical
        className={cn(
          "h-4 w-4 shrink-0 transition-colors",
          isEnabled ? "text-muted-foreground/50" : "text-muted-foreground/30"
        )}
      />

      {/* Priority number for enabled models */}
      <div className="w-6 text-center">
        {isEnabled ? (
          <span className="text-xs font-mono font-medium text-primary">{index + 1}</span>
        ) : (
          <Ban className="h-3.5 w-3.5 text-muted-foreground/50 mx-auto" />
        )}
      </div>

      {/* Model info + provider badge inline */}
      <div className="flex-1 min-w-0 flex items-center gap-2">
        <span
          className={cn(
            "text-sm font-mono truncate",
            !isEnabled && "text-muted-foreground"
          )}
        >
          {modelId}
        </span>
        {showProvider && (
          <span
            className={cn(
              "text-xs px-2 py-0.5 rounded-full shrink-0",
              isEnabled
                ? "bg-muted text-muted-foreground"
                : "bg-muted/50 text-muted-foreground/70"
            )}
          >
            {provider}
          </span>
        )}
      </div>

      {/* Pricing badge */}
      {pricing && (
        <ModelPricingBadge
          inputPricePerMillion={pricing.input}
          outputPricePerMillion={pricing.output}
          freeTierKind={freeTierKind}
        />
      )}
    </div>
  )
}

// Drag overlay item (what shows while dragging)
function DragOverlayItem({
  provider,
  modelId,
  isEnabled,
  index,
  pricing,
  freeTierKind,
}: {
  provider: string
  modelId: string
  isEnabled: boolean
  index: number
  pricing?: ModelPricingInfo
  freeTierKind?: FreeTierKind
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 px-3 py-2 border rounded-lg shadow-lg",
        isEnabled
          ? "bg-background border-primary"
          : "bg-muted/40 border-muted-foreground/30"
      )}
    >
      <GripVertical className="h-4 w-4 text-muted-foreground/50" />
      <div className="w-6 text-center">
        {isEnabled ? (
          <span className="text-xs font-mono font-medium text-primary">{index + 1}</span>
        ) : (
          <Ban className="h-3.5 w-3.5 text-muted-foreground/50 mx-auto" />
        )}
      </div>
      <span className="text-sm font-mono flex-1">{modelId}</span>
      <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
        {provider}
      </span>
      {pricing && (
        <ModelPricingBadge
          inputPricePerMillion={pricing.input}
          outputPricePerMillion={pricing.output}
          freeTierKind={freeTierKind}
        />
      )}
    </div>
  )
}

// Droppable zone for the threshold
function ThresholdDropZone({ isOver }: { isOver: boolean }) {
  const { setNodeRef } = useDroppable({ id: "threshold-zone" })

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "relative border-y-2 border-dashed transition-all",
        isOver
          ? "border-primary bg-primary/10 py-6"
          : "border-muted-foreground/30 py-3",
        "group"
      )}
    >
      <div className="flex items-center justify-center gap-2">
        <div className="h-px flex-1 bg-gradient-to-r from-transparent via-muted-foreground/30 to-transparent" />
        <span
          className={cn(
            "text-xs font-medium px-3 py-1 rounded-full transition-colors",
            isOver
              ? "bg-primary text-primary-foreground"
              : "bg-muted text-muted-foreground"
          )}
        >
          {isOver ? "Drop to enable" : "Threshold"}
        </span>
        <div className="h-px flex-1 bg-gradient-to-r from-transparent via-muted-foreground/30 to-transparent" />
      </div>
    </div>
  )
}

// Droppable zone for disabled section
function DisabledDropZone({
  children,
  isOver,
}: {
  children: React.ReactNode
  isOver: boolean
}) {
  const { setNodeRef } = useDroppable({ id: "disabled-zone" })

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "max-h-[400px] overflow-y-auto transition-colors",
        isOver && "bg-muted/40"
      )}
    >
      {children}
    </div>
  )
}

export function DragThresholdModelSelector({
  availableModels,
  enabledModels,
  onChange,
  disabled = false,
  title,
  description,
  className,
  disableDragOverlay = false,
  modelPricing,
  modelParamCounts,
  freeTierKinds,
}: DragThresholdModelSelectorProps) {
  const [activeId, setActiveId] = useState<string | null>(null)
  const [overZone, setOverZone] = useState<string | null>(null)
  const [disabledSearch, setDisabledSearch] = useState("")
  const [disabledSort, setDisabledSort] = useState<SortOption>('name')
  const [collapsedProviders, setCollapsedProviders] = useState<Set<string> | "all">("all")

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  )

  // Create a set of enabled models for quick lookup
  const enabledSet = useMemo(
    () => new Set(enabledModels.map(([p, m]) => getModelKey(p, m))),
    [enabledModels]
  )

  // Build lists with stable IDs
  const { enabledItems, disabledItems, allItemsMap } = useMemo(() => {
    const enabled = enabledModels.map(([provider, modelId]) => ({
      id: getModelKey(provider, modelId),
      provider,
      modelId,
    }))

    const disabled = availableModels
      .filter((m) => !enabledSet.has(getModelKey(m.provider, m.id)))
      .map((m) => ({
        id: getModelKey(m.provider, m.id),
        provider: m.provider,
        modelId: m.id,
      }))
      .sort((a, b) => {
        const providerCompare = a.provider.localeCompare(b.provider)
        if (providerCompare !== 0) return providerCompare
        return a.modelId.localeCompare(b.modelId)
      })

    const map = new Map<string, { provider: string; modelId: string }>()
    for (const item of [...enabled, ...disabled]) {
      map.set(item.id, { provider: item.provider, modelId: item.modelId })
    }

    return { enabledItems: enabled, disabledItems: disabled, allItemsMap: map }
  }, [availableModels, enabledModels, enabledSet])

  const enabledIds = enabledItems.map((item) => item.id)

  // Group disabled items by provider for collapsible rendering
  const disabledByProvider = useMemo(() => {
    const groups: Record<string, typeof disabledItems> = {}
    for (const item of disabledItems) {
      if (!groups[item.provider]) groups[item.provider] = []
      groups[item.provider].push(item)
    }
    return groups
  }, [disabledItems])

  const disabledProviders = useMemo(
    () => Object.keys(disabledByProvider).sort(),
    [disabledByProvider]
  )

  // Helper to get average price for a model
  const getModelAvgPrice = (provider: string, modelId: string): number => {
    const pricing = modelPricing?.[`${provider}/${modelId}`]
    if (!pricing) return Infinity
    const input = pricing.input ?? Infinity
    const output = pricing.output ?? Infinity
    if (input === Infinity && output === Infinity) return Infinity
    if (input === Infinity) return output
    if (output === Infinity) return input
    return (input + output) / 2
  }

  // Filter and sort disabled items
  const searchLower = disabledSearch.toLowerCase()
  const filteredDisabledItems = useMemo(() => {
    let items = disabledItems
    if (searchLower) {
      items = items.filter(
        (item) =>
          item.modelId.toLowerCase().includes(searchLower) ||
          item.provider.toLowerCase().includes(searchLower)
      )
    }
    if (disabledSort !== 'name') {
      items = [...items].sort((a, b) => {
        switch (disabledSort) {
          case 'provider': {
            const providerCmp = a.provider.localeCompare(b.provider)
            return providerCmp !== 0 ? providerCmp : a.modelId.localeCompare(b.modelId)
          }
          case 'price-asc': {
            const priceA = getModelAvgPrice(a.provider, a.modelId)
            const priceB = getModelAvgPrice(b.provider, b.modelId)
            return priceA - priceB || a.modelId.localeCompare(b.modelId)
          }
          case 'price-desc': {
            const priceA = getModelAvgPrice(a.provider, a.modelId)
            const priceB = getModelAvgPrice(b.provider, b.modelId)
            // Push models without pricing to the bottom
            if (priceA === Infinity && priceB === Infinity) return a.modelId.localeCompare(b.modelId)
            if (priceA === Infinity) return 1
            if (priceB === Infinity) return -1
            return priceB - priceA || a.modelId.localeCompare(b.modelId)
          }
          case 'params-asc': {
            const pA = modelParamCounts?.[`${a.provider}/${a.modelId}`]
            const pB = modelParamCounts?.[`${b.provider}/${b.modelId}`]
            const numA = pA ? parseParamCount(pA) : Infinity
            const numB = pB ? parseParamCount(pB) : Infinity
            if (numA === Infinity && numB === Infinity) return a.modelId.localeCompare(b.modelId)
            if (numA === Infinity) return 1
            if (numB === Infinity) return -1
            return numA - numB || a.modelId.localeCompare(b.modelId)
          }
          case 'params-desc': {
            const pA = modelParamCounts?.[`${a.provider}/${a.modelId}`]
            const pB = modelParamCounts?.[`${b.provider}/${b.modelId}`]
            const numA = pA ? parseParamCount(pA) : Infinity
            const numB = pB ? parseParamCount(pB) : Infinity
            if (numA === Infinity && numB === Infinity) return a.modelId.localeCompare(b.modelId)
            if (numA === Infinity) return 1
            if (numB === Infinity) return -1
            return numB - numA || a.modelId.localeCompare(b.modelId)
          }
          default:
            return a.modelId.localeCompare(b.modelId)
        }
      })
    }
    return items
  }, [disabledItems, searchLower, disabledSort, modelPricing, modelParamCounts])

  // Whether to show flat list (no provider grouping) - used for price/params sorting
  const showFlatList = disabledSort !== 'name' && disabledSort !== 'provider'

  // Visible disabled items (filtered + not in collapsed providers)
  const visibleDisabledItems = useMemo(() => {
    if (showFlatList) return filteredDisabledItems
    return filteredDisabledItems.filter((item) => {
      if (collapsedProviders === "all") return false
      return !collapsedProviders.has(item.provider)
    })
  }, [filteredDisabledItems, collapsedProviders, showFlatList])

  const disabledIds = visibleDisabledItems.map((item) => item.id)

  const toggleProviderCollapse = (provider: string) => {
    setCollapsedProviders((prev) => {
      if (prev === "all") {
        // All collapsed -> expand this one (collapse all others)
        const allProviders = new Set(disabledProviders)
        allProviders.delete(provider)
        return allProviders
      }
      const next = new Set(prev)
      if (next.has(provider)) {
        next.delete(provider)
      } else {
        next.add(provider)
      }
      return next
    })
  }

  const isProviderCollapsed = (provider: string) => {
    if (collapsedProviders === "all") return true
    return collapsedProviders.has(provider)
  }

  // Find active item info
  const activeItem = activeId ? allItemsMap.get(activeId) : null
  const activeIsEnabled = activeId ? enabledSet.has(activeId) : false
  const activeIndex = activeId ? enabledIds.indexOf(activeId) : -1

  const handleDragStart = (event: DragStartEvent) => {
    setActiveId(event.active.id as string)
  }

  const handleDragOver = (event: DragOverEvent) => {
    const { over } = event
    if (over) {
      if (over.id === "threshold-zone" || over.id === "disabled-zone") {
        setOverZone(over.id as string)
      } else {
        setOverZone(null)
      }
    } else {
      setOverZone(null)
    }
  }

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    setActiveId(null)
    setOverZone(null)

    if (!over || disabled) return

    const activeKey = active.id as string
    const overKey = over.id as string
    const [activeProvider, activeModelId] = parseModelKey(activeKey)
    const wasEnabled = enabledSet.has(activeKey)

    // Dropping on threshold zone - enable at end
    if (overKey === "threshold-zone") {
      if (!wasEnabled) {
        onChange([...enabledModels, [activeProvider, activeModelId]])
      }
      return
    }

    // Dropping on disabled zone - disable
    if (overKey === "disabled-zone") {
      if (wasEnabled) {
        onChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      }
      return
    }

    // Dropping on another item
    const overIsEnabled = enabledSet.has(overKey)

    if (wasEnabled && overIsEnabled) {
      // Reorder within enabled
      const oldIndex = enabledIds.indexOf(activeKey)
      const newIndex = enabledIds.indexOf(overKey)
      if (oldIndex !== newIndex) {
        const newEnabled = [...enabledModels]
        const [removed] = newEnabled.splice(oldIndex, 1)
        newEnabled.splice(newIndex, 0, removed)
        onChange(newEnabled)
      }
    } else if (wasEnabled && !overIsEnabled) {
      // Move from enabled to disabled
      onChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
    } else if (!wasEnabled && overIsEnabled) {
      // Move from disabled to enabled (insert at position)
      const newIndex = enabledIds.indexOf(overKey)
      const newEnabled = [...enabledModels]
      newEnabled.splice(newIndex, 0, [activeProvider, activeModelId])
      onChange(newEnabled)
    }
    // disabled -> disabled: no change
  }

  const handleToggle = (provider: string, modelId: string) => {
    if (disabled) return
    const key = getModelKey(provider, modelId)
    if (enabledSet.has(key)) {
      onChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== key))
    } else {
      onChange([...enabledModels, [provider, modelId]])
    }
  }

  return (
    <div className={cn("space-y-2", className)}>
      {/* Header */}
      {(title || description) && (
        <div className="mb-3">
          {title && <h4 className="font-medium text-sm">{title}</h4>}
          {description && (
            <p className="text-xs text-muted-foreground mt-1">{description}</p>
          )}
        </div>
      )}

      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragStart={handleDragStart}
        onDragOver={handleDragOver}
        onDragEnd={handleDragEnd}
      >
        {/* Main container */}
        <div className="border rounded-lg overflow-hidden">
          {/* Enabled section header */}
          <div className="bg-primary/5 px-4 py-2 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Zap className="h-4 w-4 text-primary" />
              <span className="text-xs font-medium">Enabled Models</span>
            </div>
            <span className="text-xs text-muted-foreground">
              {enabledItems.length} model{enabledItems.length !== 1 ? "s" : ""} • Priority order
            </span>
          </div>

          {/* Enabled models */}
          <SortableContext items={enabledIds} strategy={verticalListSortingStrategy}>
            <div className="min-h-[60px]">
              {enabledItems.length === 0 ? (
                <div className="p-4 text-center text-sm text-muted-foreground">
                  Drag models here to enable them
                </div>
              ) : (
                enabledItems.map((item, index) => (
                  <SortableRow
                    key={item.id}
                    id={item.id}
                    provider={item.provider}
                    modelId={item.modelId}
                    index={index}
                    isEnabled={true}
                    disabled={disabled}
                    onToggle={() => handleToggle(item.provider, item.modelId)}
                    pricing={modelPricing?.[`${item.provider}/${item.modelId}`]}
                    freeTierKind={freeTierKinds?.[item.provider]}
                  />
                ))
              )}
            </div>
          </SortableContext>

          {/* Threshold divider */}
          <ThresholdDropZone isOver={overZone === "threshold-zone"} />

          {/* Disabled section header */}
          <div className="bg-muted/30 px-4 py-2 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Ban className="h-4 w-4 text-muted-foreground/70" />
              <span className="text-xs font-medium text-muted-foreground">Disabled Models</span>
            </div>
            <span className="text-xs text-muted-foreground/70">
              {disabledItems.length} model{disabledItems.length !== 1 ? "s" : ""} • Click to enable
            </span>
          </div>

          {/* Search and sort controls for disabled models */}
          {disabledItems.length > 0 && (
            <div className="flex items-center gap-2 px-3 py-2 border-b bg-background">
              <Search className="h-3.5 w-3.5 text-muted-foreground/50 shrink-0" />
              <input
                type="text"
                placeholder="Search models..."
                value={disabledSearch}
                onChange={(e) => setDisabledSearch(e.target.value)}
                className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/50"
              />
              <div className="flex items-center gap-1.5 shrink-0 border-l pl-2">
                <ArrowUpDown className="h-3.5 w-3.5 text-muted-foreground/50" />
                <select
                  value={disabledSort}
                  onChange={(e) => setDisabledSort(e.target.value as SortOption)}
                  className="bg-transparent text-xs text-muted-foreground outline-none cursor-pointer"
                >
                  {SORT_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}

          {/* Disabled models - grouped by provider or flat list depending on sort */}
          <SortableContext items={disabledIds} strategy={verticalListSortingStrategy}>
            <DisabledDropZone isOver={overZone === "disabled-zone"}>
              {disabledItems.length === 0 ? (
                <div className="p-4 text-center text-sm text-muted-foreground/60">
                  All models are enabled
                </div>
              ) : filteredDisabledItems.length === 0 ? (
                <div className="p-4 text-center text-sm text-muted-foreground/60">
                  No models match &ldquo;{disabledSearch}&rdquo;
                </div>
              ) : showFlatList ? (
                /* Flat list for price sorting */
                filteredDisabledItems.map((item, index) => (
                  <SortableRow
                    key={item.id}
                    id={item.id}
                    provider={item.provider}
                    modelId={item.modelId}
                    index={index}
                    isEnabled={false}
                    disabled={disabled}
                    onToggle={() => handleToggle(item.provider, item.modelId)}
                    pricing={modelPricing?.[`${item.provider}/${item.modelId}`]}
                    freeTierKind={freeTierKinds?.[item.provider]}
                  />
                ))
              ) : (
                /* Provider-grouped list for name/provider sorting */
                disabledProviders.map((provider) => {
                  const providerItems = (disabledByProvider[provider] || []).filter(
                    (item) =>
                      !searchLower ||
                      item.modelId.toLowerCase().includes(searchLower) ||
                      item.provider.toLowerCase().includes(searchLower)
                  )
                  if (providerItems.length === 0) return null
                  const collapsed = isProviderCollapsed(provider)

                  return (
                    <div key={provider}>
                      <button
                        type="button"
                        onClick={() => toggleProviderCollapse(provider)}
                        className="flex items-center gap-2 w-full px-3 py-1.5 bg-muted/20 border-b text-left hover:bg-muted/40 transition-colors"
                      >
                        {collapsed ? (
                          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/60 shrink-0" />
                        ) : (
                          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground/60 shrink-0" />
                        )}
                        <span className="text-xs font-medium text-muted-foreground">{provider}</span>
                        <span className="text-xs text-muted-foreground/60 ml-auto">{providerItems.length}</span>
                      </button>
                      {!collapsed &&
                        providerItems.map((item, index) => (
                          <SortableRow
                            key={item.id}
                            id={item.id}
                            provider={item.provider}
                            modelId={item.modelId}
                            index={index}
                            isEnabled={false}
                            disabled={disabled}
                            onToggle={() => handleToggle(item.provider, item.modelId)}
                            pricing={modelPricing?.[`${item.provider}/${item.modelId}`]}
                            freeTierKind={freeTierKinds?.[item.provider]}
                            showProvider={false}
                          />
                        ))}
                    </div>
                  )
                })
              )}
            </DisabledDropZone>
          </SortableContext>
        </div>

        {/* Drag overlay - disabled in modals/dialogs due to transform offset issues */}
        {!disableDragOverlay && (
          <DragOverlay>
            {activeId && activeItem ? (
              <DragOverlayItem
                provider={activeItem.provider}
                modelId={activeItem.modelId}
                isEnabled={activeIsEnabled}
                index={activeIndex}
                pricing={modelPricing?.[`${activeItem.provider}/${activeItem.modelId}`]}
                freeTierKind={freeTierKinds?.[activeItem.provider]}
              />
            ) : null}
          </DragOverlay>
        )}
      </DndContext>

      {/* Help text */}
      <p className="text-xs text-muted-foreground">
        Drag models to reorder priorities. Drop below the threshold to disable.
        Click any row to toggle.
      </p>
    </div>
  )
}
