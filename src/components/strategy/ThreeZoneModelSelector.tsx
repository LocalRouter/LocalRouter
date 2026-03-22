/**
 * ThreeZoneModelSelector - Three-zone drag-and-drop model selector
 *
 * Extends the DragThresholdModelSelector pattern to support three zones:
 * 1. Enabled (Strong) Models — priority-ordered, numbered 1..N
 * 2. Weak Models — only visible when showWeakZone prop is true
 * 3. Disabled Models — searchable, sortable, grouped by provider
 */

import { useState, useMemo, useRef, useCallback } from "react"
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
import { GripVertical, Zap, Ban, Search, ChevronRight, ChevronDown, ArrowUpDown, Brain } from "lucide-react"
import { cn } from "@/lib/utils"
import { ModelPricingBadge } from "@/components/shared/model-pricing-badge"
import type { FreeTierKind } from "@/types/tauri-commands"
import type { Model, ModelPricingInfo } from "./DragThresholdModelSelector"

const getModelKey = (provider: string, modelId: string) => `${provider}::${modelId}`
const parseModelKey = (key: string): [string, string] => {
  const [provider, modelId] = key.split("::")
  return [provider, modelId]
}

type SortOption = 'name' | 'provider' | 'price-asc' | 'price-desc' | 'params-asc' | 'params-desc'

const SORT_OPTIONS: { value: SortOption; label: string }[] = [
  { value: 'name', label: 'Name' },
  { value: 'provider', label: 'Provider' },
  { value: 'price-asc', label: 'Price: Low \u2192 High' },
  { value: 'price-desc', label: 'Price: High \u2192 Low' },
  { value: 'params-asc', label: 'Params: Small \u2192 Large' },
  { value: 'params-desc', label: 'Params: Large \u2192 Small' },
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

interface ThreeZoneModelSelectorProps {
  availableModels: Model[]
  enabledModels: [string, string][]      // strong models, priority ordered
  weakModels: [string, string][]         // weak models, ordered
  showWeakZone: boolean                  // controlled by RouteLLM toggle
  onEnabledModelsChange: (models: [string, string][]) => void
  onWeakModelsChange: (models: [string, string][]) => void
  disabled?: boolean
  className?: string
  disableDragOverlay?: boolean
  modelPricing?: Record<string, ModelPricingInfo>
  modelParamCounts?: Record<string, string>
  freeTierKinds?: Record<string, FreeTierKind>
}

// Sortable row component
function SortableRow({
  id,
  provider,
  modelId,
  index,
  zone,
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
  zone: 'enabled' | 'weak' | 'disabled'
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

  // Track pointer position to distinguish clicks from drags
  const pointerStartRef = useRef<{ x: number; y: number } | null>(null)

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    pointerStartRef.current = { x: e.clientX, y: e.clientY }
    // Call dnd-kit's onPointerDown if present
    listeners?.onPointerDown?.(e as any)
  }, [listeners])

  const handlePointerUp = useCallback((e: React.PointerEvent) => {
    if (disabled) return
    const start = pointerStartRef.current
    pointerStartRef.current = null
    if (!start) return
    // Only toggle if pointer didn't move more than the drag threshold
    const dx = e.clientX - start.x
    const dy = e.clientY - start.y
    if (Math.abs(dx) < 8 && Math.abs(dy) < 8) {
      onToggle()
    }
  }, [disabled, onToggle])

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
  }

  // Spread dnd-kit listeners but override onPointerDown with our wrapper
  const mergedListeners = { ...listeners, onPointerDown: handlePointerDown }

  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...mergedListeners}
      onPointerUp={handlePointerUp}
      className={cn(
        "flex items-center gap-3 px-3 py-2 border-b border-border/50 transition-colors",
        "cursor-grab active:cursor-grabbing touch-none select-none",
        zone === 'enabled' && "bg-background hover:bg-muted/30",
        zone === 'weak' && "bg-purple-500/5 hover:bg-purple-500/10",
        zone === 'disabled' && "bg-muted/20 hover:bg-muted/40 text-muted-foreground",
        isDragging && "opacity-50 bg-primary/10 z-50",
        disabled && "opacity-60 cursor-default"
      )}
    >
      {/* Drag handle icon */}
      <GripVertical
        className={cn(
          "h-4 w-4 shrink-0 transition-colors",
          zone === 'disabled' ? "text-muted-foreground/30" : "text-muted-foreground/50"
        )}
      />

      {/* Priority number for enabled/weak models, Ban icon for disabled */}
      <div className="w-6 text-center">
        {zone === 'enabled' ? (
          <span className="text-xs font-mono font-medium text-primary">{index + 1}</span>
        ) : zone === 'weak' ? (
          <span className="text-xs font-mono font-medium text-purple-500">{index + 1}</span>
        ) : (
          <Ban className="h-3.5 w-3.5 text-muted-foreground/50 mx-auto" />
        )}
      </div>

      {/* Model info + provider badge inline */}
      <div className="flex-1 min-w-0 flex items-center gap-2">
        <span
          className={cn(
            "text-sm font-mono truncate",
            zone === 'disabled' && "text-muted-foreground"
          )}
        >
          {modelId}
        </span>
        {zone === 'weak' && (
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-500 font-medium shrink-0">
            weak
          </span>
        )}
        {showProvider && (
          <span
            className={cn(
              "text-xs px-2 py-0.5 rounded-full shrink-0",
              zone === 'disabled'
                ? "bg-muted/50 text-muted-foreground/70"
                : "bg-muted text-muted-foreground"
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
  zone,
  index,
  pricing,
  freeTierKind,
}: {
  provider: string
  modelId: string
  zone: 'enabled' | 'weak' | 'disabled'
  index: number
  pricing?: ModelPricingInfo
  freeTierKind?: FreeTierKind
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 px-3 py-2 border rounded-lg shadow-lg",
        zone === 'enabled' && "bg-background border-primary",
        zone === 'weak' && "bg-purple-500/5 border-purple-500",
        zone === 'disabled' && "bg-muted/40 border-muted-foreground/30"
      )}
    >
      <GripVertical className="h-4 w-4 text-muted-foreground/50" />
      <div className="w-6 text-center">
        {zone === 'enabled' ? (
          <span className="text-xs font-mono font-medium text-primary">{index + 1}</span>
        ) : zone === 'weak' ? (
          <span className="text-xs font-mono font-medium text-purple-500">{index + 1}</span>
        ) : (
          <Ban className="h-3.5 w-3.5 text-muted-foreground/50 mx-auto" />
        )}
      </div>
      <span className="text-sm font-mono flex-1">{modelId}</span>
      {zone === 'weak' && (
        <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-500 font-medium shrink-0">
          weak
        </span>
      )}
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

// Droppable zone for the weak threshold
function WeakThresholdDropZone({ isOver }: { isOver: boolean }) {
  const { setNodeRef } = useDroppable({ id: "weak-threshold-zone" })

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "relative border-y-2 border-dashed transition-all",
        isOver
          ? "border-purple-500 bg-purple-500/10 py-6"
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
              ? "bg-purple-500 text-white"
              : "bg-muted text-muted-foreground"
          )}
        >
          {isOver ? "Drop to add as weak" : "Weak Models"}
        </span>
        <div className="h-px flex-1 bg-gradient-to-r from-transparent via-muted-foreground/30 to-transparent" />
      </div>
    </div>
  )
}

