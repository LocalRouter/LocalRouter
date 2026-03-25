/**
 * UnifiedModelsTab Component
 *
 * Unified models configuration tab for a client.
 * Merges the previous "Allowed Models" and "Auto Router" modes into one:
 * - All enabled models appear in /v1/models for direct routing
 * - The auto router (localrouter/auto) routes through enabled models in priority order
 * - Weak models (RouteLLM) can route simpler queries to cheaper models
 *
 * Sections:
 * 1. Model Selection (three-zone: Enabled / Weak / Disabled)
 * 2. Rate Limits
 * 3. Free-Tier Mode
 * 4. Weak Models (RouteLLM toggle + threshold + download)
 */

import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Bot, Brain, Coins, Download, Gauge, Loader2, MessageSquareWarning } from "lucide-react"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import {
  Card, CardContent, CardDescription, CardHeader, CardTitle,
} from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Button } from "@/components/ui/Button"
import { Progress } from "@/components/ui/progress"
import RateLimitEditor, { StrategyRateLimit } from "@/components/strategies/RateLimitEditor"
import { ThreeZoneModelSelector } from "@/components/strategy/ThreeZoneModelSelector"
import { ThresholdSelector } from "@/components/routellm/ThresholdSelector"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { ROUTELLM_REQUIREMENTS, RouteLLMStatus } from "@/components/routellm/types"
import { PermissionStateButton } from "@/components/permissions"
import type { ModelPricingInfo } from "@/components/strategy/DragThresholdModelSelector"
import type { FreeTierKind, ProviderFreeTierStatus, PermissionState } from "@/types/tauri-commands"

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AutoModelConfig {
  permission: PermissionState
  model_name: string
  prioritized_models: [string, string][]
  available_models: [string, string][]
  routellm_config?: RouteLLMConfig | null
}

interface RouteLLMConfig {
  enabled: boolean
  threshold: number
  weak_models: [string, string][]
}

interface StrategyConfig {
  id: string
  name: string
  parent: string | null
  allowed_models: {
    selected_all: boolean
    selected_providers: string[]
    selected_models: [string, string][]
  }
  auto_config: AutoModelConfig | null
  rate_limits: StrategyRateLimit[]
  free_tier_only?: boolean
  free_tier_fallback?: 'off' | 'ask' | 'allow'
}

interface Client {
  id: string
  name: string
  client_id: string
  strategy_id: string
}

interface UnifiedModelsTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

