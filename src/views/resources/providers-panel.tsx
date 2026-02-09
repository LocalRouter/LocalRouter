import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { CheckCircle, XCircle, AlertCircle, Plus, Loader2, RefreshCw, FlaskConical, Grid, Settings, ArrowLeft, Eye, EyeOff } from "lucide-react"
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

  // Detail tab state
  const [detailTab, setDetailTab] = useState("info")

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
                              if (!ms) return ""
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
                                <div className="flex items-center gap-2 text-yellow-600">
                                  <AlertCircle className="h-4 w-4" />
                                  <span>Degraded</span>
                                  {health.latency_ms != null && (
                                    <span className="text-muted-foreground">
                                      ({formatLatency(health.latency_ms)})
                                    </span>
                                  )}
                                  {health.error && (
                                    <span className="text-muted-foreground">- {health.error}</span>
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
                              <div className="flex items-center gap-2 text-red-600">
                                <XCircle className="h-4 w-4" />
                                <span>Unhealthy</span>
                                {health.error && (
                                  <span className="text-muted-foreground">- {health.error}</span>
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

                      {/* Enable/Disable */}
                      <Card>
                        <CardHeader>
                          <CardTitle>Enable Provider</CardTitle>
                          <CardDescription>
                            When disabled, this provider will not be used for routing requests
                          </CardDescription>
                        </CardHeader>
                        <CardContent>
                          <div className="flex items-center gap-3">
                            <Switch
                              checked={selectedProvider.enabled}
                              onCheckedChange={() => handleToggle(selectedProvider)}
                            />
                            <span className="text-sm">
                              {selectedProvider.enabled ? "Enabled" : "Disabled"}
                            </span>
                          </div>
                        </CardContent>
                      </Card>

                      {/* Danger Zone */}
                      <Card className="border-red-200 dark:border-red-900">
                        <CardHeader>
                          <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                          <CardDescription>
                            Irreversible actions for this provider
                          </CardDescription>
                        </CardHeader>
                        <CardContent>
                          <div className="flex items-center justify-between">
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
    </>
  )
}
