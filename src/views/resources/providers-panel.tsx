import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Server, CheckCircle, XCircle, AlertCircle, Plus, Loader2, RefreshCw } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Label } from "@/components/ui/label"
import {
  EntityActions,
  commonActions,
  createToggleAction,
} from "@/components/shared/entity-actions"
import ProviderForm, { ProviderType } from "@/components/ProviderForm"
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
}

export function ProvidersPanel({
  selectedId,
  onSelect,
  healthStatus,
  onHealthInit,
  onRefreshHealth,
}: ProvidersPanelProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")
  const [models, setModels] = useState<ModelInfo[]>([])
  const [modelsLoading, setModelsLoading] = useState(false)

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const [editDialogOpen, setEditDialogOpen] = useState(false)
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false)
  const [providerToDelete, setProviderToDelete] = useState<Provider | null>(null)
  const [providerToEdit, setProviderToEdit] = useState<Provider | null>(null)

  // Create form state
  const [selectedProviderType, setSelectedProviderType] = useState<string>("")
  const [isSubmitting, setIsSubmitting] = useState(false)

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
    if (!providerToEdit) return
    setIsSubmitting(true)
    try {
      await invoke("update_provider_instance", {
        instanceName: providerToEdit.instance_name,
        updates: { config },
      })
      toast.success("Provider updated")
      setEditDialogOpen(false)
      setProviderToEdit(null)
      loadProvidersOnly()
    } catch (error) {
      toast.error(`Failed to update provider: ${error}`)
    } finally {
      setIsSubmitting(false)
    }
  }

  const openEditDialog = (provider: Provider) => {
    setProviderToEdit(provider)
    setEditDialogOpen(true)
  }

  const openDeleteDialog = (provider: Provider) => {
    setProviderToDelete(provider)
    setDeleteDialogOpen(true)
  }

  const filteredProviders = providers.filter((p) =>
    p.instance_name.toLowerCase().includes(search.toLowerCase()) ||
    p.provider_type.toLowerCase().includes(search.toLowerCase())
  )

  const selectedProvider = providers.find((p) => p.instance_name === selectedId)
  const selectedTypeForCreate = providerTypes.find((t) => t.provider_type === selectedProviderType)
  const selectedTypeForEdit = providerToEdit
    ? providerTypes.find((t) => t.provider_type === providerToEdit.provider_type)
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
                      if (!ms) return ""
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
                        <Server className="h-4 w-4 text-muted-foreground" />
                        <div className="flex-1 min-w-0">
                          <p className="font-medium truncate">{provider.instance_name}</p>
                          <p className="text-xs text-muted-foreground">{provider.provider_type}</p>
                        </div>
                        <div className="flex items-center gap-2">
                          {health && health.latency_ms && health.status !== "pending" && (
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
                                  ? `Healthy (${formatLatency(health.latency_ms)})`
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
                    <Badge variant={selectedProvider.enabled ? "success" : "secondary"}>
                      {selectedProvider.enabled ? "Enabled" : "Disabled"}
                    </Badge>
                    <EntityActions
                      actions={[
                        commonActions.edit(() => openEditDialog(selectedProvider)),
                        createToggleAction(selectedProvider.enabled, () =>
                          handleToggle(selectedProvider)
                        ),
                        commonActions.delete(() => openDeleteDialog(selectedProvider)),
                      ]}
                    />
                  </div>
                </div>

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
                            {health.latency_ms && (
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
                            {health.latency_ms && (
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
            </ScrollArea>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
              <Server className="h-12 w-12 opacity-30" />
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
        <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Add Provider</DialogTitle>
          </DialogHeader>

          {!selectedProviderType ? (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>Provider Type</Label>
                <Select value={selectedProviderType} onValueChange={setSelectedProviderType}>
                  <SelectTrigger>
                    <SelectValue placeholder="Select a provider type..." />
                  </SelectTrigger>
                  <SelectContent>
                    {providerTypes.map((type) => (
                      <SelectItem key={type.provider_type} value={type.provider_type}>
                        <div className="flex flex-col">
                          <span className="font-medium">{type.provider_type}</span>
                          <span className="text-xs text-muted-foreground">{type.description}</span>
                        </div>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
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

      {/* Edit Provider Dialog */}
      <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
        <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Edit Provider: {providerToEdit?.instance_name}</DialogTitle>
          </DialogHeader>

          {providerToEdit && selectedTypeForEdit && (
            <ProviderForm
              mode="edit"
              providerType={selectedTypeForEdit}
              initialInstanceName={providerToEdit.instance_name}
              initialConfig={providerToEdit.config || {}}
              onSubmit={handleEditProvider}
              onCancel={() => {
                setEditDialogOpen(false)
                setProviderToEdit(null)
              }}
              isSubmitting={isSubmitting}
            />
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