interface DetailedModelInfo {
  model_id: string
  provider_instance: string
  provider_type: string
  capabilities: string[]
  context_window: number
  input_price_per_million?: number | null
  output_price_per_million?: number | null
  parameter_count?: string | null
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ensureAutoConfig = (s: StrategyConfig): StrategyConfig => {
  if (s.auto_config) return s
  return {
    ...s,
    auto_config: {
      permission: 'allow' as PermissionState,
      model_name: 'localrouter/auto',
      prioritized_models: [],
      available_models: [],
      routellm_config: null,
    },
  }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function UnifiedModelsTab({
  client,
  onUpdate,
  onViewChange: _onViewChange,
}: UnifiedModelsTabProps) {
  // Strategy state
  const [strategy, setStrategy] = useState<StrategyConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [savingRateLimits, setSavingRateLimits] = useState(false)

  // Model metadata
  const [modelPricing, setModelPricing] = useState<Record<string, ModelPricingInfo>>({})
  const [modelParamCounts, setModelParamCounts] = useState<Record<string, string>>({})
  const [freeTierKinds, setFreeTierKinds] = useState<Record<string, FreeTierKind>>({})
  const [modelCapabilities, setModelCapabilities] = useState<Record<string, string[]>>({})
  const [modelContextWindows, setModelContextWindows] = useState<Record<string, number>>({})

  // RouteLLM state
  const [routellmStatus, setRoutellmStatus] = useState<RouteLLMStatus | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)

  // Incremental models
  const { models, loadingProviders, isFullyLoaded } = useIncrementalModels()

  // Debounce refs
  const updateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingUpdatesRef = useRef<Partial<StrategyConfig> | null>(null)
  const rateLimitTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // -------------------------------------------------------------------------
  // Load strategy on mount
  // -------------------------------------------------------------------------

  useEffect(() => {
    loadData()
  }, [client.strategy_id])

  // Cleanup debounce timeouts on unmount
  useEffect(() => {
    return () => {
      if (updateTimeoutRef.current) clearTimeout(updateTimeoutRef.current)
      if (rateLimitTimeoutRef.current) clearTimeout(rateLimitTimeoutRef.current)
    }
  }, [])

  const loadData = async () => {
    setLoading(true)
    try {
      const strategyData = await invoke<StrategyConfig>("get_strategy", {
        strategyId: client.strategy_id,
      })
      setStrategy(ensureAutoConfig(strategyData))
    } catch (error) {
      console.error("Failed to load strategy:", error)
    } finally {
      setLoading(false)
    }

    // Load pricing and free tier data in the background (non-blocking)
    loadPricingData()
  }

  const loadPricingData = async () => {
    try {
      const [detailedModels, ftStatuses] = await Promise.all([
        invoke<DetailedModelInfo[]>("list_all_models_detailed"),
        invoke<ProviderFreeTierStatus[]>("get_free_tier_status"),
      ])
      const pricingMap: Record<string, ModelPricingInfo> = {}
      const paramMap: Record<string, string> = {}
      const capMap: Record<string, string[]> = {}
      const ctxMap: Record<string, number> = {}
      for (const m of detailedModels) {
        const key = `${m.provider_instance}/${m.model_id}`
        pricingMap[key] = {
          input: m.input_price_per_million,
          output: m.output_price_per_million,
        }
        if (m.parameter_count) {
          paramMap[key] = m.parameter_count
        }
        if (m.capabilities.length > 0) {
          capMap[key] = m.capabilities
        }
        if (m.context_window > 0) {
          ctxMap[key] = m.context_window
        }
      }
      setModelPricing(pricingMap)
      setModelParamCounts(paramMap)
      setModelCapabilities(capMap)
      setModelContextWindows(ctxMap)

      const ftMap: Record<string, FreeTierKind> = {}
      for (const s of ftStatuses) {
        ftMap[s.provider_instance] = s.free_tier
      }
      setFreeTierKinds(ftMap)
    } catch (pricingError) {
      console.error("Failed to load pricing/free tier data:", pricingError)
    }
  }

  // -------------------------------------------------------------------------
  // RouteLLM status + download events
  // -------------------------------------------------------------------------

  useEffect(() => {
    const loadRouteLLMStatus = async () => {
      try {
        const status = await invoke<RouteLLMStatus>("routellm_get_status")
        setRoutellmStatus(status)
        if (status.state === 'downloading') {
          setIsDownloading(true)
        }
      } catch (error) {
        console.error("Failed to load RouteLLM status:", error)
      }
    }

    loadRouteLLMStatus()

    const lProgress = listenSafe("routellm-download-progress", (event: any) => {
      const { progress } = event.payload
      setDownloadProgress(progress * 100)
    })

    const lComplete = listenSafe("routellm-download-complete", () => {
      setIsDownloading(false)
      setDownloadProgress(100)
      loadRouteLLMStatus()
      toast.success("Strong/Weak model downloaded successfully!")
    })

    const lFailed = listenSafe("routellm-download-failed", (event: any) => {
      setIsDownloading(false)
      toast.error(`Download failed: ${event.payload.error}`)
    })

    // Poll status while initializing/downloading
    const interval = setInterval(() => {
      if (routellmStatus?.state === 'initializing' || routellmStatus?.state === 'downloading') {
        loadRouteLLMStatus()
      }
    }, 1000)

    return () => {
      lProgress.cleanup()
      lComplete.cleanup()
      lFailed.cleanup()
      clearInterval(interval)
    }
  }, [routellmStatus?.state])

  const isRouteLLMDownloaded =
    routellmStatus?.state !== 'not_downloaded' &&
    routellmStatus?.state !== 'downloading' &&
    !isDownloading

  // -------------------------------------------------------------------------
  // Debounced strategy update
  // -------------------------------------------------------------------------

  const updateStrategy = useCallback((updates: Partial<StrategyConfig>) => {
    if (!strategy) return

    pendingUpdatesRef.current = { ...pendingUpdatesRef.current, ...updates }
    setStrategy(prev => prev ? { ...prev, ...updates } : null)

    if (updateTimeoutRef.current) clearTimeout(updateTimeoutRef.current)
    updateTimeoutRef.current = setTimeout(async () => {
      const pending = pendingUpdatesRef.current
      pendingUpdatesRef.current = null
      if (!pending) return

      setSaving(true)
      try {
        await invoke("update_strategy", {
          strategyId: strategy.id,
          name: null,
          allowedModels: pending.allowed_models || null,
          autoConfig: pending.auto_config !== undefined ? pending.auto_config : null,
          rateLimits: pending.rate_limits || null,
          freeTierOnly: pending.free_tier_only ?? null,
          freeTierFallback: pending.free_tier_fallback ?? null,
        })
        onUpdate()
      } catch (error) {
        console.error("Failed to update strategy:", error)
      } finally {
        setSaving(false)
      }
    }, 300)
  }, [strategy, onUpdate])

  // -------------------------------------------------------------------------
  // Model change handler — keeps allowed_models + auto_config in sync
  // -------------------------------------------------------------------------

  const handleModelsChange = useCallback((
    strong: [string, string][],
    weak: [string, string][]
  ) => {
    if (!strategy || !strategy.auto_config) return

    const allEnabled = [...strong, ...weak]
    updateStrategy({
      allowed_models: {
        selected_all: false,
        selected_providers: [],
        selected_models: allEnabled,
      },
      auto_config: {
        ...strategy.auto_config,
        prioritized_models: strong,
        routellm_config: strategy.auto_config.routellm_config
          ? { ...strategy.auto_config.routellm_config, weak_models: weak }
          : null,
      },
    })
  }, [strategy, updateStrategy])

  // -------------------------------------------------------------------------
  // Auto-router permission handler
  // -------------------------------------------------------------------------

  const handleAutoRouterPermissionChange = useCallback((permission: PermissionState) => {
    if (!strategy?.auto_config) return
    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        permission,
      },
    })
  }, [strategy, updateStrategy])

  // -------------------------------------------------------------------------
  // Rate limits (debounced separately at 500ms)
  // -------------------------------------------------------------------------

  const handleRateLimitsChange = useCallback((limits: StrategyRateLimit[]) => {
    if (!strategy) return

    setStrategy(prev => prev ? { ...prev, rate_limits: limits } : null)

    if (rateLimitTimeoutRef.current) clearTimeout(rateLimitTimeoutRef.current)
    rateLimitTimeoutRef.current = setTimeout(async () => {
      setSavingRateLimits(true)
      try {
        await invoke("update_strategy", {
          strategyId: strategy.id,
          name: null,
          allowedModels: null,
          autoConfig: null,
          rateLimits: limits,
          freeTierOnly: null,
          freeTierFallback: null,
        })
        toast.success("Rate limits updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update rate limits:", error)
        toast.error("Failed to update rate limits")
      } finally {
        setSavingRateLimits(false)
      }
    }, 500)
  }, [strategy, onUpdate])

  // -------------------------------------------------------------------------
  // Free tier handlers
  // -------------------------------------------------------------------------

  const handleFreeTierToggle = useCallback(async (enabled: boolean) => {
    if (!strategy) return

    setStrategy(prev => prev ? { ...prev, free_tier_only: enabled } : null)

    try {
      await invoke("update_strategy", {
        strategyId: strategy.id,
        name: null,
        allowedModels: null,
        autoConfig: null,
        rateLimits: null,
        freeTierOnly: enabled,
        freeTierFallback: null,
      })
      toast.success(enabled ? "Free-tier mode enabled" : "Free-tier mode disabled")
      onUpdate()
    } catch (error) {
      console.error("Failed to update free tier mode:", error)
      toast.error("Failed to update free tier mode")
      loadData()
    }
  }, [strategy, onUpdate])

  const handleFreeTierFallbackChange = useCallback(async (value: string) => {
    if (!strategy) return
    const fallback = value as 'off' | 'ask' | 'allow'

    setStrategy(prev => prev ? { ...prev, free_tier_fallback: fallback } : null)

    try {
      await invoke("update_strategy", {
        strategyId: strategy.id,
        name: null,
        allowedModels: null,
        autoConfig: null,
        rateLimits: null,
        freeTierOnly: null,
        freeTierFallback: fallback,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to update free tier fallback:", error)
      toast.error("Failed to update paid fallback setting")
      loadData()
    }
  }, [strategy, onUpdate])

  // -------------------------------------------------------------------------
  // RouteLLM handlers
  // -------------------------------------------------------------------------

  const handleRouteLLMToggle = useCallback((enabled: boolean) => {
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
  }, [strategy, updateStrategy])

  const handleThresholdChange = useCallback((threshold: number) => {
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
  }, [strategy, updateStrategy])

  const handleDownload = async () => {
    setIsDownloading(true)
    setDownloadProgress(0)
    try {
      await invoke("routellm_download_models")
    } catch (error: any) {
      console.error("Failed to start download:", error)
      toast.error(`Download failed: ${error.message || error}`)
      setIsDownloading(false)
    }
  }

  // -------------------------------------------------------------------------
  // Loading / error states
  // -------------------------------------------------------------------------

  if (loading) {
    return (
      <div className="space-y-4">
        <Card>
          <CardContent className="py-8">
            <div className="flex items-center justify-center">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  if (!strategy) {
    return (
      <div className="space-y-4">
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

  // -------------------------------------------------------------------------
  // Derived values
  // -------------------------------------------------------------------------

  const autoConfig = strategy.auto_config
  const routellmConfig = autoConfig?.routellm_config

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  return (
    <div className="space-y-4">
      {/* Section 1: Model Selection */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="p-2 rounded-lg bg-primary/10">
                <Bot className="h-4 w-4 text-primary" />
              </div>
              <div>
                <CardTitle className="text-base">Model Selection</CardTitle>
                <CardDescription>
                  Select and prioritize models. Enabled models appear in the API model list and are used for auto-routing in priority order.
                </CardDescription>
                {/* Show loading indicator for providers */}
                {!isFullyLoaded && loadingProviders.size > 0 && (
                  <div className="flex items-center gap-2 text-xs text-muted-foreground mt-1">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    <span>Loading models from {loadingProviders.size} provider{loadingProviders.size > 1 ? 's' : ''}...</span>
                  </div>
                )}
              </div>
            </div>
            <PermissionStateButton
              value={autoConfig?.permission || 'allow'}
              onChange={handleAutoRouterPermissionChange}
              disabled={saving}
              size="sm"
            />
          </div>
        </CardHeader>
        <CardContent>
          <ThreeZoneModelSelector
            availableModels={models}
            enabledModels={autoConfig?.prioritized_models || []}
            weakModels={routellmConfig?.weak_models || []}
            showWeakZone={routellmConfig?.enabled ?? false}
            onEnabledModelsChange={(strong) => handleModelsChange(strong, routellmConfig?.weak_models || [])}
            onWeakModelsChange={(weak) => handleModelsChange(autoConfig?.prioritized_models || [], weak)}
            disabled={saving}
            modelPricing={modelPricing}
            modelParamCounts={modelParamCounts}
            freeTierKinds={freeTierKinds}
            modelCapabilities={modelCapabilities}
            modelContextWindows={modelContextWindows}
          />
        </CardContent>
      </Card>

      {/* Section 2: Rate Limits */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Gauge className="h-4 w-4" />
            Rate Limits
          </CardTitle>
          <CardDescription className="flex items-center gap-1">
            Set usage limits to control costs and prevent abuse
            <InfoTooltip content="Maximum number of requests this client can make within the configured time window. Applies across all models." />
          </CardDescription>
        </CardHeader>
        <CardContent>
          <RateLimitEditor
            limits={strategy.rate_limits || []}
            onChange={handleRateLimitsChange}
            disabled={savingRateLimits}
          />
        </CardContent>
      </Card>

      {/* Section 3: Free-Tier Mode */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="flex items-center gap-2 text-base">
              <Coins className="h-4 w-4" />
              Free-Tier Mode
            </CardTitle>
            <InfoTooltip content="Restricts this client to models marked as free. Paid models are hidden from the model list entirely.">
              <Switch
                checked={strategy.free_tier_only ?? false}
                onCheckedChange={handleFreeTierToggle}
              />
            </InfoTooltip>
          </div>
          <CardDescription>
            Restrict model usage to Free and Free-tier only. For Auto Routing, will move onto using next model if model usage limits are reached.
          </CardDescription>
        </CardHeader>
        {strategy.free_tier_only && (
          <CardContent className="pt-0">
            <div className="border-t pt-3">
              {/* Paid Fallback buttons: allow/ask/off */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm font-medium flex items-center gap-1">
                    Paid Fallback
                    <InfoTooltip content="Controls behavior when free-tier rate limits are exhausted. Allow: silently routes to paid models. Ask: prompts user via firewall. Off: returns an error." />
                  </span>
                  <p className="text-xs text-muted-foreground mt-0.5">
                    What to do when free-tier usage is depleted
                  </p>
                </div>
                <div className="inline-flex rounded-md border border-border bg-muted/50">
                  {(['allow', 'ask', 'off'] as const).map((value) => {
                    const isActive = (strategy.free_tier_fallback ?? 'off') === value
                    const config = {
                      allow: { label: 'Allow', activeClass: 'bg-emerald-500 text-white' },
                      ask: { label: 'Ask', activeClass: 'bg-amber-500 text-white' },
                      off: { label: 'Off', activeClass: 'bg-zinc-500 text-white' },
                    }[value]
                    return (
                      <button
                        key={value}
                        onClick={() => handleFreeTierFallbackChange(value)}
                        className={`px-3 py-1 text-sm font-medium transition-colors ${
                          isActive ? config.activeClass : 'text-muted-foreground hover:text-foreground hover:bg-muted'
                        } ${value === 'allow' ? 'rounded-l-md' : ''} ${value === 'off' ? 'rounded-r-md' : ''}`}
                      >
                        {config.label}
                      </button>
                    )
                  })}
                </div>
              </div>
              <div className="border-t pt-3 mt-3 flex items-center justify-between">
                <div>
                  <span className="text-sm font-medium">Approval Popup Preview</span>
                  <p className="text-xs text-muted-foreground mt-0.5">
                    Preview the popup shown when free-tier usage is depleted
                  </p>
                </div>
                <SamplePopupButton popupType="free_tier_fallback" />
              </div>
            </div>
          </CardContent>
        )}
      </Card>

      {/* Section 4: Weak Models (RouteLLM) */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="p-2 rounded-lg bg-purple-500/10">
                <Brain className="h-4 w-4 text-purple-500" />
              </div>
              <div>
                <CardTitle className="text-base flex items-center gap-2">
                  Weak Models
                  <ExperimentalBadge />
                </CardTitle>
                <CardDescription>
                  Use weaker models for simpler prompts for faster and cheaper results.
                </CardDescription>
              </div>
            </div>
            <InfoTooltip content="Routes simple requests to cheaper models automatically. Uses a classifier to estimate request complexity and pick the appropriate tier.">
              <Switch
                checked={routellmConfig?.enabled ?? false}
                onCheckedChange={handleRouteLLMToggle}
                disabled={saving || !isRouteLLMDownloaded}
              />
            </InfoTooltip>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {!routellmConfig?.enabled ? (
            <div className="space-y-4">
              {/* Resource Requirements */}
              <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-600/50">
                <div className="flex items-start gap-2">
                  <MessageSquareWarning className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
                  <div className="text-xs text-amber-900 dark:text-amber-300">
                    <p className="font-medium mb-2">Resource Requirements</p>
                    <div className="grid grid-cols-2 gap-x-4 gap-y-1">
                      <span>Disk Space:</span><span>{ROUTELLM_REQUIREMENTS.DISK_GB} GB</span>
                      <span>Memory:</span><span>{ROUTELLM_REQUIREMENTS.MEMORY_GB} GB</span>
                      <span>Cold Start:</span><span>{ROUTELLM_REQUIREMENTS.COLD_START_SECS}s</span>
                      <span>Per-request:</span><span>{ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms</span>
                    </div>
                  </div>
                </div>
              </div>
              {/* Download section */}
              {!isRouteLLMDownloaded && (
                <div className="space-y-3">
                  {isDownloading ? (
                    <div className="space-y-2">
                      <div className="flex justify-between text-xs text-muted-foreground">
                        <span>Downloading Strong/Weak model...</span>
                        <span>{downloadProgress.toFixed(0)}%</span>
                      </div>
                      <Progress value={downloadProgress} className="h-1.5" />
                    </div>
                  ) : (
                    <>
                      <p className="text-xs text-muted-foreground">
                        Download the Strong/Weak model to enable intelligent selection between strong and weak models.
                      </p>
                      <Button onClick={handleDownload} size="sm" variant="outline" className="w-full">
                        <Download className="h-3 w-3 mr-2" />
                        Download Model ({ROUTELLM_REQUIREMENTS.DISK_GB} GB)
                      </Button>
                    </>
                  )}
                </div>
              )}
            </div>
          ) : (
            <>
              {/* Threshold Selector */}
              <div className="space-y-1">
                <div className="flex items-center gap-1">
                  <span className="text-sm font-medium">Routing Threshold</span>
                  <InfoTooltip content="Lower values route more requests to weak models (saves cost, lower quality). Higher values route more to strong models (higher cost, better quality)." />
                </div>
                <ThresholdSelector
                  value={routellmConfig.threshold}
                  onChange={handleThresholdChange}
                />
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
