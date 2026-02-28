import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { CheckCircle, XCircle, AlertCircle, Plus, Loader2, RefreshCw, FlaskConical, Grid, Settings, ArrowLeft, Eye, EyeOff, Coins, Pencil, RotateCcw } from "lucide-react"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { ProvidersIcon } from "@/components/icons/category-icons"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/Toggle"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { Input } from "@/components/ui/Input"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import ProviderForm, { ProviderType } from "@/components/ProviderForm"
import ProviderIcon from "@/components/ProviderIcon"
import { cn } from "@/lib/utils"
import type { FreeTierKind, ProviderFreeTierStatus } from "@/types/tauri-commands"
import { ModelPricingBadge } from "@/components/shared/model-pricing-badge"

const FREE_TIER_LABELS: Record<string, string> = {
  none: 'No Free Tier',
  always_free_local: 'Always Free (Local)',
  subscription: 'Subscription',
  rate_limited_free: 'Rate Limited',
  credit_based: 'Credit Based',
  free_models_only: 'Free Models Only',
}

const FREE_TIER_DESCRIPTIONS: Record<string, string> = {
  none: 'This provider has no free tier. All requests are treated as paid. The provider is always skipped when a strategy has free-tier mode enabled.',
  always_free_local: 'Local or self-hosted provider with no external billing. Always treated as free with no usage limits tracked. Use for Ollama, LM Studio, or other locally-running models.',
  subscription: 'Included in an existing subscription (e.g. GitHub Copilot). Always treated as free. No usage counters are tracked.',
  rate_limited_free: 'Free access within rate limits (requests per minute/day, tokens per minute/day, monthly caps). The router tracks usage against these limits and skips the provider when any limit is reached. Used by Gemini, Groq, Cerebras, Mistral, and Cohere.',
  credit_based: 'Dollar-budget credits that are consumed per request. The router estimates cost from token usage and compares against the budget. When credits run out, the provider is skipped. Used by OpenRouter, xAI, DeepInfra, and Perplexity.',
  free_models_only: 'Only specific models from this provider are free. The router checks each model ID against the configured patterns. Models that don\'t match are treated as paid and skipped in free-tier mode. Used by Together AI.',
}

interface Provider {
  instance_name: string
  provider_type: string
  enabled: boolean
  base_url?: string
  config?: Record<string, string>
}

export interface HealthStatus {
  status: "pending" | "healthy" | "degraded" | "unhealthy" | "disabled"
  latency_ms?: number
  error?: string
}

export interface HealthCheckEvent {
  provider_name: string
  status: string
  latency_ms?: number
  error_message?: string
}

interface ModelInfo {
  id: string
  name: string
  provider: string
  parameter_count?: number
  context_window: number
  supports_streaming: boolean
  capabilities: string[]
}

interface DetailedModel {
  model_id: string
  provider_instance: string
  provider_type: string
  capabilities: string[]
  context_window: number
  supports_streaming: boolean
  input_price_per_million?: number
  output_price_per_million?: number
  parameter_count?: string
  pricing_source?: string
}