// Droppable zone for the disabled threshold
function DisabledThresholdDropZone({ isOver }: { isOver: boolean }) {
  const { setNodeRef } = useDroppable({ id: "disabled-threshold-zone" })

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
          {isOver ? "Drop to disable" : "Disabled"}
        </span>
        <div className="h-px flex-1 bg-gradient-to-r from-transparent via-muted-foreground/30 to-transparent" />
      </div>
    </div>
  )
}

// Droppable zone wrapping disabled section
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

export function ThreeZoneModelSelector({
  availableModels,
  enabledModels,
  weakModels,
  showWeakZone,
  onEnabledModelsChange,
  onWeakModelsChange,
  disabled = false,
  className,
  disableDragOverlay = false,
  modelPricing,
  modelParamCounts,
  freeTierKinds,
}: ThreeZoneModelSelectorProps) {
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

  // Create sets of enabled and weak models for quick lookup
  const enabledSet = useMemo(
    () => new Set(enabledModels.map(([p, m]) => getModelKey(p, m))),
    [enabledModels]
  )

  const weakSet = useMemo(
    () => new Set(weakModels.map(([p, m]) => getModelKey(p, m))),
    [weakModels]
  )

  // Build lists with stable IDs
  const { enabledItems, weakItems, disabledItems, allItemsMap } = useMemo(() => {
    const enabled = enabledModels.map(([provider, modelId]) => ({
      id: getModelKey(provider, modelId),
      provider,
      modelId,
    }))

    const weak = weakModels.map(([provider, modelId]) => ({
      id: getModelKey(provider, modelId),
      provider,
      modelId,
    }))

    const disabledList = availableModels
      .filter((m) => {
        const key = getModelKey(m.provider, m.id)
        return !enabledSet.has(key) && !weakSet.has(key)
      })
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
    for (const item of [...enabled, ...weak, ...disabledList]) {
      map.set(item.id, { provider: item.provider, modelId: item.modelId })
    }

    return { enabledItems: enabled, weakItems: weak, disabledItems: disabledList, allItemsMap: map }
  }, [availableModels, enabledModels, weakModels, enabledSet, weakSet])

  const enabledIds = enabledItems.map((item) => item.id)
  const weakIds = weakItems.map((item) => item.id)

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

  // Determine which zone the active item belongs to
  const getItemZone = (itemId: string): 'enabled' | 'weak' | 'disabled' => {
    if (enabledSet.has(itemId)) return 'enabled'
    if (weakSet.has(itemId)) return 'weak'
    return 'disabled'
  }

  // Find active item info
  const activeItem = activeId ? allItemsMap.get(activeId) : null
  const activeZone = activeId ? getItemZone(activeId) : 'disabled'
  const activeIndex = activeId
    ? activeZone === 'enabled'
      ? enabledIds.indexOf(activeId)
      : activeZone === 'weak'
        ? weakIds.indexOf(activeId)
        : -1
    : -1

  const handleDragStart = (event: DragStartEvent) => {
    setActiveId(event.active.id as string)
  }

  const handleDragOver = (event: DragOverEvent) => {
    const { over } = event
    if (over) {
      const overId = over.id as string
      if (
        overId === "weak-threshold-zone" ||
        overId === "disabled-threshold-zone" ||
        overId === "disabled-zone"
      ) {
        setOverZone(overId)
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
    const activeItemZone = getItemZone(activeKey)

    // Drop on weak-threshold-zone: Move model to weak zone (at end)
    if (overKey === "weak-threshold-zone") {
      if (activeItemZone === 'enabled') {
        onEnabledModelsChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
        onWeakModelsChange([...weakModels, [activeProvider, activeModelId]])
      } else if (activeItemZone === 'disabled') {
        onWeakModelsChange([...weakModels, [activeProvider, activeModelId]])
      }
      // weak -> weak-threshold: no-op (already in weak)
      return
    }

    // Drop on disabled-threshold-zone or disabled-zone: Move model to disabled
    if (overKey === "disabled-threshold-zone" || overKey === "disabled-zone") {
      if (activeItemZone === 'enabled') {
        onEnabledModelsChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      } else if (activeItemZone === 'weak') {
        onWeakModelsChange(weakModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      }
      // disabled -> disabled: no-op
      return
    }

    // Dropping on another item - determine the target zone
    const overItemZone = getItemZone(overKey)

    // Drag within enabled: reorder
    if (activeItemZone === 'enabled' && overItemZone === 'enabled') {
      const oldIndex = enabledIds.indexOf(activeKey)
      const newIndex = enabledIds.indexOf(overKey)
      if (oldIndex !== newIndex) {
        const newEnabled = [...enabledModels]
        const [removed] = newEnabled.splice(oldIndex, 1)
        newEnabled.splice(newIndex, 0, removed)
        onEnabledModelsChange(newEnabled)
      }
      return
    }

    // Drag within weak: reorder
    if (activeItemZone === 'weak' && overItemZone === 'weak') {
      const oldIndex = weakIds.indexOf(activeKey)
      const newIndex = weakIds.indexOf(overKey)
      if (oldIndex !== newIndex) {
        const newWeak = [...weakModels]
        const [removed] = newWeak.splice(oldIndex, 1)
        newWeak.splice(newIndex, 0, removed)
        onWeakModelsChange(newWeak)
      }
      return
    }

    // Drag from enabled to weak item: remove from enabled, insert in weak at position
    if (activeItemZone === 'enabled' && overItemZone === 'weak') {
      onEnabledModelsChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      const newIndex = weakIds.indexOf(overKey)
      const newWeak = [...weakModels]
      newWeak.splice(newIndex, 0, [activeProvider, activeModelId])
      onWeakModelsChange(newWeak)
      return
    }

    // Drag from weak to enabled item: remove from weak, insert in enabled at position
    if (activeItemZone === 'weak' && overItemZone === 'enabled') {
      onWeakModelsChange(weakModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      const newIndex = enabledIds.indexOf(overKey)
      const newEnabled = [...enabledModels]
      newEnabled.splice(newIndex, 0, [activeProvider, activeModelId])
      onEnabledModelsChange(newEnabled)
      return
    }

    // Drag from disabled to enabled item: add to enabled at position
    if (activeItemZone === 'disabled' && overItemZone === 'enabled') {
      const newIndex = enabledIds.indexOf(overKey)
      const newEnabled = [...enabledModels]
      newEnabled.splice(newIndex, 0, [activeProvider, activeModelId])
      onEnabledModelsChange(newEnabled)
      return
    }

    // Drag from disabled to weak item: add to weak at position (only if showWeakZone)
    if (activeItemZone === 'disabled' && overItemZone === 'weak' && showWeakZone) {
      const newIndex = weakIds.indexOf(overKey)
      const newWeak = [...weakModels]
      newWeak.splice(newIndex, 0, [activeProvider, activeModelId])
      onWeakModelsChange(newWeak)
      return
    }

    // Drag from enabled to disabled item: remove from enabled
    if (activeItemZone === 'enabled' && overItemZone === 'disabled') {
      onEnabledModelsChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      return
    }

    // Drag from weak to disabled item: remove from weak
    if (activeItemZone === 'weak' && overItemZone === 'disabled') {
      onWeakModelsChange(weakModels.filter(([p, m]) => getModelKey(p, m) !== activeKey))
      return
    }

    // disabled -> disabled: no change
  }

  const handleToggle = (provider: string, modelId: string) => {
    if (disabled) return
    const key = getModelKey(provider, modelId)
    if (enabledSet.has(key)) {
      // Click on enabled model -> move to disabled
      onEnabledModelsChange(enabledModels.filter(([p, m]) => getModelKey(p, m) !== key))
    } else if (weakSet.has(key)) {
      // Click on weak model -> move to disabled
      onWeakModelsChange(weakModels.filter(([p, m]) => getModelKey(p, m) !== key))
    } else {
      // Click on disabled model -> add to enabled (at bottom)
      onEnabledModelsChange([...enabledModels, [provider, modelId]])
    }
  }

  return (
    <div className={cn("space-y-2", className)}>
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
              {enabledItems.length} model{enabledItems.length !== 1 ? "s" : ""} &bull; Priority order
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
                    zone="enabled"
                    disabled={disabled}
                    onToggle={() => handleToggle(item.provider, item.modelId)}
                    pricing={modelPricing?.[`${item.provider}/${item.modelId}`]}
                    freeTierKind={freeTierKinds?.[item.provider]}
                  />
                ))
              )}
            </div>
          </SortableContext>

          {/* Weak threshold drop zone - only when showWeakZone is true */}
          {showWeakZone && (
            <WeakThresholdDropZone isOver={overZone === "weak-threshold-zone"} />
          )}

          {/* Weak models header - only when showWeakZone is true */}
          {showWeakZone && (
            <div className="bg-purple-500/5 px-4 py-2 border-b flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Brain className="h-4 w-4 text-purple-500" />
                <span className="text-xs font-medium">Weak Models</span>
              </div>
              <span className="text-xs text-muted-foreground">
                {weakItems.length} model{weakItems.length !== 1 ? "s" : ""}
              </span>
            </div>
          )}

          {/* Weak models */}
          {showWeakZone && (
            <SortableContext items={weakIds} strategy={verticalListSortingStrategy}>
              <div className="min-h-[40px]">
                {weakItems.length === 0 ? (
                  <div className="p-3 text-center text-sm text-muted-foreground">
                    Drag models here to mark as weak
                  </div>
                ) : (
                  weakItems.map((item, index) => (
                    <SortableRow
                      key={item.id}
                      id={item.id}
                      provider={item.provider}
                      modelId={item.modelId}
                      index={index}
                      zone="weak"
                      disabled={disabled}
                      onToggle={() => handleToggle(item.provider, item.modelId)}
                      pricing={modelPricing?.[`${item.provider}/${item.modelId}`]}
                      freeTierKind={freeTierKinds?.[item.provider]}
                    />
                  ))
                )}
              </div>
            </SortableContext>
          )}

          {/* Disabled threshold drop zone */}
          <DisabledThresholdDropZone isOver={overZone === "disabled-threshold-zone"} />

          {/* Disabled section header */}
          <div className="bg-muted/30 px-4 py-2 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Ban className="h-4 w-4 text-muted-foreground/70" />
              <span className="text-xs font-medium text-muted-foreground">Disabled Models</span>
            </div>
            <span className="text-xs text-muted-foreground/70">
              {disabledItems.length} model{disabledItems.length !== 1 ? "s" : ""} &bull; Click to enable
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
                /* Flat list for price/params sorting */
                filteredDisabledItems.map((item, index) => (
                  <SortableRow
                    key={item.id}
                    id={item.id}
                    provider={item.provider}
                    modelId={item.modelId}
                    index={index}
                    zone="disabled"
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
                            zone="disabled"
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
                zone={activeZone}
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
        Drag models to reorder priorities. Drop below threshold to disable.
        Click any row to toggle.
      </p>
    </div>
  )
}
