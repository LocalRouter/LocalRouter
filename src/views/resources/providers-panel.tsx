import { ProviderPrivacySettings, PRIVACY_ATTESTATION_KEY } from "@/components/providers/privacy-attestation-settings"
import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { isValidHttpUrl } from "@/utils/url"
import { listenSafe } from "@/hooks/useTauriListener"
import { GatewayRoutingSettings, GATEWAY_CONFIG_KEY } from "@/components/providers/GatewayRoutingSettings"
import { EmbeddedEngineTab, isEmbeddedProviderType } from "@/components/providers/EmbeddedEngineTab"
import { LocalModelsTab } from "@/components/providers/LocalModelsTab"
import { EngineModelsTab } from "@/components/providers/EngineModelsTab"
import { ProviderModelsTab } from "@/components/providers/ProviderModelsTab"
import { toast } from "sonner"
import { CheckCircle, XCircle, AlertCircle, Plus, Loader2, RefreshCw, FlaskConical, Grid, Settings, ArrowLeft, Eye, EyeOff, Copy, Trash2, ExternalLink, Terminal, Boxes, Search } from "lucide-react"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"

import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/Toggle"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

import { Input } from "@/components/ui/Input"
import { Textarea } from "@/components/ui/textarea"
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
import { ProviderFreeTierTab } from "@/views/resources/provider-free-tier"
import { OAuthSettingsControls, OAUTH_PROVIDER_MAP } from "@/components/OAuthSettingsControls"
import ProviderIcon from "@/components/ProviderIcon"
import { LlmTab } from "@/views/try-it-out/llm-tab"
import { cn } from "@/lib/utils"
import type { DetailedModelInfo, ProviderFreeTierStatus, ProviderFeatureSupport, GetProviderFeatureSupportParams, ListProviderModelsDetailedParams, ListProviderModelsManagedParams, ProviderManagedModels } from "@/types/tauri-commands"
import { ModelPricingBadge, FREE_TIER_LABELS, pricingSourceLabel } from "@/components/shared/model-pricing-badge"
import { ProviderFeatureTable } from "@/components/shared/feature-support-matrix"

const PROVIDER_STATUS_PAGES: Record<string, string> = {
  openai: "https://status.openai.com",
  anthropic: "https://status.anthropic.com",
  gemini: "https://www.google.com/appsstatus/dashboard/",
  mistral: "https://status.mistral.ai",
  cohere: "https://status.cohere.io",
  xai: "https://status.x.ai",
  openrouter: "https://status.openrouter.ai",
  groq: "https://groqstatus.com",
  togetherai: "https://status.together.ai",
  perplexity: "https://status.perplexity.com",
  deepinfra: "https://status.deepinfra.com",
  cerebras: "https://status.cerebras.ai",
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


type DetailedModel = DetailedModelInfo

interface ProvidersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  healthStatus: Record<string, HealthStatus>
  onHealthInit: (providerNames: string[]) => void
  onRefreshHealth: (instanceName: string) => Promise<void>
  initialAddProviderType?: string | null
  /** Open the provider filtered to this model (e.g. from the command palette) */
  initialModelSearch?: string | null
  onViewChange?: (view: string, subTab?: string | null) => void
}

/** Config key of the "Discover models" switch (shown on the Models tab). */
const MODEL_DISCOVERY_KEY = "model_discovery"