interface ProvidersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  healthStatus: Record<string, HealthStatus>
  onHealthInit: (providerNames: string[]) => void
  onRefreshHealth: (instanceName: string) => Promise<void>
  initialAddProviderType?: string | null
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ProvidersPanel({
  selectedId,
  onSelect,
  healthStatus,
  onHealthInit,
  onRefreshHealth,
  initialAddProviderType,
  onViewChange,
}: ProvidersPanelProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")
  const [models, setModels] = useState<ModelInfo[]>([])
  const [modelsLoading, setModelsLoading] = useState(false)

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false)
  const [providerToDelete, setProviderToDelete] = useState<Provider | null>(null)

  // Detailed models for the Models tab
  const [detailedModels, setDetailedModels] = useState<DetailedModel[]>([])
  const [detailedModelsLoading, setDetailedModelsLoading] = useState(false)

  // Detail tab state
  const [detailTab, setDetailTab] = useState("info")

  // Free tier state
  const [freeTierStatus, setFreeTierStatus] = useState<Record<string, ProviderFreeTierStatus>>({})
  const [freeTierOverrideEditing, setFreeTierOverrideEditing] = useState(false)
  const [freeTierOverrideKind, setFreeTierOverrideKind] = useState<FreeTierKind>({ kind: 'none' })
  const [setUsageDialogOpen, setSetUsageDialogOpen] = useState(false)
  const [setUsageValues, setSetUsageValues] = useState<{
    creditUsedUsd: string
    creditRemainingUsd: string
    dailyRequests: string
    monthlyRequests: string
    monthlyTokens: string
  }>({ creditUsedUsd: '', creditRemainingUsd: '', dailyRequests: '', monthlyRequests: '', monthlyTokens: '' })

  // Create form state
  const [dialogPage, setDialogPage] = useState<"select" | "configure">("select")
  const [createTab, setCreateTab] = useState<"templates" | "custom">("templates")
  const [selectedProviderType, setSelectedProviderType] = useState<string>("")
  const [isSubmitting, setIsSubmitting] = useState(false)

  // Handle initial add provider type from navigation
  useEffect(() => {
    if (initialAddProviderType && providerTypes.length > 0) {
      const typeExists = providerTypes.some(t => t.provider_type === initialAddProviderType)
      if (typeExists) {
        setSelectedProviderType(initialAddProviderType)
        setDialogPage("configure")
        setCreateDialogOpen(true)
      }
    }
  }, [initialAddProviderType, providerTypes])

  useEffect(() => {
    loadProviders()
    loadProviderTypes()

    // Listen for provider changes
    const unsubProviders = listen("providers-changed", () => {
      loadProvidersOnly()
    })

    return () => {
      unsubProviders.then((fn) => fn())
    }
  }, [])

  // Reset detail tab when a different provider is selected (not during rename)
  const skipTabResetRef = useRef(false)
  useEffect(() => {
    if (skipTabResetRef.current) {
      skipTabResetRef.current = false
      return
    }
    setDetailTab("info")
  }, [selectedId])

  // Load providers and initialize health checks (only on first load)
  const loadProviders = async () => {
    try {
      setLoading(true)
      const providerList = await invoke<Provider[]>("list_provider_instances")
      setProviders(providerList)

      // Initialize health checks (parent will only do this once)
      onHealthInit(providerList.map(p => p.instance_name))
    } catch (error) {
      console.error("Failed to load providers:", error)
    } finally {
      setLoading(false)
    }
  }

  // Load providers without triggering health checks (for refreshes/updates)
  const loadProvidersOnly = async () => {
    try {
      const providerList = await invoke<Provider[]>("list_provider_instances")
      setProviders(providerList)
    } catch (error) {
      console.error("Failed to load providers:", error)
    }
  }

  const loadProviderTypes = async () => {
    try {
      const types = await invoke<ProviderType[]>("list_provider_types")
      setProviderTypes(types)
    } catch (error) {
      console.error("Failed to load provider types:", error)
    }
  }

  // Load free tier status for a provider
  const loadFreeTierStatus = useCallback(async (instanceName: string) => {
    try {
      const statuses = await invoke<any[]>("get_free_tier_status")
      const status = statuses.find((s: any) => s.provider_instance === instanceName)
      if (status) {
        setFreeTierStatus(prev => ({ ...prev, [instanceName]: status }))
      }
    } catch (error) {
      console.error("Failed to load free tier status:", error)
    }
  }, [])

  const saveFreeTierOverride = useCallback(async (instanceName: string, freeTier: FreeTierKind | null) => {
    try {
      await invoke("set_provider_free_tier", {
        providerInstance: instanceName,
        freeTier,
      })
      toast.success(freeTier ? "Free tier override saved" : "Reset to provider default")
      setFreeTierOverrideEditing(false)
      loadFreeTierStatus(instanceName)
    } catch (error) {
      console.error("Failed to save free tier override:", error)
      toast.error("Failed to save free tier override")
    }
  }, [loadFreeTierStatus])

  const saveSetUsage = useCallback(async (instanceName: string, kind: string) => {
    try {
      const params: Record<string, unknown> = { providerInstance: instanceName }
      if (kind === 'credit_based') {
        params.creditUsedUsd = setUsageValues.creditUsedUsd ? parseFloat(setUsageValues.creditUsedUsd) : null
        params.creditRemainingUsd = setUsageValues.creditRemainingUsd ? parseFloat(setUsageValues.creditRemainingUsd) : null
        params.dailyRequests = null
        params.monthlyRequests = null
        params.monthlyTokens = null
      } else {
        params.creditUsedUsd = null
        params.creditRemainingUsd = null
        params.dailyRequests = setUsageValues.dailyRequests ? parseInt(setUsageValues.dailyRequests) : null
        params.monthlyRequests = setUsageValues.monthlyRequests ? parseInt(setUsageValues.monthlyRequests) : null
        params.monthlyTokens = setUsageValues.monthlyTokens ? parseInt(setUsageValues.monthlyTokens) : null
      }
      await invoke("set_provider_free_tier_usage", params)
      toast.success("Usage updated")
      setSetUsageDialogOpen(false)
      loadFreeTierStatus(instanceName)
    } catch (error) {
      console.error("Failed to set usage:", error)
      toast.error("Failed to set usage")
    }
  }, [setUsageValues, loadFreeTierStatus])

  const loadModels = useCallback(async (instanceName: string) => {
    setModelsLoading(true)
    try {
      const modelList = await invoke<ModelInfo[]>("list_provider_models", { instanceName })
      setModels(modelList)
    } catch (error) {
      console.error("Failed to load models:", error)
      setModels([])
    } finally {
      setModelsLoading(false)
    }
  }, [])

  const loadDetailedModels = useCallback(async (instanceName: string) => {
    setDetailedModelsLoading(true)
    try {
      const allModels = await invoke<DetailedModel[]>("list_all_models_detailed")
      setDetailedModels(allModels.filter(m => m.provider_instance === instanceName))
    } catch (error) {
      console.error("Failed to load detailed models:", error)
      setDetailedModels([])
    } finally {
      setDetailedModelsLoading(false)
    }
  }, [])

  // Load models when a provider is selected
  useEffect(() => {
    if (selectedId) {
      loadModels(selectedId)
    } else {
      setModels([])
    }
  }, [selectedId, loadModels])

  const handleToggle = async (provider: Provider) => {
    try {
      await invoke("set_provider_enabled", {
        instanceName: provider.instance_name,
        enabled: !provider.enabled,
      })
      toast.success(`Provider ${provider.enabled ? "disabled" : "enabled"}`)
      loadProvidersOnly()
      // Trigger health check to update status to disabled/enabled
      onRefreshHealth(provider.instance_name)
    } catch (error) {
      toast.error("Failed to update provider")
    }
  }

  const handleDelete = async () => {
    if (!providerToDelete) return
    try {
      await invoke("remove_provider_instance", {
        instanceName: providerToDelete.instance_name,
      })
      toast.success("Provider deleted")
      if (selectedId === providerToDelete.instance_name) {
        onSelect(null)
      }
      loadProvidersOnly()
    } catch (error) {
      toast.error("Failed to delete provider")
    } finally {
      setProviderToDelete(null)
      setDeleteDialogOpen(false)
    }
  }

  const handleCreateProvider = async (instanceName: string, config: Record<string, string>) => {
    setIsSubmitting(true)
    try {
      await invoke("create_provider_instance", {
        instanceName,
        providerType: selectedProviderType,
        config,
      })
      toast.success("Provider created")
      setCreateDialogOpen(false)
      setSelectedProviderType("")
      setDialogPage("select")
      setCreateTab("templates")
      await loadProvidersOnly()
      onSelect(instanceName)
      // Trigger health check for the new provider
      onRefreshHealth(instanceName)
    } catch (error) {
      toast.error(`Failed to create provider: ${error}`)
    } finally {
      setIsSubmitting(false)
    }
  }

  const filteredProviders = providers.filter((p) =>
    p.instance_name.toLowerCase().includes(search.toLowerCase()) ||
    p.provider_type.toLowerCase().includes(search.toLowerCase())
  )

  const selectedProvider = providers.find((p) => p.instance_name === selectedId)
  const selectedTypeForCreate = providerTypes.find((t) => t.provider_type === selectedProviderType)
  const selectedTypeForEdit = selectedProvider
    ? providerTypes.find((t) => t.provider_type === selectedProvider.provider_type)
    : null

  // --- Inline edit state for settings tab ---
  const [editName, setEditName] = useState("")
  const [editConfig, setEditConfig] = useState<Record<string, string>>({})
  const [configLoading, setConfigLoading] = useState(false)
  const [visibleFields, setVisibleFields] = useState<Set<string>>(new Set())

  // Debounce refs
  const renameTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const configTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingConfigRef = useRef<Record<string, string> | null>(null)

  // Cleanup debounce timeouts on unmount
  useEffect(() => {
    return () => {
      if (renameTimeoutRef.current) clearTimeout(renameTimeoutRef.current)
      if (configTimeoutRef.current) clearTimeout(configTimeoutRef.current)
    }
  }, [])

  // Load config when switching to settings tab or selecting a different provider
  useEffect(() => {
    if (detailTab === "settings" && selectedId) {
      setConfigLoading(true)
      setVisibleFields(new Set())
      setEditName(selectedId)
      invoke<Record<string, string>>("get_provider_config", { instanceName: selectedId })
        .then((config) => setEditConfig(config))
        .catch(() => setEditConfig({}))
        .finally(() => setConfigLoading(false))
    }
  }, [detailTab, selectedId])

  // Debounced rename
  const debouncedRename = useCallback((newName: string) => {
    if (!selectedProvider) return
    if (renameTimeoutRef.current) clearTimeout(renameTimeoutRef.current)
    renameTimeoutRef.current = setTimeout(async () => {
      const trimmed = newName.trim()
      if (!trimmed || trimmed === selectedProvider.instance_name) return
      try {
        await invoke("rename_provider_instance", {
          instanceName: selectedProvider.instance_name,
          newName: trimmed,
        })
        toast.success("Provider renamed")
        await loadProvidersOnly()
        skipTabResetRef.current = true
        onSelect(trimmed)
      } catch (error) {
        toast.error(`Failed to rename: ${error}`)
      }
    }, 500)
  }, [selectedProvider, onSelect])

  // Debounced config update
  const debouncedConfigUpdate = useCallback((updatedConfig: Record<string, string>) => {
    if (!selectedProvider) return
    pendingConfigRef.current = updatedConfig
    if (configTimeoutRef.current) clearTimeout(configTimeoutRef.current)
    configTimeoutRef.current = setTimeout(async () => {
      const config = pendingConfigRef.current
      pendingConfigRef.current = null
      if (!config) return
      try {
        await invoke("update_provider_instance", {
          instanceName: selectedProvider.instance_name,
          providerType: selectedProvider.provider_type,
          config,
        })
        toast.success("Provider updated")
        loadProvidersOnly()
      } catch (error) {
        toast.error(`Failed to update provider: ${error}`)
      }
    }, 500)
  }, [selectedProvider])

  const handleConfigFieldChange = (key: string, value: string) => {
    const updated = { ...editConfig, [key]: value }
    setEditConfig(updated)
    debouncedConfigUpdate(updated)
  }

  const toggleFieldVisibility = (key: string) => {
    setVisibleFields((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  // Generate a clean default instance name like "Perplexity", "Perplexity (2)", etc.
  const generateDefaultName = (displayName: string): string => {
    const existingNames = new Set(providers.map(p => p.instance_name))
    if (!existingNames.has(displayName)) return displayName
    let i = 2
    while (existingNames.has(`${displayName} (${i})`)) i++
    return `${displayName} (${i})`
  }

  return (
    <>
      <ResizablePanelGroup direction="horizontal" className="h-full rounded-lg border">
        {/* List Panel */}
        <ResizablePanel defaultSize={35} minSize={25}>
          <div className="flex flex-col h-full">
            <div className="p-4 border-b">
              <div className="flex items-center gap-2">
                <Input
                  placeholder="Search providers..."
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="flex-1"
                />
                <Button size="icon" onClick={() => setCreateDialogOpen(true)}>
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <div className="p-2 space-y-1">
                {loading ? (
                  <p className="text-sm text-muted-foreground p-4">Loading...</p>
                ) : filteredProviders.length === 0 ? (
                  <p className="text-sm text-muted-foreground p-4">No providers found</p>
                ) : (
                  filteredProviders.map((provider) => {
                    const health = healthStatus[provider.instance_name]
                    const formatLatency = (ms?: number) => {
                      if (ms == null) return ""
                      return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
                    }
                    return (
                      <div
                        key={provider.instance_name}
                        onClick={() => onSelect(provider.instance_name)}
                        className={cn(
                          "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                          selectedId === provider.instance_name
                            ? "bg-accent"
                            : "hover:bg-muted"
                        )}
                      >
                        <ProviderIcon providerId={provider.provider_type.toLowerCase()} size={20} />
                        <div className="flex-1 min-w-0">
                          <p className="font-medium truncate">{provider.instance_name}</p>
                          <p className="text-xs text-muted-foreground">{provider.provider_type}</p>
                        </div>
                        <div className="flex items-center gap-2">
                          {health && health.latency_ms != null && health.status !== "pending" && (
                            <span className="text-xs text-muted-foreground">
                              {formatLatency(health.latency_ms)}
                            </span>
                          )}
                          {health?.status === "pending" ? (
                            <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
                          ) : (
                            <div
                              className={cn(
                                "h-2 w-2 rounded-full",
                                !health && "bg-gray-400",
                                health?.status === "healthy" && "bg-green-500",
                                health?.status === "degraded" && "bg-yellow-500",
                                health?.status === "unhealthy" && "bg-red-500",
                                health?.status === "disabled" && "bg-gray-400"
                              )}
                              title={
                                health?.status === "healthy"
                                  ? health.latency_ms != null
                                    ? `Healthy (${formatLatency(health.latency_ms)})`
                                    : "Healthy"
                                  : health?.status === "degraded"
                                  ? `Degraded: ${health.error}`
                                  : health?.status === "disabled"
                                  ? "Disabled"
                                  : health?.error
                              }
                            />
                          )}
                        </div>
                      </div>
                    )
                  })
                )}
              </div>
            </ScrollArea>
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Detail Panel */}
        <ResizablePanel defaultSize={65}>
          {selectedProvider ? (
            <ScrollArea className="h-full">
              <div className="p-6 space-y-6">
                <div className="flex items-start justify-between">
                  <div>
                    <h2 className="text-xl font-bold">{selectedProvider.instance_name}</h2>
                    <p className="text-sm text-muted-foreground">
                      {selectedProvider.provider_type}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    {onViewChange && selectedProvider.enabled && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => onViewChange("try-it-out", `llm/init/direct/${selectedProvider.instance_name}`)}
                      >
                        <FlaskConical className="h-4 w-4 mr-1" />
                        Try It Out
                      </Button>
                    )}
                  </div>
                </div>

                <Tabs value={detailTab} onValueChange={setDetailTab}>
                  <TabsList>
                    <TabsTrigger value="info">Info</TabsTrigger>
                    <TabsTrigger value="models" onClick={() => { loadDetailedModels(selectedProvider.instance_name); loadFreeTierStatus(selectedProvider.instance_name) }}>Models</TabsTrigger>
                    <TabsTrigger value="free-tier" onClick={() => loadFreeTierStatus(selectedProvider.instance_name)}>Free Tier</TabsTrigger>
                    <TabsTrigger value="settings">Settings</TabsTrigger>
                  </TabsList>

                  <TabsContent value="info">
                    <div className="space-y-6">
                      {/* Health Status */}
                      <Card>
                        <CardHeader className="pb-3">
                          <div className="flex items-center justify-between">
                            <CardTitle className="text-sm">Health Status</CardTitle>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-6 w-6"
                              onClick={() => onRefreshHealth(selectedProvider.instance_name)}
                              disabled={healthStatus[selectedProvider.instance_name]?.status === "pending"}
                            >
                              <RefreshCw className={cn(
                                "h-3 w-3",
                                healthStatus[selectedProvider.instance_name]?.status === "pending" && "animate-spin"
                              )} />
                            </Button>
                          </div>
                        </CardHeader>
                        <CardContent>
                          {(() => {
                            const health = healthStatus[selectedProvider.instance_name]
                            const formatLatency = (ms?: number) => {
                              if (ms == null) return ""
                              return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
                            }

                            if (!health || health.status === "pending") {
                              return (
                                <div className="flex items-center gap-2 text-muted-foreground">
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                  <span>Checking health...</span>
                                </div>
                              )
                            }

                            if (health.status === "healthy") {
                              return (
                                <div className="flex items-center gap-2 text-green-600">
                                  <CheckCircle className="h-4 w-4" />
                                  <span>Healthy</span>
                                  {health.latency_ms != null && (
                                    <span className="text-muted-foreground">
                                      ({formatLatency(health.latency_ms)})
                                    </span>
                                  )}
                                </div>
                              )
                            }

                            if (health.status === "degraded") {
                              return (
                                <div className="space-y-1">
                                  <div className="flex items-center gap-2 text-yellow-600">
                                    <AlertCircle className="h-4 w-4 shrink-0" />
                                    <span>Degraded</span>
                                    {health.latency_ms != null && (
                                      <span className="text-muted-foreground">
                                        ({formatLatency(health.latency_ms)})
                                      </span>
                                    )}
                                  </div>
                                  {health.error && (
                                    <pre className="text-xs text-muted-foreground overflow-auto max-h-32 whitespace-pre-wrap break-all bg-muted/50 rounded p-2">{health.error}</pre>
                                  )}
                                </div>
                              )
                            }

                            if (health.status === "disabled") {
                              return (
                                <div className="flex items-center gap-2 text-muted-foreground">
                                  <XCircle className="h-4 w-4" />
                                  <span>Disabled</span>
                                </div>
                              )
                            }

                            return (
                              <div className="space-y-1">
                                <div className="flex items-center gap-2 text-red-600">
                                  <XCircle className="h-4 w-4 shrink-0" />
                                  <span>Unhealthy</span>
                                </div>
                                {health.error && (
                                  <pre className="text-xs text-muted-foreground overflow-auto max-h-32 whitespace-pre-wrap break-all bg-muted/50 rounded p-2">{health.error}</pre>
                                )}
                              </div>
                            )
                          })()}
                        </CardContent>
                      </Card>

                      {/* Models List */}
                      <Card>
                        <CardHeader className="pb-3">
                          <CardTitle className="text-sm">
                            Models {!modelsLoading && models.length > 0 && (
                              <span className="text-muted-foreground font-normal">({models.length})</span>
                            )}
                          </CardTitle>
                        </CardHeader>
                        <CardContent>
                          {modelsLoading ? (
                            <div className="flex items-center gap-2 text-muted-foreground">
                              <Loader2 className="h-4 w-4 animate-spin" />
                              <span>Loading models...</span>
                            </div>
                          ) : models.length === 0 ? (
                            <p className="text-sm text-muted-foreground">No models available</p>
                          ) : (
                            <div className="space-y-2">
                              {models.map((model) => (
                                <div
                                  key={model.id}
                                  className="flex items-center justify-between p-2 rounded-md bg-muted/50"
                                >
                                  <div className="min-w-0 flex-1">
                                    <p className="font-medium text-sm truncate">{model.name || model.id}</p>
                                    <p className="text-xs text-muted-foreground truncate">{model.id}</p>
                                  </div>
                                  <div className="flex items-center gap-2 ml-2">
                                    {model.context_window > 0 && (
                                      <Badge variant="secondary" className="text-xs whitespace-nowrap">
                                        {model.context_window >= 1000000
                                          ? `${(model.context_window / 1000000).toFixed(1)}M`
                                          : model.context_window >= 1000
                                          ? `${Math.round(model.context_window / 1000)}k`
                                          : model.context_window} ctx
                                      </Badge>
                                    )}
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>

                  <TabsContent value="models">
                    <div className="space-y-4">
                      <div className="flex items-center justify-between">
                        <p className="text-sm text-muted-foreground">
                          {detailedModelsLoading ? "Loading..." : `${detailedModels.length} model${detailedModels.length !== 1 ? "s" : ""}`}
                        </p>
                      </div>
                      {detailedModelsLoading ? (
                        <div className="flex items-center gap-2 text-muted-foreground">
                          <Loader2 className="h-4 w-4 animate-spin" />
                          <span>Loading models...</span>
                        </div>
                      ) : detailedModels.length === 0 ? (
                        <p className="text-sm text-muted-foreground">No models available</p>
                      ) : (
                        <div className="space-y-1">
                          {detailedModels.map((model) => {
                            const ftStatus = freeTierStatus[selectedProvider.instance_name]
                            return (
                              <div
                                key={model.model_id}
                                className="flex items-center justify-between p-3 rounded-md hover:bg-muted cursor-pointer"
                                onClick={() => {
                                  if (onViewChange) {
                                    onViewChange("resources", `models/${model.provider_instance}/${model.model_id}`)
                                  }
                                }}
                              >
                                <div className="min-w-0 flex-1">
                                  <p className="font-medium text-sm truncate">{model.model_id}</p>
                                  {model.parameter_count && (
                                    <p className="text-xs text-muted-foreground">{model.parameter_count}</p>
                                  )}
                                </div>
                                <div className="flex items-center gap-3 ml-2">
                                  <ModelPricingBadge
                                    inputPricePerMillion={model.input_price_per_million}
                                    outputPricePerMillion={model.output_price_per_million}
                                    freeTierKind={ftStatus?.free_tier}
                                  />
                                  {model.context_window > 0 && (
                                    <Badge variant="secondary" className="text-xs whitespace-nowrap">
                                      {model.context_window >= 1000000
                                        ? `${(model.context_window / 1000000).toFixed(1)}M`
                                        : model.context_window >= 1000
                                        ? `${Math.round(model.context_window / 1000)}k`
                                        : model.context_window} ctx
                                    </Badge>
                                  )}
                                </div>
                              </div>
                            )
                          })}
                        </div>
                      )}
                    </div>
                  </TabsContent>

                  <TabsContent value="settings">
                    <div className="space-y-6">
                      {/* Inline Edit Fields */}
                      {selectedTypeForEdit && (
                        <Card>
                          <CardHeader>
                            <CardTitle>Provider Configuration</CardTitle>
                            <CardDescription>
                              Changes are saved automatically
                            </CardDescription>
                          </CardHeader>
                          <CardContent className="space-y-4">
                            {configLoading ? (
                              <div className="flex items-center gap-2 text-muted-foreground">
                                <Loader2 className="h-4 w-4 animate-spin" />
                                <span>Loading configuration...</span>
                              </div>
                            ) : (
                              <>
                                {/* Instance Name */}
                                <div>
                                  <label className="block text-sm font-medium mb-2">Instance Name</label>
                                  <Input
                                    value={editName}
                                    onChange={(e) => {
                                      setEditName(e.target.value)
                                      debouncedRename(e.target.value)
                                    }}
                                    placeholder="e.g., OpenAI, Groq"
                                  />
                                </div>

                                {/* Dynamic Parameter Fields */}
                                {selectedTypeForEdit.setup_parameters
                                  .filter((param) => param.param_type !== "oauth")
                                  .map((param) => {
                                    const isSensitive = param.sensitive
                                    const isVisible = visibleFields.has(param.key)

                                    if (param.param_type === "boolean") {
                                      return (
                                        <div key={param.key} className="flex items-center gap-2">
                                          <input
                                            type="checkbox"
                                            id={`edit-${param.key}`}
                                            checked={editConfig[param.key] === "true"}
                                            onChange={(e) => handleConfigFieldChange(param.key, e.target.checked.toString())}
                                            className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500"
                                          />
                                          <label htmlFor={`edit-${param.key}`} className="text-sm font-medium">
                                            {param.description}
                                          </label>
                                        </div>
                                      )
                                    }

                                    const fieldType = isSensitive && !isVisible
                                      ? "password"
                                      : param.param_type === "number"
                                      ? "number"
                                      : "text"
                                    const label = `${param.description}${param.required ? "" : " (Optional)"}`

                                    return (
                                      <div key={param.key}>
                                        <label className="block text-sm font-medium mb-2">{label}</label>
                                        <div className="relative">
                                          <Input
                                            type={fieldType}
                                            placeholder={param.default_value || ""}
                                            value={editConfig[param.key] || ""}
                                            onChange={(e) => handleConfigFieldChange(param.key, e.target.value)}
                                          />
                                          {isSensitive && (
                                            <button
                                              type="button"
                                              onClick={() => toggleFieldVisibility(param.key)}
                                              className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                                              title={isVisible ? "Hide" : "Show"}
                                            >
                                              {isVisible ? (
                                                <EyeOff className="h-4 w-4" />
                                              ) : (
                                                <Eye className="h-4 w-4" />
                                              )}
                                            </button>
                                          )}
                                        </div>
                                      </div>
                                    )
                                  })}
                              </>
                            )}
                          </CardContent>
                        </Card>
                      )}

                      {/* Danger Zone */}
                      <Card className="border-red-200 dark:border-red-900">
                        <CardHeader>
                          <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                          <CardDescription>
                            Irreversible and destructive actions for this provider
                          </CardDescription>
                        </CardHeader>
                        <CardContent className="space-y-4">
                          <div className="flex items-center justify-between">
                            <div>
                              <p className="text-sm font-medium">Enable provider</p>
                              <p className="text-sm text-muted-foreground">
                                When disabled, this provider will not be used for routing requests
                              </p>
                            </div>
                            <Switch
                              checked={selectedProvider.enabled}
                              onCheckedChange={() => handleToggle(selectedProvider)}
                            />
                          </div>
                          <div className="flex items-center justify-between pt-4 border-t">
                            <div>
                              <p className="text-sm font-medium">Delete this provider</p>
                              <p className="text-sm text-muted-foreground">
                                Permanently delete "{selectedProvider.instance_name}" and its configuration
                              </p>
                            </div>
                            <Button
                              variant="destructive"
                              onClick={() => {
                                setProviderToDelete(selectedProvider)
                                setDeleteDialogOpen(true)
                              }}
                            >
                              Delete Provider
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>

                  <TabsContent value="free-tier">
                    <div className="space-y-6">
                      {/* Configuration Card */}
                      <Card>
                        <CardHeader>
                          <CardTitle className="flex items-center gap-2">
                            <Coins className="h-4 w-4" />
                            Free Tier Configuration
                          </CardTitle>
                          <CardDescription>
                            Configure how this provider's free tier is tracked
                          </CardDescription>
                          {(() => {
                            const typeInfo = providerTypes.find(t => t.provider_type === selectedProvider.provider_type)
                            return typeInfo?.free_tier_long_text ? (
                              <p className="text-sm text-muted-foreground mt-1">{typeInfo.free_tier_long_text}</p>
                            ) : null
                          })()}
                        </CardHeader>
                        <CardContent>
                          {(() => {
                            const status = freeTierStatus[selectedProvider.instance_name]
                            if (!status) {
                              return (
                                <div className="flex items-center gap-2 text-muted-foreground">
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                  <span>Loading free tier status...</span>
                                </div>
                              )
                            }

                            const kind = status.free_tier?.kind

                            return (
                              <div className="space-y-4">
                                {/* Type + Override Toggle */}
                                <div className="flex items-center justify-between">
                                  <div className="flex items-center gap-2">
                                    <span className="text-sm font-medium">Type</span>
                                    <TooltipProvider delayDuration={300}>
                                      <Tooltip>
                                        <TooltipTrigger asChild>
                                          <span>
                                            <Badge variant={
                                              kind === 'always_free_local' ? 'default' :
                                              kind === 'subscription' ? 'default' :
                                              kind === 'none' ? 'secondary' :
                                              'outline'
                                            } className="cursor-help">
                                              {FREE_TIER_LABELS[kind ?? 'none'] ?? 'No Free Tier'}
                                            </Badge>
                                          </span>
                                        </TooltipTrigger>
                                        <TooltipContent side="bottom" className="max-w-xs text-xs">
                                          {FREE_TIER_DESCRIPTIONS[kind ?? 'none']}
                                        </TooltipContent>
                                      </Tooltip>
                                    </TooltipProvider>
                                    {status.is_user_override && (
                                      <Badge variant="outline" className="text-xs">Override</Badge>
                                    )}
                                  </div>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => {
                                      if (freeTierOverrideEditing) {
                                        setFreeTierOverrideEditing(false)
                                      } else {
                                        setFreeTierOverrideKind(status.free_tier ?? { kind: 'none' })
                                        setFreeTierOverrideEditing(true)
                                      }
                                    }}
                                  >
                                    <Pencil className="h-3.5 w-3.5 mr-1" />
                                    {freeTierOverrideEditing ? 'Cancel' : 'Edit'}
                                  </Button>
                                </div>

                                {/* Override Editor */}
                                {freeTierOverrideEditing && (
                                  <div className="space-y-3 p-3 rounded-md border bg-muted/30">
                                    <div className="space-y-1.5">
                                      <label className="text-xs font-medium text-muted-foreground">Free Tier Type</label>
                                      <Select
                                        value={freeTierOverrideKind.kind}
                                        onValueChange={(value) => {
                                          switch (value) {
                                            case 'none': setFreeTierOverrideKind({ kind: 'none' }); break
                                            case 'always_free_local': setFreeTierOverrideKind({ kind: 'always_free_local' }); break
                                            case 'subscription': setFreeTierOverrideKind({ kind: 'subscription' }); break
                                            case 'rate_limited_free': setFreeTierOverrideKind({ kind: 'rate_limited_free', max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 }); break
                                            case 'credit_based': setFreeTierOverrideKind({ kind: 'credit_based', budget_usd: 5.0, reset_period: 'monthly', detection: { type: 'local_only' } }); break
                                            case 'free_models_only': setFreeTierOverrideKind({ kind: 'free_models_only', free_model_patterns: [], max_rpm: 3 }); break
                                          }
                                        }}
                                      >
                                        <SelectTrigger className="h-8 text-xs">
                                          <SelectValue />
                                        </SelectTrigger>
                                        <SelectContent>
                                          {([
                                            ['none', 'No Free Tier'],
                                            ['always_free_local', 'Always Free (Local)'],
                                            ['subscription', 'Subscription'],
                                            ['rate_limited_free', 'Rate Limited'],
                                            ['credit_based', 'Credit Based'],
                                            ['free_models_only', 'Free Models Only'],
                                          ] as const).map(([value, label]) => (
                                            <SelectItem key={value} value={value} className="text-xs py-2">
                                              <div>
                                                <div className="font-medium">{label}</div>
                                                <div className="text-[11px] text-muted-foreground font-normal mt-0.5 whitespace-normal break-words">{FREE_TIER_DESCRIPTIONS[value]}</div>
                                              </div>
                                            </SelectItem>
                                          ))}
                                        </SelectContent>
                                      </Select>
                                    </div>

                                    {/* Rate Limited fields */}
                                    {freeTierOverrideKind.kind === 'rate_limited_free' && (
                                      <div className="space-y-3">
                                        <div className="grid grid-cols-2 gap-2">
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Requests per Minute (RPM)</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_rpm}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_rpm: parseInt(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Requests per Day (RPD)</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_rpd}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_rpd: parseInt(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Tokens per Minute (TPM)</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_tpm}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_tpm: parseInt(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Tokens per Day (TPD)</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_tpd}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_tpd: parseInt(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Monthly Call Limit</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_monthly_calls}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_monthly_calls: parseInt(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Monthly Token Limit</label>
                                            <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_monthly_tokens}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_monthly_tokens: parseInt(e.target.value) || 0 })} />
                                          </div>
                                        </div>
                                        <p className="text-xs text-muted-foreground">Set to 0 to disable tracking for that limit. The router also reads rate limit headers from provider responses when available, which take precedence over these configured limits.</p>
                                      </div>
                                    )}

                                    {/* Credit Based fields */}
                                    {freeTierOverrideKind.kind === 'credit_based' && (
                                      <div className="space-y-2">
                                        <div className="grid grid-cols-2 gap-2">
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Credit Budget (USD)</label>
                                            <Input type="number" step="0.01" className="h-7 text-xs" value={freeTierOverrideKind.budget_usd}
                                              onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, budget_usd: parseFloat(e.target.value) || 0 })} />
                                          </div>
                                          <div className="space-y-1">
                                            <label className="text-xs text-muted-foreground">Reset Period</label>
                                            <Select
                                              value={freeTierOverrideKind.reset_period}
                                              onValueChange={(value: 'daily' | 'monthly' | 'never') => setFreeTierOverrideKind({ ...freeTierOverrideKind, reset_period: value })}
                                            >
                                              <SelectTrigger className="h-7 text-xs">
                                                <SelectValue />
                                              </SelectTrigger>
                                              <SelectContent>
                                                <SelectItem value="daily" className="text-xs">Daily</SelectItem>
                                                <SelectItem value="monthly" className="text-xs">Monthly</SelectItem>
                                                <SelectItem value="never" className="text-xs">One-time (no reset)</SelectItem>
                                              </SelectContent>
                                            </Select>
                                          </div>
                                        </div>
                                        <p className="text-xs text-muted-foreground">The router estimates cost from token usage and model pricing. When estimated spend reaches the budget, the provider is skipped. Use "Set Usage" below to sync with your actual balance from the provider dashboard.</p>
                                      </div>
                                    )}

                                    {/* Free Models Only fields */}
                                    {freeTierOverrideKind.kind === 'free_models_only' && (
                                      <div className="space-y-3">
                                        <div className="space-y-1.5">
                                          <label className="text-xs text-muted-foreground">Free Models</label>
                                          {models.length > 0 ? (
                                            <div className="rounded-md border max-h-48 overflow-y-auto">
                                              {models.map((model) => {
                                                const isSelected = freeTierOverrideKind.free_model_patterns.includes(model.id)
                                                return (
                                                  <div
                                                    key={model.id}
                                                    className="flex items-center gap-2.5 px-3 py-1.5 cursor-pointer hover:bg-muted/50 border-b last:border-b-0"
                                                    onClick={() => {
                                                      const patterns = new Set(freeTierOverrideKind.free_model_patterns)
                                                      if (isSelected) {
                                                        patterns.delete(model.id)
                                                      } else {
                                                        patterns.add(model.id)
                                                      }
                                                      setFreeTierOverrideKind({
                                                        ...freeTierOverrideKind,
                                                        free_model_patterns: Array.from(patterns),
                                                      })
                                                    }}
                                                  >
                                                    <Checkbox checked={isSelected} onCheckedChange={() => {}} />
                                                    <span className="text-xs truncate">{model.name || model.id}</span>
                                                  </div>
                                                )
                                              })}
                                            </div>
                                          ) : (
                                            <p className="text-xs text-muted-foreground italic py-2">No models loaded for this provider. Models will appear here once the provider is connected.</p>
                                          )}
                                          <p className="text-xs text-muted-foreground">
                                            {freeTierOverrideKind.free_model_patterns.length > 0
                                              ? `${freeTierOverrideKind.free_model_patterns.length} model${freeTierOverrideKind.free_model_patterns.length === 1 ? '' : 's'} selected as free. All other models from this provider will be skipped in free-tier mode.`
                                              : 'Select which models are free. Unselected models will be skipped in free-tier mode.'}
                                          </p>
                                        </div>
                                        <div className="space-y-1">
                                          <label className="text-xs text-muted-foreground">Requests per Minute (RPM)</label>
                                          <Input type="number" className="h-7 text-xs" value={freeTierOverrideKind.max_rpm}
                                            onChange={(e) => setFreeTierOverrideKind({ ...freeTierOverrideKind, max_rpm: parseInt(e.target.value) || 0 })} />
                                          <p className="text-xs text-muted-foreground">Rate limit applied to the free models. Set to 0 to disable.</p>
                                        </div>
                                      </div>
                                    )}

                                    {/* Save / Reset to Default buttons */}
                                    <div className="flex gap-2 pt-2">
                                      <Button
                                        size="sm"
                                        onClick={() => saveFreeTierOverride(selectedProvider.instance_name, freeTierOverrideKind)}
                                      >
                                        Save Override
                                      </Button>
                                      {status.is_user_override && (
                                        <Button
                                          variant="outline"
                                          size="sm"
                                          onClick={() => saveFreeTierOverride(selectedProvider.instance_name, null)}
                                        >
                                          <RotateCcw className="h-3.5 w-3.5 mr-1" />
                                          Reset to Default
                                        </Button>
                                      )}
                                    </div>
                                  </div>
                                )}
                              </div>
                            )
                          })()}
                        </CardContent>
                      </Card>

                      {/* Usage & Status Card */}
                      <Card>
                        <CardHeader>
                          <CardTitle className="text-sm">Usage & Status</CardTitle>
                        </CardHeader>
                        <CardContent>
                          {(() => {
                            const status = freeTierStatus[selectedProvider.instance_name]
                            if (!status) return null

                            const kind = status.free_tier?.kind
                            return (
                              <div className="space-y-4">
                                {/* Free tier status */}
                                <div className="flex items-center justify-between">
                                  <span className="text-sm font-medium">Free Tier</span>
                                  <span className={`text-sm ${status.has_capacity ? 'text-green-600' : 'text-red-600'}`}>
                                    {status.status_message || (status.has_capacity ? 'Available' : 'Exhausted')}
                                  </span>
                                </div>

                                {/* Rate limit details */}
                                {kind === 'rate_limited_free' && (
                                  <div className="space-y-3 pt-2 border-t">
                                    <span className="text-sm font-medium">Usage</span>
                                    {status.rate_rpm_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Requests/min</span>
                                          <span>{status.rate_rpm_used ?? 0} / {status.rate_rpm_limit}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_rpm_used ?? 0) / status.rate_rpm_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_rpm_used ?? 0) / status.rate_rpm_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_rpm_used ?? 0) / status.rate_rpm_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                    {status.rate_rpd_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Requests/day</span>
                                          <span>{status.rate_rpd_used ?? 0} / {status.rate_rpd_limit}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_rpd_used ?? 0) / status.rate_rpd_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_rpd_used ?? 0) / status.rate_rpd_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_rpd_used ?? 0) / status.rate_rpd_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                    {status.rate_tpm_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Tokens/min</span>
                                          <span>{(status.rate_tpm_used ?? 0).toLocaleString()} / {status.rate_tpm_limit.toLocaleString()}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_tpm_used ?? 0) / status.rate_tpm_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_tpm_used ?? 0) / status.rate_tpm_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_tpm_used ?? 0) / status.rate_tpm_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                    {status.rate_tpd_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Tokens/day</span>
                                          <span>{(status.rate_tpd_used ?? 0).toLocaleString()} / {status.rate_tpd_limit.toLocaleString()}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_tpd_used ?? 0) / status.rate_tpd_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_tpd_used ?? 0) / status.rate_tpd_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_tpd_used ?? 0) / status.rate_tpd_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                    {status.rate_monthly_calls_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Monthly calls</span>
                                          <span>{status.rate_monthly_calls_used ?? 0} / {status.rate_monthly_calls_limit}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_monthly_calls_used ?? 0) / status.rate_monthly_calls_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_monthly_calls_used ?? 0) / status.rate_monthly_calls_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_monthly_calls_used ?? 0) / status.rate_monthly_calls_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                    {status.rate_monthly_tokens_limit != null && (
                                      <div>
                                        <div className="flex justify-between text-xs mb-1">
                                          <span className="text-muted-foreground">Monthly tokens</span>
                                          <span>{(status.rate_monthly_tokens_used ?? 0).toLocaleString()} / {status.rate_monthly_tokens_limit.toLocaleString()}</span>
                                        </div>
                                        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                          <div
                                            className={`h-full rounded-full ${
                                              (status.rate_monthly_tokens_used ?? 0) / status.rate_monthly_tokens_limit > 0.9 ? 'bg-red-500' :
                                              (status.rate_monthly_tokens_used ?? 0) / status.rate_monthly_tokens_limit > 0.7 ? 'bg-amber-500' :
                                              'bg-green-500'
                                            }`}
                                            style={{ width: `${Math.min(100, ((status.rate_monthly_tokens_used ?? 0) / status.rate_monthly_tokens_limit) * 100)}%` }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                  </div>
                                )}

                                {/* Credit details */}
                                {kind === 'credit_based' && (
                                  <div className="space-y-3 pt-2 border-t">
                                    <span className="text-sm font-medium">Credits</span>
                                    <div>
                                      <div className="flex justify-between text-xs mb-1">
                                        <span className="text-muted-foreground">Used</span>
                                        <span>${(status.credit_used_usd ?? 0).toFixed(4)} / ${(status.credit_budget_usd ?? 0).toFixed(2)}</span>
                                      </div>
                                      <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                                        <div
                                          className={`h-full rounded-full ${
                                            status.credit_remaining_usd != null && status.credit_budget_usd != null && status.credit_budget_usd > 0
                                              ? (status.credit_remaining_usd / status.credit_budget_usd < 0.1 ? 'bg-red-500' :
                                                 status.credit_remaining_usd / status.credit_budget_usd < 0.3 ? 'bg-amber-500' :
                                                 'bg-green-500')
                                              : 'bg-muted'
                                          }`}
                                          style={{
                                            width: `${status.credit_budget_usd && status.credit_budget_usd > 0
                                              ? Math.min(100, ((status.credit_used_usd ?? 0) / status.credit_budget_usd) * 100)
                                              : 0}%`
                                          }}
                                        />
                                      </div>
                                    </div>
                                    {status.credit_remaining_usd != null && (
                                      <p className="text-xs text-muted-foreground">
                                        ${status.credit_remaining_usd.toFixed(4)} remaining
                                      </p>
                                    )}
                                  </div>
                                )}

                                {/* Backoff status */}
                                {status.is_backed_off && (
                                  <div className="flex items-center gap-2 pt-2 border-t text-amber-600">
                                    <AlertCircle className="h-4 w-4" />
                                    <span className="text-sm">
                                      {status.backoff_reason ?? 'Rate limited'}
                                      {status.backoff_retry_after_secs != null && ` (available in ${status.backoff_retry_after_secs}s)`}
                                    </span>
                                  </div>
                                )}

                                {/* Actions - only for kinds that track usage */}
                                {(kind === 'rate_limited_free' || kind === 'credit_based') && (
                                  <div className="flex gap-2 pt-2 border-t">
                                    <Button
                                      variant="outline"
                                      size="sm"
                                      onClick={async () => {
                                        try {
                                          await invoke("reset_provider_free_tier_usage", {
                                            providerInstance: selectedProvider.instance_name,
                                          })
                                          toast.success("Free tier usage reset")
                                          loadFreeTierStatus(selectedProvider.instance_name)
                                        } catch (error) {
                                          console.error("Failed to reset free tier usage:", error)
                                          toast.error("Failed to reset free tier usage")
                                        }
                                      }}
                                    >
                                      Reset Usage
                                    </Button>
                                    <Button
                                      variant="outline"
                                      size="sm"
                                      onClick={() => {
                                        setSetUsageValues({
                                          creditUsedUsd: status.credit_used_usd != null ? String(status.credit_used_usd) : '',
                                          creditRemainingUsd: status.credit_remaining_usd != null ? String(status.credit_remaining_usd) : '',
                                          dailyRequests: status.rate_rpd_used != null ? String(status.rate_rpd_used) : '',
                                          monthlyRequests: status.rate_monthly_calls_used != null ? String(status.rate_monthly_calls_used) : '',
                                          monthlyTokens: '',
                                        })
                                        setSetUsageDialogOpen(true)
                                      }}
                                    >
                                      Set Usage
                                    </Button>
                                  </div>
                                )}

                                {/* Simple reset for free_models_only which has rate tracking but no editable usage */}
                                {kind === 'free_models_only' && (
                                  <div className="pt-2 border-t">
                                    <Button
                                      variant="outline"
                                      size="sm"
                                      onClick={async () => {
                                        try {
                                          await invoke("reset_provider_free_tier_usage", {
                                            providerInstance: selectedProvider.instance_name,
                                          })
                                          toast.success("Free tier usage reset")
                                          loadFreeTierStatus(selectedProvider.instance_name)
                                        } catch (error) {
                                          console.error("Failed to reset free tier usage:", error)
                                          toast.error("Failed to reset free tier usage")
                                        }
                                      }}
                                    >
                                      Reset Usage
                                    </Button>
                                  </div>
                                )}
                              </div>
                            )
                          })()}
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>
                </Tabs>

              </div>
            </ScrollArea>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
              <ProvidersIcon className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">Select a provider to view details</p>
                <p className="text-sm">
                  or add a new one with the + button
                </p>
              </div>
            </div>
          )}
        </ResizablePanel>
      </ResizablePanelGroup>

      {/* Create Provider Dialog */}
      <Dialog open={createDialogOpen} onOpenChange={(open) => {
        setCreateDialogOpen(open)
        if (!open) {
          setSelectedProviderType("")
          setDialogPage("select")
          setCreateTab("templates")
        }
      }}>
        <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Add Provider</DialogTitle>
          </DialogHeader>

          {dialogPage === "select" ? (
            /* Page 1: Selection */
            <Tabs value={createTab} onValueChange={(v) => setCreateTab(v as typeof createTab)}>
              <TabsList className="grid w-full grid-cols-2">
                <TabsTrigger value="templates" className="gap-2">
                  <Grid className="h-4 w-4" />
                  Templates
                </TabsTrigger>
                <TabsTrigger value="custom" className="gap-2">
                  <Settings className="h-4 w-4" />
                  Custom
                </TabsTrigger>
              </TabsList>

              {/* Templates Tab */}
              <TabsContent value="templates" className="mt-4">
                {(() => {
                  // Group providers by category from backend (excluding generic for templates)
                  const localProviders = providerTypes.filter(t => t.category === 'local')
                  const subscriptionProviders = providerTypes.filter(t => t.category === 'subscription')
                  const firstPartyProviders = providerTypes.filter(t => t.category === 'first_party')
                  const thirdPartyProviders = providerTypes.filter(t => t.category === 'third_party')

                  const ProviderButton = ({ type }: { type: ProviderType }) => (
                    <button
                      key={type.provider_type}
                      onClick={() => {
                        setSelectedProviderType(type.provider_type)
                        setDialogPage("configure")
                      }}
                      className={cn(
                        "flex flex-col items-center gap-2 p-4 rounded-lg border-2 border-muted",
                        "hover:border-primary hover:bg-accent transition-colors",
                        "focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
                      )}
                    >
                      <ProviderIcon providerId={type.provider_type.toLowerCase()} size={40} />
                      <div className="text-center">
                        <p className="font-medium text-sm">{type.display_name}</p>
                        <p className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
                          {type.description}
                        </p>
                        {type.free_tier_short_text && (
                          <p className="text-[11px] text-green-600 dark:text-green-400 font-medium mt-1">
                            {type.free_tier_short_text}
                          </p>
                        )}
                      </div>
                    </button>
                  )

                  const ProviderSection = ({ title, description, providers }: {
                    title: string
                    description: string
                    providers: ProviderType[]
                  }) => {
                    if (providers.length === 0) return null
                    return (
                      <div className="space-y-3">
                        <div>
                          <h3 className="text-sm font-semibold">{title}</h3>
                          <p className="text-xs text-muted-foreground">{description}</p>
                        </div>
                        <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
                          {providers.map((type) => (
                            <ProviderButton key={type.provider_type} type={type} />
                          ))}
                        </div>
                      </div>
                    )
                  }

                  return (
                    <div className="space-y-6">
                      <ProviderSection
                        title="Local Providers"
                        description="Connect to models running on your machine"
                        providers={localProviders}
                      />
                      <ProviderSection
                        title="Subscription Cloud Providers"
                        description="Connect using your existing subscription (OAuth)"
                        providers={subscriptionProviders}
                      />
                      <ProviderSection
                        title="First-Party Cloud Providers"
                        description="Direct API access to model creators"
                        providers={firstPartyProviders}
                      />
                      <ProviderSection
                        title="Third-Party Hosting"
                        description="Platforms hosting models from multiple sources"
                        providers={thirdPartyProviders}
                      />
                    </div>
                  )
                })()}
              </TabsContent>

              {/* Custom Tab - Generic/OpenAI-compatible only */}
              <TabsContent value="custom" className="mt-4">
                {(() => {
                  const genericType = providerTypes.find(t => t.category === 'generic')
                  if (!genericType) {
                    return (
                      <div className="text-center py-8 text-muted-foreground">
                        <p>Generic provider type not available</p>
                      </div>
                    )
                  }
                  return (
                    <div className="space-y-4">
                      <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                        <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                          OpenAI-Compatible Provider
                        </p>
                        <p className="text-xs text-blue-700 dark:text-blue-300 mt-1">
                          Connect to any API that follows the OpenAI API format
                        </p>
                      </div>
                      <ProviderForm
                        mode="create"
                        providerType={genericType}
                        initialInstanceName={generateDefaultName(genericType.display_name)}
                        onSubmit={handleCreateProvider}
                        onCancel={() => {
                          setCreateDialogOpen(false)
                          setSelectedProviderType("")
                          setCreateTab("templates")
                        }}
                        isSubmitting={isSubmitting}
                      />
                    </div>
                  )
                })()}
              </TabsContent>
            </Tabs>
          ) : (
            /* Page 2: Configuration Form */
            <div className="space-y-4">
              {/* Back button and provider header */}
              <div className="flex items-center gap-3 pb-2 border-b">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setDialogPage("select")
                    setSelectedProviderType("")
                  }}
                  className="h-8 px-2"
                >
                  <ArrowLeft className="h-4 w-4 mr-1" />
                  Back
                </Button>
                {selectedTypeForCreate && (
                  <div className="flex items-center gap-2">
                    <ProviderIcon providerId={selectedTypeForCreate.provider_type.toLowerCase()} size={24} />
                    <div>
                      <p className="text-sm font-medium">{selectedTypeForCreate.display_name}</p>
                      <p className="text-xs text-muted-foreground">{selectedTypeForCreate.description}</p>
                    </div>
                  </div>
                )}
              </div>

              {selectedTypeForCreate && (
                <ProviderForm
                  mode="create"
                  providerType={selectedTypeForCreate}
                  initialInstanceName={generateDefaultName(selectedTypeForCreate.display_name)}
                  onSubmit={handleCreateProvider}
                  onCancel={() => {
                    setCreateDialogOpen(false)
                    setSelectedProviderType("")
                    setDialogPage("select")
                  }}
                  isSubmitting={isSubmitting}
                />
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <AlertDialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{providerToDelete?.instance_name}"? This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Set Usage Dialog */}
      <Dialog open={setUsageDialogOpen} onOpenChange={setSetUsageDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Set Usage</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            Manually adjust the tracked usage counters for this provider. This is useful when the router's local tracking has drifted from your actual usage, or to sync with the balance shown on your provider dashboard. Leave fields empty to keep them unchanged.
          </p>
          {(() => {
            const provider = providers.find(p => p.instance_name === selectedId)
            const status = provider ? freeTierStatus[provider.instance_name] : null
            const kind = status?.free_tier?.kind

            return (
              <div className="space-y-4">
                {kind === 'credit_based' && (
                  <>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Credits Used (USD)</label>
                      <Input type="number" step="0.01" placeholder="0.00"
                        value={setUsageValues.creditUsedUsd}
                        onChange={(e) => setSetUsageValues(prev => ({ ...prev, creditUsedUsd: e.target.value }))} />
                      <p className="text-xs text-muted-foreground">How much of the credit budget has been consumed so far in the current period. The router uses this to calculate remaining capacity.</p>
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Credits Remaining (USD)</label>
                      <Input type="number" step="0.01" placeholder="e.g. 4.50"
                        value={setUsageValues.creditRemainingUsd}
                        onChange={(e) => setSetUsageValues(prev => ({ ...prev, creditRemainingUsd: e.target.value }))} />
                      <p className="text-xs text-muted-foreground">The actual remaining balance from your provider dashboard. This overrides the router's calculated estimate and is the most accurate way to sync usage.</p>
                    </div>
                  </>
                )}
                {kind === 'rate_limited_free' && (
                  <>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Daily Requests Used</label>
                      <Input type="number" placeholder="0"
                        value={setUsageValues.dailyRequests}
                        onChange={(e) => setSetUsageValues(prev => ({ ...prev, dailyRequests: e.target.value }))} />
                      <p className="text-xs text-muted-foreground">Number of requests made today. Resets automatically at the start of each day (midnight UTC, or midnight PT for Gemini).</p>
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Monthly Requests Used</label>
                      <Input type="number" placeholder="0"
                        value={setUsageValues.monthlyRequests}
                        onChange={(e) => setSetUsageValues(prev => ({ ...prev, monthlyRequests: e.target.value }))} />
                      <p className="text-xs text-muted-foreground">Number of requests made this month. Only relevant for providers with monthly call caps (e.g. Cohere: 1,000/month). Resets on the 1st of each month.</p>
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-sm font-medium">Monthly Tokens Used</label>
                      <Input type="number" placeholder="0"
                        value={setUsageValues.monthlyTokens}
                        onChange={(e) => setSetUsageValues(prev => ({ ...prev, monthlyTokens: e.target.value }))} />
                      <p className="text-xs text-muted-foreground">Total tokens used this month. Only relevant for providers with monthly token caps (e.g. Mistral: 1B tokens/month). Resets on the 1st of each month.</p>
                    </div>
                  </>
                )}
                <div className="flex justify-end gap-2 pt-2">
                  <Button variant="outline" onClick={() => setSetUsageDialogOpen(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => {
                    if (provider) saveSetUsage(provider.instance_name, kind ?? 'none')
                  }}>
                    Save
                  </Button>
                </div>
              </div>
            )
          })()}
        </DialogContent>
      </Dialog>
    </>
  )
}
