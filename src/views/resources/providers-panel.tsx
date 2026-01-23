import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Server, CheckCircle, XCircle } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { Input } from "@/components/ui/Input"
import {
  EntityActions,
  commonActions,
  createToggleAction,
} from "@/components/shared/entity-actions"
import { MetricsChart } from "@/components/shared/metrics-chart"
import { cn } from "@/lib/utils"

interface Provider {
  instance_name: string
  provider_type: string
  enabled: boolean
  base_url?: string
}

interface HealthStatus {
  healthy: boolean
  latency_ms?: number
  error?: string
}

interface ProvidersPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  refreshTrigger?: number
}

export function ProvidersPanel({
  selectedId,
  onSelect,
  refreshTrigger = 0,
}: ProvidersPanelProps) {
  const [providers, setProviders] = useState<Provider[]>([])
  const [healthStatus, setHealthStatus] = useState<Record<string, HealthStatus>>({})
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")

  useEffect(() => {
    loadProviders()

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

  const handleDelete = async (provider: Provider) => {
    if (!confirm(`Delete provider "${provider.instance_name}"?`)) return
    try {
      await invoke("remove_provider_instance", {
        instanceName: provider.instance_name,
      })
      toast.success("Provider deleted")
      if (selectedId === provider.instance_name) {
        onSelect(null)
      }
      loadProviders()
    } catch (error) {
      toast.error("Failed to delete provider")
    }
  }

  const filteredProviders = providers.filter((p) =>
    p.instance_name.toLowerCase().includes(search.toLowerCase()) ||
    p.provider_type.toLowerCase().includes(search.toLowerCase())
  )

  const selectedProvider = providers.find((p) => p.instance_name === selectedId)

  return (
    <ResizablePanelGroup direction="horizontal" className="h-full rounded-lg border">
      {/* List Panel */}
      <ResizablePanel defaultSize={35} minSize={25}>
        <div className="flex flex-col h-full">
          <div className="p-4 border-b">
            <Input
              placeholder="Search providers..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
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
                      createToggleAction(selectedProvider.enabled, () =>
                        handleToggle(selectedProvider)
                      ),
                      commonActions.refresh(() => checkHealth(selectedProvider.instance_name)),
                      commonActions.delete(() => handleDelete(selectedProvider)),
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

              {/* Metrics */}
              <MetricsChart
                title="Provider Metrics"
                scope="provider"
                scopeId={selectedProvider.instance_name}
                defaultMetricType="requests"
                refreshTrigger={refreshTrigger}
                height={250}
              />
            </div>
          </ScrollArea>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a provider to view details</p>
          </div>
        )}
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