export function ProvidersPanel({
  selectedId,
  onSelect,
  healthStatus,
  onHealthInit,
  onRefreshHealth,
  initialAddProviderType,
  initialModelSearch,
  onViewChange: _onViewChange,
}: ProvidersPanelProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const [providerToDelete, setProviderToDelete] = useState<Provider | null>(null)

  // Detailed models for the Models tab
  const [detailedModels, setDetailedModels] = useState<DetailedModel[]>([])
  const [detailedModelsLoading, setDetailedModelsLoading] = useState(false)
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null)
  // Listing error and hidden count for the Info tab's model list
  const [modelsSummary, setModelsSummary] = useState<ProviderManagedModels | null>(null)
  // Filter for the Info tab's model list
  const [modelSearch, setModelSearch] = useState("")

  // Detail tab state
  const [detailTab, setDetailTab] = useState("info")

  // Feature support state
  const [featureSupport, setFeatureSupport] = useState<ProviderFeatureSupport | null>(null)

  // Free tier state
  const [freeTierStatus, setFreeTierStatus] = useState<Record<string, ProviderFreeTierStatus>>({})

  // Create form state
  const [dialogPage, setDialogPage] = useState<"select" | "configure">("select")
  const [createTab, setCreateTab] = useState<"templates" | "custom">("templates")
  const [customTypeId, setCustomTypeId] = useState<string>("openai_compatible")
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
    const l = listenSafe("providers-changed", () => {
      loadProvidersOnly()
    })

    return () => {
      l.cleanup()
    }
  }, [])

  // Reset detail tab when a different provider is selected (not during rename).
  // A just-created Local Embedded provider opens on its Engine tab instead.
  const skipTabResetRef = useRef(false)
  const nextTabRef = useRef<string | null>(null)
  useEffect(() => {
    if (skipTabResetRef.current) {
      skipTabResetRef.current = false
      return
    }
    setDetailTab(nextTabRef.current ?? "info")
    nextTabRef.current = null
    setFeatureSupport(null)
    setModelSearch("")
  }, [selectedId])

  // Opened for one model (command palette): show the list filtered to it.
  useEffect(() => {
    if (initialModelSearch) {
      setDetailTab("info")
      setModelSearch(initialModelSearch)
    }
  }, [initialModelSearch, selectedId])

  // Load feature support and models when a provider is selected.
  // Uses a cancellation flag so a slow response for a previous provider cannot
  // overwrite the fresh data for the currently-selected one.
  useEffect(() => {
    if (!selectedId) return
    let cancelled = false

    invoke<ProviderFeatureSupport>("get_provider_feature_support", {
      instanceName: selectedId,
    } satisfies GetProviderFeatureSupportParams)
      .then((result) => { if (!cancelled) setFeatureSupport(result) })
      .catch((err) => console.error("Failed to load feature support:", err))

    setDetailedModelsLoading(true)
    setSelectedModelId(null)
    setModelsSummary(null)
    // Only this provider's models: other providers never hold it up.
    const loadModels = () =>
      invoke<DetailedModel[]>("list_provider_models_detailed", {
        instanceName: selectedId,
      } satisfies ListProviderModelsDetailedParams)
        .then((models) => {
          if (!cancelled) setDetailedModels(models)
        })
        .catch((error) => {
          if (cancelled) return
          console.error("Failed to load detailed models:", error)
          setDetailedModels([])
        })
        .finally(() => {
          if (cancelled) return
          setDetailedModelsLoading(false)
          // From the cache the call above filled: no second fetch.
          invoke<ProviderManagedModels>("list_provider_models_managed", {
            instanceName: selectedId,
          } satisfies ListProviderModelsManagedParams)
            .then((summary) => {
              if (!cancelled) setModelsSummary(summary)
            })
            .catch(() => {})
        })
    loadModels()
    // Downloads and refreshes change the list while the page is open.
    const changed = listenSafe("models-changed", () => {
      if (!cancelled) loadModels()
    })

    loadFreeTierStatus(selectedId)

    return () => {
      cancelled = true
      changed.cleanup()
    }
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
      const statuses = await invoke<ProviderFreeTierStatus[]>("get_free_tier_status")
      const status = statuses.find((s) => s.provider_instance === instanceName)
      if (status) {
        setFreeTierStatus(prev => ({ ...prev, [instanceName]: status }))
      }
    } catch (error) {
      console.error("Failed to load free tier status:", error)
    }
  }, [])

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
    }
  }

  const handleCloneProvider = async (e: React.MouseEvent, provider: Provider) => {
    e.stopPropagation()
    try {
      await invoke("clone_provider_instance", { instanceName: provider.instance_name })
      toast.success(`Cloned "${provider.instance_name}"`)
    } catch (error) {
      toast.error(`Failed to clone provider: ${error}`)
    }
  }

  const handleCreateProvider = async (providerType: string, instanceName: string, config: Record<string, string>) => {
    setIsSubmitting(true)
    try {
      await invoke("create_provider_instance", {
        instanceName,
        providerType,
        config,
      })
      toast.success("Provider created")
      setCreateDialogOpen(false)
      setSelectedProviderType("")
      setDialogPage("select")
      setCreateTab("templates")
      await loadProvidersOnly()
      if (isEmbeddedProviderType(providerType)) nextTabRef.current = "engine"
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
    if ((detailTab === "settings" || detailTab === "engine" || detailTab === "models") && selectedId) {
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

  const modelQuery = modelSearch.trim().toLowerCase()
  const visibleModels = modelQuery
    ? detailedModels.filter((m) => m.model_id.toLowerCase().includes(modelQuery))
    : detailedModels

  return (
    <>
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
                    {selectedProvider.enabled && (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setDetailTab("try-it-out")}
                      >
                        <FlaskConical className="h-4 w-4 mr-1" />
                        Try It Out
                      </Button>
                    )}
                  </div>
                </div>

                <Tabs value={detailTab} onValueChange={setDetailTab}>
                  <TabsList>
                    <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
                    {isEmbeddedProviderType(selectedProvider.provider_type) && (
                      <TabsTrigger value="engine"><Terminal className={TAB_ICON_CLASS} />Engine</TabsTrigger>
                    )}
                    <TabsTrigger value="models"><Boxes className={TAB_ICON_CLASS} />Models</TabsTrigger>
                    {selectedProvider.enabled && <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>}
                    <TabsTrigger value="compatibility"><TAB_ICONS.compatibility className={TAB_ICON_CLASS} />Compatibility</TabsTrigger>
                    <TabsTrigger value="free-tier" onClick={() => loadFreeTierStatus(selectedProvider.instance_name)}><TAB_ICONS.freeTier className={TAB_ICON_CLASS} />Free Tier</TabsTrigger>
                    <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
                  </TabsList>

                  {selectedProvider.enabled && (
                  <TabsContent value="try-it-out">
                    <LlmTab
                      key={selectedProvider.instance_name}
                      initialMode="direct"
                      initialProvider={selectedProvider.instance_name}
                      hideModeSwitcher
                      hideProviderSelector
                    />
                  </TabsContent>
                  )}

                  {isEmbeddedProviderType(selectedProvider.provider_type) && (
                    <TabsContent value="engine">
                      <EmbeddedEngineTab
                        key={selectedProvider.instance_name}
                        providerType={selectedProvider.provider_type}
                        instanceName={selectedProvider.instance_name}
                        binaryPath={editConfig.binary_path}
                      />
                    </TabsContent>
                  )}

                  {!isEmbeddedProviderType(selectedProvider.provider_type) && (
                    <TabsContent value="models">
                      <ProviderModelsTab
                        key={selectedProvider.instance_name}
                        instanceName={selectedProvider.instance_name}
                        discoveryValue={editConfig[MODEL_DISCOVERY_KEY]}
                        initialSearch={initialModelSearch ?? undefined}
                        onDiscoveryChange={(on) => handleConfigFieldChange(MODEL_DISCOVERY_KEY, on ? "true" : "false")}
                      />
                    </TabsContent>
                  )}

                  {isEmbeddedProviderType(selectedProvider.provider_type) && (
                    <TabsContent value="models">
                      {selectedProvider.provider_type === "llamacpp_embedded" ? (
                        <LocalModelsTab
                          key={selectedProvider.instance_name}
                          instanceName={selectedProvider.instance_name}
                          kvCache={editConfig.kv_cache}
                        />
                      ) : (
                        <EngineModelsTab
                          key={selectedProvider.instance_name}
                          providerType={selectedProvider.provider_type}
                          instanceName={selectedProvider.instance_name}
                          enabled={selectedProvider.enabled}
                        />
                      )}
                    </TabsContent>
                  )}

                  <TabsContent value="info">
                    <div className="space-y-6">
                      {/* Health Status */}
                      <Card>
                        <CardHeader className="pb-3">
                          <div className="flex items-center justify-between">
                            <CardTitle className="text-sm">Health Status</CardTitle>
                            <div className="flex items-center gap-1">
                              {PROVIDER_STATUS_PAGES[selectedProvider.provider_type] && (
                                <TooltipProvider delayDuration={300}>
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Button
                                        variant="ghost"
                                        size="icon"
                                        className="h-6 w-6"
                                        onClick={() => open(PROVIDER_STATUS_PAGES[selectedProvider.provider_type])}
                                      >
                                        <ExternalLink className="h-3 w-3" />
                                      </Button>
                                    </TooltipTrigger>
                                    <TooltipContent>Status page</TooltipContent>
                                  </Tooltip>
                                </TooltipProvider>
                              )}
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

                      {/* Models */}
                      <Card>
                        <CardHeader className="pb-3">
                          <div className="flex items-center justify-between gap-3">
                            <CardTitle className="text-sm">
                              Models
                              {!detailedModelsLoading && detailedModels.length > 0 && (
                                <span className="ml-2 text-xs font-normal text-muted-foreground">
                                  ({visibleModels.length === detailedModels.length
                                    ? detailedModels.length
                                    : `${visibleModels.length} of ${detailedModels.length}`})
                                </span>
                              )}
                              {modelsSummary && modelsSummary.hidden_count > 0 && (
                                <button
                                  type="button"
                                  className="ml-2 text-xs font-normal text-muted-foreground underline-offset-2 hover:underline"
                                  onClick={() => setDetailTab("models")}
                                >
                                  {modelsSummary.hidden_count} hidden
                                </button>
                              )}
                            </CardTitle>
                            {!isEmbeddedProviderType(selectedProvider.provider_type) && (
                              <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={() => setDetailTab("models")}>
                                Manage models
                              </Button>
                            )}
                          </div>
                          {detailedModels.length > 0 && (
                            <div className="relative mt-2">
                              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                              <Input
                                value={modelSearch}
                                onChange={(e) => setModelSearch(e.target.value)}
                                placeholder="Filter models"
                                className="pl-8 h-8 text-sm"
                                aria-label="Filter models"
                              />
                            </div>
                          )}
                        </CardHeader>
                        <CardContent>
                          {detailedModelsLoading ? (
                            <div className="flex items-center gap-2 text-muted-foreground">
                              <Loader2 className="h-4 w-4 animate-spin" />
                              <span>Loading models...</span>
                            </div>
                          ) : detailedModels.length === 0 ? (
                            modelsSummary?.discovery_error ? (
                              <div className="space-y-1">
                                <div className="flex items-center gap-2 text-sm text-red-600">
                                  <XCircle className="h-4 w-4 shrink-0" />
                                  <span>Listing models failed</span>
                                </div>
                                <pre className="text-xs text-muted-foreground overflow-auto max-h-32 whitespace-pre-wrap break-all bg-muted/50 rounded p-2">{modelsSummary.discovery_error}</pre>
                              </div>
                            ) : (
                              <p className="text-sm text-muted-foreground">
                                {modelsSummary && modelsSummary.hidden_count > 0
                                  ? "All of this provider's models are hidden."
                                  : modelsSummary && !modelsSummary.discovery_enabled
                                    ? "Model discovery is off and no models were added."
                                    : "No models available"}
                              </p>
                            )
                          ) : visibleModels.length === 0 ? (
                            <p className="text-sm text-muted-foreground">No models match &ldquo;{modelSearch}&rdquo;</p>
                          ) : (
                            <div className="space-y-1">
                              {modelsSummary?.discovery_error && (
                                <p className="text-xs text-yellow-600 dark:text-yellow-500 pb-2 break-all">
                                  Listing models failed; showing the last known list. {modelsSummary.discovery_error}
                                </p>
                              )}
                              {visibleModels.map((model) => {
                                const ftStatus = freeTierStatus[selectedProvider.instance_name]
                                const isExpanded = selectedModelId === model.model_id
                                return (
                                  <div key={model.model_id}>
                                    <div
                                      className={cn(
                                        "flex items-center justify-between p-3 rounded-md hover:bg-muted cursor-pointer",
                                        isExpanded && "bg-muted"
                                      )}
                                      onClick={() => setSelectedModelId(isExpanded ? null : model.model_id)}
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
                                          freeModel={model.free_tier_model}
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
                                    {isExpanded && (
                                      <div className="px-3 pb-3 pt-1 space-y-3">
                                        <div className="grid grid-cols-2 gap-3 text-sm">
                                          <div>
                                            <p className="text-muted-foreground text-xs">Context Window</p>
                                            <p className="font-medium">
                                              {model.context_window === 0
                                                ? "N/A"
                                                : model.context_window >= 1000000
                                                ? `${(model.context_window / 1000000).toFixed(1)}M tokens`
                                                : model.context_window >= 1000
                                                ? `${(model.context_window / 1000).toFixed(0)}K tokens`
                                                : `${model.context_window} tokens`}
                                            </p>
                                          </div>
                                          <div>
                                            <p className="text-muted-foreground text-xs">Streaming</p>
                                            <p className="font-medium">
                                              {model.supports_streaming ? "Supported" : "Not supported"}
                                            </p>
                                          </div>
                                          {model.parameter_count && (
                                            <div>
                                              <p className="text-muted-foreground text-xs">Parameters</p>
                                              <p className="font-medium">{model.parameter_count}</p>
                                            </div>
                                          )}
                                          {model.pricing_source && (
                                            <div>
                                              <p className="text-muted-foreground text-xs">Pricing Source</p>
                                              <p className="font-medium">{pricingSourceLabel(model.pricing_source)}</p>
                                            </div>
                                          )}
                                        </div>
                                        <div>
                                          <p className="text-muted-foreground text-xs mb-1">Pricing</p>
                                          <ModelPricingBadge
                                            inputPricePerMillion={model.input_price_per_million}
                                            outputPricePerMillion={model.output_price_per_million}
                                            freeTierKind={ftStatus?.free_tier}
                                            freeModel={model.free_tier_model}
                                            variant="full"
                                          />
                                        </div>
                                        {model.capabilities.length > 0 && (
                                          <div>
                                            <p className="text-muted-foreground text-xs mb-1">Capabilities</p>
                                            <div className="flex flex-wrap gap-1">
                                              {model.capabilities.map((cap) => (
                                                <Badge key={cap} variant="secondary" className="text-xs">
                                                  {cap}
                                                </Badge>
                                              ))}
                                            </div>
                                          </div>
                                        )}
                                        {ftStatus && ftStatus.free_tier.kind !== "none" && (
                                          <div>
                                            <p className="text-muted-foreground text-xs mb-1">Free Tier</p>
                                            <div className="text-sm space-y-1">
                                              <div className="flex items-center gap-2">
                                                <span className={cn(
                                                  "inline-block h-2 w-2 rounded-full",
                                                  model.free_tier_model?.is_free === false ? "bg-muted-foreground/50" : "bg-green-500",
                                                )} />
                                                <span className="font-medium text-xs">{FREE_TIER_LABELS[ftStatus.free_tier.kind]}</span>
                                              </div>
                                              {model.free_tier_model && (
                                                <p className="text-xs text-muted-foreground">
                                                  {model.free_tier_model.is_free ? "Free: " : "Not free: "}
                                                  {model.free_tier_model.reason}
                                                </p>
                                              )}
                                              {ftStatus.free_tier.kind === "rate_limited_free" && (
                                                <div className="flex gap-3 text-xs">
                                                  {ftStatus.rate_rpm_limit != null && (
                                                    <span>
                                                      <span className="text-muted-foreground">RPM: </span>
                                                      {ftStatus.rate_rpm_used ?? 0}/{ftStatus.rate_rpm_limit}
                                                    </span>
                                                  )}
                                                  {ftStatus.rate_rpd_limit != null && (
                                                    <span>
                                                      <span className="text-muted-foreground">RPD: </span>
                                                      {ftStatus.rate_rpd_used ?? 0}/{ftStatus.rate_rpd_limit}
                                                    </span>
                                                  )}
                                                </div>
                                              )}
                                              {ftStatus.free_tier.kind === "credit_based" && ftStatus.credit_budget_usd != null && (
                                                <div className="text-xs">
                                                  <span className="text-muted-foreground">Credits: </span>
                                                  ${(ftStatus.credit_remaining_usd ?? 0).toFixed(2)} / ${ftStatus.credit_budget_usd.toFixed(2)} remaining
                                                </div>
                                              )}
                                            </div>
                                          </div>
                                        )}
                                      </div>
                                    )}
                                  </div>
                                )
                              })}
                            </div>
                          )}
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>

                  <TabsContent value="compatibility">
                    {featureSupport ? (
                      <div className="space-y-4">
                        <ProviderFeatureTable title="API Endpoints" items={featureSupport.endpoints} />
                        <ProviderFeatureTable title="Model Features" items={featureSupport.model_features} />
                        <ProviderFeatureTable title="Optimization Features" items={featureSupport.optimization_features} />
                      </div>
                    ) : (
                      <div className="flex items-center gap-2 text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        <span>Loading compatibility data...</span>
                      </div>
                    )}
                  </TabsContent>

                  <TabsContent value="settings">
                    <div className="space-y-6">
                      <ProviderPrivacySettings instanceName={selectedId ?? ""} key={selectedId} value={editConfig[PRIVACY_ATTESTATION_KEY] || ""} disabled={configLoading} onChange={value => handleConfigFieldChange(PRIVACY_ATTESTATION_KEY,value)} />
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
                                  .filter((param) => param.param_type !== "oauth" && param.key !== MODEL_DISCOVERY_KEY)
                                  .map((param) => {
                                    if (param.key === GATEWAY_CONFIG_KEY) {
                                      return <GatewayRoutingSettings key={param.key} providerType={selectedProvider.provider_type} value={editConfig[param.key] || ""} onChange={value => handleConfigFieldChange(param.key, value)} />
                                    }
                                    const isSensitive = param.sensitive
                                    const isVisible = visibleFields.has(param.key)

                                    if (param.param_type === "boolean") {
                                      return (
                                        <div key={param.key} className="flex items-center gap-2">
                                          <input
                                            type="checkbox"
                                            id={`edit-${param.key}`}
                                            checked={(editConfig[param.key] ?? param.default_value) === "true"}
                                            onChange={(e) => handleConfigFieldChange(param.key, e.target.checked.toString())}
                                            className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500"
                                          />
                                          <label htmlFor={`edit-${param.key}`} className="text-sm font-medium">
                                            {param.description}
                                          </label>
                                        </div>
                                      )
                                    }

                                    const label = `${param.description}${param.required ? "" : " (Optional)"}`

                                    if (param.param_type === "headers") {
                                      return (
                                        <div key={param.key}>
                                          <label className="block text-sm font-medium mb-2">{label}</label>
                                          {/* Saved on blur, not per keystroke: a
                                              half-typed line ("X-Api-Vers") is
                                              rejected by the backend parser, so
                                              debounced saves would error while
                                              the user is still typing. */}
                                          <Textarea
                                            placeholder={"X-Api-Version: 2024-01-01\nX-Tenant-Id: acme"}
                                            value={editConfig[param.key] || ""}
                                            onChange={(e) => setEditConfig({ ...editConfig, [param.key]: e.target.value })}
                                            onBlur={(e) => handleConfigFieldChange(param.key, e.target.value)}
                                            rows={3}
                                            className="font-mono text-xs"
                                          />
                                          <p className="text-xs text-muted-foreground mt-1">
                                            One header per line, in "Name: Value" format
                                          </p>
                                        </div>
                                      )
                                    }

                                    const fieldType = isSensitive && !isVisible
                                      ? "password"
                                      : param.param_type === "number"
                                      ? "number"
                                      : "text"

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

                                {/* OAuth re-authentication for subscription
                                    providers (e.g. ChatGPT Plus). The OAuth
                                    setup param is rendered here instead of the
                                    plain field loop so an expired/revoked token
                                    can be refreshed without re-creating the
                                    provider. */}
                                {selectedTypeForEdit.setup_parameters.some(
                                  (param) => param.param_type === "oauth"
                                ) &&
                                  OAUTH_PROVIDER_MAP[selectedTypeForEdit.provider_type] && (
                                    <OAuthSettingsControls
                                      oauthProviderId={OAUTH_PROVIDER_MAP[selectedTypeForEdit.provider_type]}
                                      displayName={selectedTypeForEdit.display_name}
                                    />
                                  )}
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
                              <p className="text-sm font-medium">Remove this provider</p>
                              <p className="text-sm text-muted-foreground">
                                Remove "{selectedProvider.instance_name}" from LocalRouter. This does not delete the provider account or its models.
                              </p>
                            </div>
                            <Button
                              variant="destructive"
                              onClick={() => setProviderToDelete(selectedProvider)}
                            >
                              Remove Provider
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>

                  <TabsContent value="free-tier">
                    <ProviderFreeTierTab
                      key={selectedProvider.instance_name}
                      instanceName={selectedProvider.instance_name}
                      typeDescription={selectedTypeForEdit?.free_tier_long_text}
                      typeDefaultKind={selectedTypeForEdit?.default_free_tier?.kind}
                      status={freeTierStatus[selectedProvider.instance_name]}
                      onReload={() => loadFreeTierStatus(selectedProvider.instance_name)}
                    />
                  </TabsContent>
                </Tabs>

              </div>
            </ScrollArea>
      ) : (
        <div className="flex flex-col h-full rounded-lg border">
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
                      className="group flex items-center gap-3 p-3 rounded-md cursor-pointer hover:bg-muted"
                    >
                      <ProviderIcon providerId={provider.provider_type.toLowerCase()} size={20} />
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate">{provider.instance_name}</p>
                        <p className="text-xs text-muted-foreground">{provider.provider_type}</p>
                      </div>
                      <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7"
                          title="Clone provider"
                          onClick={(e) => handleCloneProvider(e, provider)}
                        >
                          <Copy className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7 text-destructive hover:text-destructive"
                          title="Delete provider"
                          onClick={(e) => {
                            e.stopPropagation()
                            setProviderToDelete(provider)
                          }}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
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
      )}

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
                  // Group providers by category from backend (excluding generic for templates).
                  // Unlisted types (retired, e.g. the legacy llama.cpp server wrapper) are
                  // hidden; their existing instances keep working.
                  const listedTypes = providerTypes.filter(t => t.listed !== false)
                  const embeddedProviders = listedTypes.filter(t => t.category === 'embedded')
                  const localProviders = listedTypes.filter(t => t.category === 'local')
                  const subscriptionProviders = listedTypes.filter(t => t.category === 'subscription')
                  const firstPartyProviders = listedTypes.filter(t => t.category === 'first_party')
                  const thirdPartyProviders = listedTypes.filter(t => t.category === 'third_party')

                  const ProviderButton = ({ type }: { type: ProviderType }) => {
                    const added = providers.filter(p => p.provider_type === type.provider_type).length
                    // Local and Local Embedded providers use the one engine on
                    // this machine, so they are added once.
                    const singleInstance = type.category === 'local' || type.category === 'embedded'
                    const blocked = singleInstance && added > 0
                    return (
                    <button
                      key={type.provider_type}
                      disabled={blocked}
                      title={blocked ? "Already added" : undefined}
                      onClick={() => {
                        setSelectedProviderType(type.provider_type)
                        setDialogPage("configure")
                      }}
                      className={cn(
                        "relative flex flex-col items-center gap-2 p-4 rounded-lg border-2 border-muted",
                        "hover:border-primary hover:bg-accent transition-colors",
                        "focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
                        blocked && "cursor-not-allowed opacity-60 hover:border-muted hover:bg-transparent"
                      )}
                    >
                      {added > 0 && (
                        <span className="absolute right-2 top-2 rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
                          {singleInstance || added === 1 ? "Added" : `Added ×${added}`}
                        </span>
                      )}
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
                  }

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
                        title="Local Embedded Providers"
                        description="LocalRouter runs the engine on your machine and manages models directly in-app: browse and download from Hugging Face, load and unload on demand. Install the engine once with your package manager."
                        providers={embeddedProviders}
                      />
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
                  const genericTypes = providerTypes.filter(t => t.category === 'generic' && t.listed !== false)
                  const genericType =
                    genericTypes.find(t => t.provider_type === customTypeId) ??
                    genericTypes.find(t => t.provider_type === 'openai_compatible') ??
                    genericTypes[0]
                  if (!genericType) {
                    // Distinguish "still loading" from "truly missing" so users
                    // don't see a misleading "not available" message while the
                    // async list_provider_types invoke is in flight.
                    if (providerTypes.length === 0) {
                      return (
                        <div className="flex items-center justify-center gap-2 py-8 text-sm text-muted-foreground">
                          <Loader2 className="h-4 w-4 animate-spin" />
                          <span>Loading providers…</span>
                        </div>
                      )
                    }
                    return (
                      <div className="text-center py-8 text-sm text-muted-foreground">
                        <p>OpenAI-compatible providers are not available in this build.</p>
                      </div>
                    )
                  }
                  return (
                    <div className="space-y-4">
                      {genericTypes.length > 1 && (
                        <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Custom provider kind">
                          {genericTypes.map(t => (
                            <button
                              key={t.provider_type}
                              type="button"
                              role="radio"
                              aria-checked={t.provider_type === genericType.provider_type}
                              onClick={() => setCustomTypeId(t.provider_type)}
                              className={cn(
                                "rounded border p-2 text-left text-sm transition-colors",
                                t.provider_type === genericType.provider_type
                                  ? "border-primary bg-accent"
                                  : "border-muted hover:bg-accent"
                              )}
                            >
                              {t.display_name}
                            </button>
                          ))}
                        </div>
                      )}
                      <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
                        <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                          {genericType.display_name}
                        </p>
                        <p className="text-xs text-blue-700 dark:text-blue-300 mt-1">
                          {genericType.description}
                        </p>
                      </div>
                      <ProviderForm
                        key={genericType.provider_type}
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
                      {selectedTypeForCreate.docs_url && isValidHttpUrl(selectedTypeForCreate.docs_url) && (
                        <button
                          type="button"
                          onClick={() => open(selectedTypeForCreate.docs_url!)}
                          className="mt-0.5 inline-flex items-center gap-1 text-xs text-blue-600 dark:text-blue-400 hover:underline focus:outline-none"
                        >
                          Official documentation
                          <ExternalLink className="h-3 w-3" />
                        </button>
                      )}
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
      <AlertDialog open={!!providerToDelete} onOpenChange={(open) => !open && setProviderToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to remove "{providerToDelete?.instance_name}" from LocalRouter? This only removes the configuration — your provider account and models are not affected.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

    </>
  )
}
