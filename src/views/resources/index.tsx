import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ProvidersIcon } from "@/components/icons/category-icons"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { useTauriListener } from "@/hooks/useTauriListener"
import { ProvidersPanel, HealthStatus, HealthCheckEvent } from "./providers-panel"
import { ModelsPanel } from "./models-panel"
import { CompatibilityPanel } from "./compatibility-panel"

interface LlmProvidersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

interface CacheItemHealth {
  name: string
  status: string
  latency_ms?: number
  error?: string
}
interface HealthCacheState {
  providers: Record<string, CacheItemHealth>
}

function mapCacheToHealthStatus(providers: Record<string, CacheItemHealth>): Record<string, HealthStatus> {
  const status: Record<string, HealthStatus> = {}
  for (const [name, health] of Object.entries(providers)) {
    status[name] = {
      status: health.status as HealthStatus["status"],
      latency_ms: health.latency_ms,
      error: health.error,
    }
  }
  return status
}

export function ResourcesView({ activeSubTab, onTabChange }: LlmProvidersViewProps) {
  // Lifted health status state - persists across tab switches
  const [healthStatus, setHealthStatus] = useState<Record<string, HealthStatus>>({})

  // Refresh health for a single provider
  const refreshHealth = useCallback(async (instanceName: string) => {
    setHealthStatus((prev) => ({
      ...prev,
      [instanceName]: { status: "pending" },
    }))
    await invoke("check_single_provider_health", { instanceName })
  }, [])

  // Load cached health state on mount (populated by periodic health checks)
  useEffect(() => {
    invoke<HealthCacheState>("get_health_cache").then((cache) => {
      if (!cache?.providers) return
      setHealthStatus(mapCacheToHealthStatus(cache.providers))
    }).catch(() => {})
  }, [])

  // Listen for health check events (individual provider checks)
  useTauriListener<HealthCheckEvent>("provider-health-check", (event) => {
    const { provider_name, status, latency_ms, error_message } = event.payload
    setHealthStatus((prev) => ({
      ...prev,
      [provider_name]: {
        status: status as HealthStatus["status"],
        latency_ms,
        error: error_message,
      },
    }))
  })

  // Listen for global health cache updates (e.g. from sidebar refresh button, periodic checks)
  useTauriListener<HealthCacheState>("health-status-changed", (event) => {
    const { providers } = event.payload
    if (!providers) return
    setHealthStatus((prev) => ({
      ...prev,
      ...mapCacheToHealthStatus(providers),
    }))
  })

  // Parse subTab to determine which resource type and item is selected
  // Format: "providers" or "models"
  // Or: "providers/instance-name" or "models/provider/model-id"
  // Or: "providers/add/provider-type" for opening add dialog
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { resourceType: "providers", itemId: null, addType: null }
    const parts = subTab.split("/")
    const resourceType = parts[0] || "providers"

    // Check for add pattern: "providers/add/OpenAI"
    if (parts[1] === "add" && parts[2]) {
      return { resourceType, itemId: null, addType: parts[2] }
    }

    const itemId = parts.slice(1).join("/") || null
    return { resourceType, itemId, addType: null }
  }

  const { resourceType, itemId, addType } = parseSubTab(activeSubTab)

  const handleResourceChange = (type: string) => {
    onTabChange("resources", type)
  }

  const handleItemSelect = (type: string, id: string | null) => {
    onTabChange("resources", id ? `${type}/${id}` : type)
  }

  // When a specific provider is selected, show detail view with back button
  const providerSelected = resourceType === "providers" && !!itemId

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      {providerSelected ? (
        <div className="flex-shrink-0 pb-2">
          <Button variant="ghost" size="sm" className="gap-1 -ml-2" onClick={() => handleItemSelect("providers", null)}>
            <ArrowLeft className="h-3 w-3" />
            Back to LLM Providers
          </Button>
        </div>
      ) : (
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><ProvidersIcon className="h-6 w-6" />LLM Providers</h1>
          <p className="text-sm text-muted-foreground">
            Manage LLM providers
          </p>
        </div>
      )}

      <Tabs
        value={resourceType}
        onValueChange={handleResourceChange}
        className="flex flex-col flex-1 min-h-0"
      >
        {!providerSelected && (
          <TabsList className="flex-shrink-0 w-fit">
            <TabsTrigger value="providers"><TAB_ICONS.providers className={TAB_ICON_CLASS} />Providers</TabsTrigger>
            <TabsTrigger value="models"><TAB_ICONS.allModels className={TAB_ICON_CLASS} />All Models</TabsTrigger>
            <TabsTrigger value="compatibility"><TAB_ICONS.compatibility className={TAB_ICON_CLASS} />Compatibility</TabsTrigger>
          </TabsList>
        )}

        <TabsContent value="providers" className="flex-1 min-h-0 mt-4">
          <ProvidersPanel
            selectedId={resourceType === "providers" ? itemId : null}
            onSelect={(id) => handleItemSelect("providers", id)}
            healthStatus={healthStatus}
            onHealthInit={() => {}}
            onRefreshHealth={refreshHealth}
            initialAddProviderType={resourceType === "providers" ? addType : null}
            onViewChange={onTabChange}
          />
        </TabsContent>

        {!providerSelected && (
          <>
            <TabsContent value="models" className="flex-1 min-h-0 mt-4">
              <ModelsPanel
                selectedId={resourceType === "models" ? itemId : null}
                onSelect={(id) => handleItemSelect("models", id)}
                onViewChange={onTabChange}
              />
            </TabsContent>

            <TabsContent value="compatibility" className="flex-1 min-h-0 mt-4">
              <CompatibilityPanel />
            </TabsContent>
          </>
        )}
      </Tabs>
    </div>
  )
}
