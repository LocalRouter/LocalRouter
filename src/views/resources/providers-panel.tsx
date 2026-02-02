import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { CheckCircle, XCircle, AlertCircle, Plus, Loader2, RefreshCw, FlaskConical } from "lucide-react"
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
  const [selectedProviderType, setSelectedProviderType] = useState<string>("")
  const [isSubmitting, setIsSubmitting] = useState(false)

  // Handle initial add provider type from navigation
  useEffect(() => {
    if (initialAddProviderType && providerTypes.length > 0) {
      const typeExists = providerTypes.some(t => t.provider_type === initialAddProviderType)
      if (typeExists) {
        setSelectedProviderType(initialAddProviderType)
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

  // Reset detail tab when selection changes
  useEffect(() => {
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

  const handleEditProvider = async (_instanceName: string, config: Record<string, string>) => {
    if (!selectedProvider) return
    setIsSubmitting(true)
    try {
      await invoke("update_provider_instance", {
        instanceName: selectedProvider.instance_name,
        updates: { config },
      })
      toast.success("Provider updated")
      loadProvidersOnly()
    } catch (error) {
      toast.error(`Failed to update provider: ${error}`)
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
                      {/* Inline Edit Form */}
                      {selectedTypeForEdit && (
                        <Card>
                          <CardHeader>
                            <CardTitle>Provider Configuration</CardTitle>
                            <CardDescription>
                              Update the configuration for this provider
                            </CardDescription>
                          </CardHeader>
                          <CardContent>
                            <ProviderForm
                              mode="edit"
                              providerType={selectedTypeForEdit}
                              initialInstanceName={selectedProvider.instance_name}
                              initialConfig={selectedProvider.config || {}}
                              onSubmit={handleEditProvider}
                              onCancel={() => setDetailTab("info")}
                              isSubmitting={isSubmitting}
                            />
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
      <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
        <DialogContent className={cn(
          "max-h-[90vh] overflow-y-auto",
          !selectedProviderType ? "max-w-2xl" : "max-w-lg"
        )}>
          <DialogHeader>
            <DialogTitle>Add Provider</DialogTitle>
          </DialogHeader>

          {!selectedProviderType ? (
            (() => {
              // Group providers by category from backend
              const genericProviders = providerTypes.filter(t => t.category === 'generic')
              const localProviders = providerTypes.filter(t => t.category === 'local')
              const subscriptionProviders = providerTypes.filter(t => t.category === 'subscription')
              const firstPartyProviders = providerTypes.filter(t => t.category === 'first_party')
              const thirdPartyProviders = providerTypes.filter(t => t.category === 'third_party')

              const ProviderButton = ({ type }: { type: ProviderType }) => (
                <button
                  key={type.provider_type}
                  onClick={() => setSelectedProviderType(type.provider_type)}
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
                    title="Generic / Custom"
                    description="Connect to any OpenAI-compatible API endpoint"
                    providers={genericProviders}
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
            })()
          ) : selectedTypeForCreate ? (
            <ProviderForm
              mode="create"
              providerType={selectedTypeForCreate}
              onSubmit={handleCreateProvider}
              onCancel={() => {
                setCreateDialogOpen(false)
                setSelectedProviderType("")
              }}
              isSubmitting={isSubmitting}
            />
          ) : null}

          {selectedProviderType && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setSelectedProviderType("")}
              className="mt-2"
            >
              Back to provider selection
            </Button>
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
