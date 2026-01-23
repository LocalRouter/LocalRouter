/**
 * StrategyModelConfiguration Component
 *
 * Main component for configuring model routing for a strategy.
 * Contains three sections:
 * 1. Allowed Models - Hierarchical selection of which models are permitted
 * 2. Auto Router - Enable localrouter/auto with prioritized model fallback
 * 3. Strong/Weak Routing - RouteLLM-based intelligent routing
 *
 * Used in:
 * - Client -> Models tab
 * - Resources -> Model Routing tab
 */

import { useState, useEffect, useMemo, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { ChevronDown, Zap, Brain, Info } from "lucide-react"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import {
  AllowedModelsSelector,
  AllowedModelsSelection,
  Model,
} from "./AllowedModelsSelector"
import { PrioritizedModelSelector } from "./PrioritizedModelSelector"
import { RouteLLMStatusIndicator } from "@/components/routellm/RouteLLMStatusIndicator"
import { ThresholdSlider } from "@/components/routellm/ThresholdSlider"
import { RouteLLMStatus } from "@/components/routellm/types"

// Strategy configuration types
export interface AutoModelConfig {
  enabled: boolean
  prioritized_models: [string, string][]
  available_models: [string, string][]
  routellm_config?: RouteLLMConfig
}

export interface RouteLLMConfig {
  enabled: boolean
  threshold: number
  weak_models: [string, string][]
}

export interface StrategyConfig {
  id: string
  name: string
  parent: string | null
  allowed_models: AllowedModelsSelection
  auto_config: AutoModelConfig | null
  rate_limits: any[]
}

interface StrategyModelConfigurationProps {
  /**
   * Strategy ID to configure
   */
  strategyId: string
  /**
   * Whether the configuration is read-only
   */
  readOnly?: boolean
  /**
   * Callback when configuration is saved
   */
  onSave?: () => void
  /**
   * Additional CSS classes
   */
  className?: string
}

export function StrategyModelConfiguration({
  strategyId,
  readOnly = false,
  onSave,
  className,
}: StrategyModelConfigurationProps) {
  const [strategy, setStrategy] = useState<StrategyConfig | null>(null)
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [routellmStatus, setRoutellmStatus] = useState<RouteLLMStatus | null>(null)

  // Section collapse state
  const [allowedModelsOpen, setAllowedModelsOpen] = useState(true)
  const [autoRouterOpen, setAutoRouterOpen] = useState(true)
  const [strongWeakOpen, setStrongWeakOpen] = useState(true)

  // Debounce refs for update operations
  const updateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingUpdatesRef = useRef<Partial<StrategyConfig> | null>(null)

  // Load data on mount
  useEffect(() => {
    loadData()
    loadRouteLLMStatus()
  }, [strategyId])

  // Cleanup debounce timeout on unmount
  useEffect(() => {
    return () => {
      if (updateTimeoutRef.current) {
        clearTimeout(updateTimeoutRef.current)
      }
    }
  }, [])

  const loadData = async () => {
    setLoading(true)
    try {
      const [strategyData, modelsData] = await Promise.all([
        invoke<StrategyConfig>("get_strategy", { strategyId }),
        invoke<Model[]>("list_all_models"),
      ])
      setStrategy(strategyData)
      setModels(modelsData)
    } catch (error) {
      console.error("Failed to load strategy:", error)
    } finally {
      setLoading(false)
    }
  }

  const loadRouteLLMStatus = async () => {
    try {
      const status = await invoke<RouteLLMStatus>("routellm_get_status")
      setRoutellmStatus(status)
    } catch (error) {
      console.error("Failed to load RouteLLM status:", error)
    }
  }

  // Get models allowed by the current selection
  const allowedModels = useMemo(() => {
    if (!strategy) return models

    // Handle both old and new field names with defaults
    const allowed = strategy.allowed_models || {}
    const selectedAll = allowed.selected_all ?? true
    const selectedProviders = allowed.selected_providers || []
    const selectedModels = allowed.selected_models || []

    // If selected_all is true, all models are allowed
    if (selectedAll) {
      return models
    }

    return models.filter((m) => {
      // Check if provider is fully allowed
      if (selectedProviders.some((p: string) => p.toLowerCase() === m.provider.toLowerCase())) return true
      // Check if individual model is allowed
      return selectedModels.some(
        ([p, id]: [string, string]) => p.toLowerCase() === m.provider.toLowerCase() && id.toLowerCase() === m.id.toLowerCase()
      )
    })
  }, [models, strategy?.allowed_models])

  // Update strategy on backend with debouncing to prevent race conditions
  const updateStrategy = useCallback((updates: Partial<StrategyConfig>) => {
    if (!strategy || readOnly) return

    // Merge with pending updates
    pendingUpdatesRef.current = {
      ...pendingUpdatesRef.current,
      ...updates,
    }

    // Update local state immediately for responsive UI
    setStrategy((prev) => (prev ? { ...prev, ...updates } : null))

    // Clear existing timeout
    if (updateTimeoutRef.current) {
      clearTimeout(updateTimeoutRef.current)
    }

    // Debounce the API call
    updateTimeoutRef.current = setTimeout(async () => {
      const pendingUpdates = pendingUpdatesRef.current
      pendingUpdatesRef.current = null

      if (!pendingUpdates) return

      setSaving(true)
      try {
        await invoke("update_strategy", {
          strategyId: strategy.id,
          name: null,
          allowedModels: pendingUpdates.allowed_models || null,
          autoConfig: pendingUpdates.auto_config !== undefined ? pendingUpdates.auto_config : null,
          rateLimits: null,
        })
        onSave?.()
      } catch (error) {
        console.error("Failed to update strategy:", error)
      } finally {
        setSaving(false)
      }
    }, 300) // 300ms debounce
  }, [strategy, readOnly, onSave])

  // Handler for allowed models change
  const handleAllowedModelsChange = (selection: AllowedModelsSelection) => {
    updateStrategy({ allowed_models: selection })
  }

  // Handler for auto config toggle
  const handleAutoConfigToggle = (enabled: boolean) => {
    const newConfig: AutoModelConfig = {
      enabled,
      prioritized_models: strategy?.auto_config?.prioritized_models || [],
      available_models: strategy?.auto_config?.available_models || [],
      routellm_config: strategy?.auto_config?.routellm_config,
    }
    updateStrategy({ auto_config: newConfig })
  }

  // Handler for prioritized models change
  const handlePrioritizedModelsChange = (models: [string, string][]) => {
    if (!strategy?.auto_config) return
    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        prioritized_models: models,
      },
    })
  }

  // Handler for RouteLLM toggle
  const handleRouteLLMToggle = (enabled: boolean) => {
    if (!strategy?.auto_config) return

    const newRouteLLMConfig: RouteLLMConfig = {
      enabled,
      threshold: strategy.auto_config.routellm_config?.threshold ?? 0.3,
      weak_models: strategy.auto_config.routellm_config?.weak_models ?? [],
    }

    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        routellm_config: newRouteLLMConfig,
      },
    })
  }

  // Handler for threshold change
  const handleThresholdChange = (threshold: number) => {
    if (!strategy?.auto_config?.routellm_config) return
    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        routellm_config: {
          ...strategy.auto_config.routellm_config,
          threshold,
        },
      },
    })
  }

  // Handler for weak models change
  const handleWeakModelsChange = (models: [string, string][]) => {
    if (!strategy?.auto_config?.routellm_config) return
    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        routellm_config: {
          ...strategy.auto_config.routellm_config,
          weak_models: models,
        },
      },
    })
  }

  if (loading) {
    return (
      <div className={cn("space-y-4", className)}>
        <Card>
          <CardContent className="py-8">
            <div className="text-center text-muted-foreground">
              Loading strategy configuration...
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  if (!strategy) {
    return (
      <div className={cn("space-y-4", className)}>
        <Card>
          <CardContent className="py-8">
            <div className="text-center text-destructive">
              Strategy not found
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  const autoConfig = strategy.auto_config
  const routellmConfig = autoConfig?.routellm_config

  return (
    <div className={cn("space-y-4", className)}>
      {/* Section 1: Allowed Models */}
      <Collapsible open={allowedModelsOpen} onOpenChange={setAllowedModelsOpen}>
        <Card>
          <CollapsibleTrigger asChild>
            <CardHeader className="cursor-pointer hover:bg-muted/30 transition-colors">
              <div className="flex items-center justify-between">
                <div>
                  <CardTitle className="text-base">Allowed Models</CardTitle>
                  <CardDescription>
                    Select which models clients using this strategy can access
                  </CardDescription>
                </div>
                <ChevronDown
                  className={cn(
                    "h-5 w-5 text-muted-foreground transition-transform",
                    allowedModelsOpen && "rotate-180"
                  )}
                />
              </div>
            </CardHeader>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <CardContent>
              <AllowedModelsSelector
                models={models}
                value={strategy.allowed_models}
                onChange={handleAllowedModelsChange}
                disabled={readOnly || saving}
              />
            </CardContent>
          </CollapsibleContent>
        </Card>
      </Collapsible>

      {/* Section 2: Auto Router */}
      <Collapsible open={autoRouterOpen} onOpenChange={setAutoRouterOpen}>
        <Card>
          <CollapsibleTrigger asChild>
            <CardHeader className="cursor-pointer hover:bg-muted/30 transition-colors">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-primary/10">
                    <Zap className="h-4 w-4 text-primary" />
                  </div>
                  <div>
                    <CardTitle className="text-base flex items-center gap-2">
                      Auto Router
                      <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
                        localrouter/auto
                      </code>
                    </CardTitle>
                    <CardDescription>
                      Intelligent fallback routing with prioritized models
                    </CardDescription>
                  </div>
                </div>
                <div className="flex items-center gap-3">
                  <Switch
                    checked={autoConfig?.enabled ?? false}
                    onCheckedChange={handleAutoConfigToggle}
                    disabled={readOnly || saving}
                    onClick={(e) => e.stopPropagation()}
                  />
                  <ChevronDown
                    className={cn(
                      "h-5 w-5 text-muted-foreground transition-transform",
                      autoRouterOpen && "rotate-180"
                    )}
                  />
                </div>
              </div>
            </CardHeader>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <CardContent className="space-y-4">
              {!autoConfig?.enabled ? (
                <div className="text-center py-4 text-muted-foreground text-sm">
                  Enable Auto Router to configure prioritized model fallback
                </div>
              ) : (
                <>
                  <div className="p-3 rounded-lg bg-blue-500/10 border border-blue-500/20">
                    <p className="text-sm text-blue-700 dark:text-blue-300">
                      <strong>How it works:</strong> When a client uses{" "}
                      <code className="bg-blue-500/20 px-1 rounded">localrouter/auto</code>,
                      models are tried in priority order. If one fails (rate limited,
                      unavailable, etc.), the next model is tried automatically.
                    </p>
                  </div>

                  <PrioritizedModelSelector
                    title="Strong Models (Prioritized)"
                    description="Models tried in order for complex requests. Add models and drag to reorder."
                    availableModels={allowedModels}
                    selectedModels={autoConfig.prioritized_models}
                    onChange={handlePrioritizedModelsChange}
                    disabled={readOnly || saving}
                    emptyText="No models configured. Add models to enable auto-routing."
                  />
                </>
              )}
            </CardContent>
          </CollapsibleContent>
        </Card>
      </Collapsible>

      {/* Section 3: Strong/Weak Routing (RouteLLM) */}
      <Collapsible open={strongWeakOpen} onOpenChange={setStrongWeakOpen}>
        <Card>
          <CollapsibleTrigger asChild>
            <CardHeader className="cursor-pointer hover:bg-muted/30 transition-colors">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-lg bg-purple-500/10">
                    <Brain className="h-4 w-4 text-purple-500" />
                  </div>
                  <div>
                    <CardTitle className="text-base flex items-center gap-2">
                      Strong/Weak Routing
                      <span className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-700 dark:text-purple-300 font-medium">
                        EXPERIMENTAL
                      </span>
                    </CardTitle>
                    <CardDescription>
                      ML-powered routing to optimize cost vs quality
                    </CardDescription>
                  </div>
                </div>
                <div className="flex items-center gap-3">
                  <TooltipProvider>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <div>
                          <Switch
                            checked={routellmConfig?.enabled ?? false}
                            onCheckedChange={handleRouteLLMToggle}
                            disabled={readOnly || saving || !autoConfig?.enabled}
                            onClick={(e) => e.stopPropagation()}
                          />
                        </div>
                      </TooltipTrigger>
                      {!autoConfig?.enabled && (
                        <TooltipContent>
                          Enable Auto Router first to use Strong/Weak routing
                        </TooltipContent>
                      )}
                    </Tooltip>
                  </TooltipProvider>
                  <ChevronDown
                    className={cn(
                      "h-5 w-5 text-muted-foreground transition-transform",
                      strongWeakOpen && "rotate-180"
                    )}
                  />
                </div>
              </div>
            </CardHeader>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <CardContent className="space-y-4">
              {!autoConfig?.enabled ? (
                <div className="text-center py-4 text-muted-foreground text-sm">
                  Enable Auto Router first to configure Strong/Weak routing
                </div>
              ) : !routellmConfig?.enabled ? (
                <div className="text-center py-4 text-muted-foreground text-sm">
                  Enable Strong/Weak Routing to configure ML-based model selection
                </div>
              ) : (
                <>
                  {/* RouteLLM Status */}
                  {routellmStatus && (
                    <RouteLLMStatusIndicator status={routellmStatus} compact />
                  )}

                  {/* Resource Requirements */}
                  <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/20">
                    <div className="flex items-start gap-2">
                      <Info className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
                      <div className="text-xs text-amber-700 dark:text-amber-300">
                        <p className="font-medium mb-1">Resource Requirements</p>
                        <div className="grid grid-cols-2 gap-x-4 gap-y-1">
                          <span>Disk Space:</span>
                          <span>1.08 GB</span>
                          <span>Memory (loaded):</span>
                          <span>~2.65 GB</span>
                          <span>Cold Start:</span>
                          <span>~1.5s</span>
                          <span>Per-request:</span>
                          <span>~10ms</span>
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* How it works */}
                  <div className="p-3 rounded-lg bg-purple-500/10 border border-purple-500/20">
                    <p className="text-sm text-purple-700 dark:text-purple-300">
                      <strong>How it works:</strong> Each request is analyzed by an ML model
                      to determine if it's "simple" or "complex". Simple requests use{" "}
                      <strong>Weak Models</strong> (cheaper), while complex requests use{" "}
                      <strong>Strong Models</strong> (the prioritized list above).
                    </p>
                  </div>

                  {/* Threshold Slider */}
                  <ThresholdSlider
                    value={routellmConfig.threshold}
                    onChange={handleThresholdChange}
                  />

                  {/* Strong Models Info */}
                  <div className="p-3 rounded-lg bg-muted/50 border">
                    <div className="flex items-center gap-2 mb-2">
                      <Zap className="h-4 w-4 text-primary" />
                      <span className="text-sm font-medium">Strong Models</span>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      Strong models use the same prioritized list from Auto Router above.
                      They are used for complex requests that require higher quality responses.
                    </p>
                    {autoConfig.prioritized_models.length === 0 && (
                      <p className="text-xs text-amber-600 dark:text-amber-400 mt-2">
                        No strong models configured. Add models to the prioritized list above.
                      </p>
                    )}
                  </div>

                  {/* Weak Models */}
                  <PrioritizedModelSelector
                    title="Weak Models (Cost Efficient)"
                    description="Used for simple requests. Typically faster and cheaper models."
                    availableModels={allowedModels}
                    selectedModels={routellmConfig.weak_models}
                    onChange={handleWeakModelsChange}
                    disabled={readOnly || saving}
                    emptyText="No weak models configured. Add cost-efficient models for simple requests."
                  />
                </>
              )}
            </CardContent>
          </CollapsibleContent>
        </Card>
      </Collapsible>
    </div>
  )
}
