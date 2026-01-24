import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Server, CheckCircle, XCircle, Plus } from "lucide-react"
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

interface HealthStatus {
  healthy: boolean
  latency_ms?: number
  error?: string
}

interface ProvidersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
}

export function ProvidersPanel({
  selectedId,
  onSelect,
}: ProvidersPanelProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [healthStatus, setHealthStatus] = useState<Record<string, HealthStatus>>({})
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")

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

    const unsubscribe = listen("providers-changed", () => {
      loadProviders()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  const loadProviders = async () => {
    try {
      setLoading(true)
      const providerList = await invoke<Provider[]>("list_provider_instances")
      setProviders(providerList)

      // Check health for each provider
      for (const provider of providerList) {
        checkHealth(provider.instance_name)
      }
    } catch (error) {
      console.error("Failed to load providers:", error)
    } finally {
      setLoading(false)
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

  const checkHealth = async (instanceName: string) => {
    try {
      const status = await invoke<HealthStatus>("check_provider_health", {
        instanceName,
      })
      setHealthStatus((prev) => ({ ...prev, [instanceName]: status }))
    } catch (error) {
      setHealthStatus((prev) => ({
        ...prev,
        [instanceName]: { healthy: false, error: "Health check failed" },
      }))
    }
  }

  const handleToggle = async (provider: Provider) => {
    try {
      await invoke("update_provider_instance", {
        instanceName: provider.instance_name,
        updates: { enabled: !provider.enabled },
      })
      toast.success(`Provider ${provider.enabled ? "disabled" : "enabled"}`)
      loadProviders()
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
      loadProviders()
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
      await invoke("add_provider_instance", {
        instanceName,
        providerType: selectedProviderType,
        config,
      })
      toast.success("Provider created")
      setCreateDialogOpen(false)
      setSelectedProviderType("")
      loadProviders()
      onSelect(instanceName)
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
      loadProviders()
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
                        {health && (
                          <div
                            className={cn(
                              "h-2 w-2 rounded-full",
                              health.healthy ? "bg-green-500" : "bg-red-500"
                            )}
                            title={health.healthy ? `${health.latency_ms}ms` : health.error}
                          />
                        )}
                        {!provider.enabled && (
                          <Badge variant="secondary" className="text-xs">Off</Badge>
                        )}
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
                        commonActions.refresh(() => checkHealth(selectedProvider.instance_name)),
                        commonActions.delete(() => openDeleteDialog(selectedProvider)),
                      ]}
                    />
                  </div>
                </div>

                {/* Health Status */}
                {healthStatus[selectedProvider.instance_name] && (
                  <Card>
                    <CardHeader className="pb-3">
                      <CardTitle className="text-sm">Health Status</CardTitle>
                    </CardHeader>
                    <CardContent>
                      {healthStatus[selectedProvider.instance_name].healthy ? (
                        <div className="flex items-center gap-2 text-green-600">
                          <CheckCircle className="h-4 w-4" />
                          <span>Healthy</span>
                          {healthStatus[selectedProvider.instance_name].latency_ms && (
                            <span className="text-muted-foreground">
                              ({healthStatus[selectedProvider.instance_name].latency_ms}ms)
                            </span>
                          )}
                        </div>
                      ) : (
                        <div className="flex items-center gap-2 text-red-600">
                          <XCircle className="h-4 w-4" />
                          <span>Unhealthy</span>
                          {healthStatus[selectedProvider.instance_name].error && (
                            <span className="text-muted-foreground">
                              ({healthStatus[selectedProvider.instance_name].error})
                            </span>
                          )}
                        </div>
                      )}
                    </CardContent>
                  </Card>
                )}

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
