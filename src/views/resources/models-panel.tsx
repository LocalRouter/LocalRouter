
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Cpu, ExternalLink } from "lucide-react"
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { cn } from "@/lib/utils"
import { ModelPricingBadge, FREE_TIER_LABELS } from "@/components/shared/model-pricing-badge"
import type { ProviderFreeTierStatus } from "@/types/tauri-commands"

interface Model {
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

interface ModelsPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ModelsPanel({
  selectedId,
  onSelect,
  onViewChange,
}: ModelsPanelProps) {
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")
  const [filterCapability, setFilterCapability] = useState<string>("all")
  const [filterPrice, setFilterPrice] = useState<string>("all")
  const [filterFreeTier, setFilterFreeTier] = useState<string>("all")
  const [freeTierStatuses, setFreeTierStatuses] = useState<Record<string, ProviderFreeTierStatus>>({})

  useEffect(() => {
    loadModels()
    loadFreeTierStatuses()

    const unsubscribe = listen("models-changed", () => {
      loadModels()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  const loadModels = async () => {
    try {
      setLoading(true)
      const modelList = await invoke<Model[]>("list_all_models_detailed")
      setModels(modelList)
    } catch (error) {
      console.error("Failed to load models:", error)
      try {
        const basicList = await invoke<Array<{ id: string; provider: string }>>("list_all_models")
        setModels(basicList.map((m) => ({
          model_id: m.id,
          provider_instance: m.provider,
          provider_type: m.provider.split("/")[0] || "unknown",
          capabilities: [],
          context_window: 0,
          supports_streaming: true,
        })))
      } catch (fallbackError) {
        console.error("Failed to load models (fallback):", fallbackError)
      }
    } finally {
      setLoading(false)
    }
  }

  const loadFreeTierStatuses = async () => {
    try {
      const statuses = await invoke<ProviderFreeTierStatus[]>("get_free_tier_status")
      const map: Record<string, ProviderFreeTierStatus> = {}
      for (const s of statuses) {
        map[s.provider_instance] = s
      }
      setFreeTierStatuses(map)
    } catch (error) {
      console.error("Failed to load free tier statuses:", error)
    }
  }

  const allCapabilities = Array.from(new Set(models.flatMap((m) => m.capabilities)))

  const filteredModels = models.filter((model) => {
    const matchesSearch =
      model.model_id.toLowerCase().includes(search.toLowerCase()) ||
      model.provider_instance.toLowerCase().includes(search.toLowerCase())

    if (!matchesSearch) return false

    if (filterCapability !== "all" && !model.capabilities.includes(filterCapability)) return false

    // Price filter
    if (filterPrice !== "all") {
      const price = model.input_price_per_million
      const ftKind = freeTierStatuses[model.provider_instance]?.free_tier?.kind
      if (filterPrice === "free") {
        if (ftKind !== "always_free_local" && ftKind !== "subscription" && (price == null || price > 0)) return false
      } else if (filterPrice === "under1") {
        if (price == null || price >= 1) return false
      } else if (filterPrice === "1to10") {
        if (price == null || price < 1 || price > 10) return false
      } else if (filterPrice === "over10") {
        if (price == null || price <= 10) return false
      }
    }

    // Free tier filter
    if (filterFreeTier !== "all") {
      const ftKind = freeTierStatuses[model.provider_instance]?.free_tier?.kind
      const hasFreeTier = ftKind && ftKind !== "none"
      if (filterFreeTier === "has_free" && !hasFreeTier) return false
      if (filterFreeTier === "no_free" && hasFreeTier) return false
    }

    return true
  })

  const selectedModel = models.find(
    (m) => `${m.provider_instance}/${m.model_id}` === selectedId
  )

  const formatContextWindow = (tokens: number) => {
    if (tokens === 0) return "N/A"
    if (tokens >= 1000000) return `${(tokens / 1000000).toFixed(1)}M`
    if (tokens >= 1000) return `${(tokens / 1000).toFixed(0)}K`
    return `${tokens}`
  }

  const selectedFreeTier = selectedModel ? freeTierStatuses[selectedModel.provider_instance] : null

  return (
    <ResizablePanelGroup direction="horizontal" className="h-full rounded-lg border">
      {/* List Panel */}
      <ResizablePanel defaultSize={35} minSize={25}>
        <div className="flex flex-col h-full">
          <div className="p-4 border-b space-y-2">
            <Input
              placeholder="Search models..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            <Select value={filterCapability} onValueChange={setFilterCapability}>
              <SelectTrigger>
                <SelectValue placeholder="Filter by capability" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All capabilities</SelectItem>
                {allCapabilities.map((cap) => (
                  <SelectItem key={cap} value={cap}>
                    {cap}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex gap-2">
              <Select value={filterPrice} onValueChange={setFilterPrice}>
                <SelectTrigger className="flex-1">
                  <SelectValue placeholder="Price range" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All prices</SelectItem>
                  <SelectItem value="free">Free</SelectItem>
                  <SelectItem value="under1">Under $1/M</SelectItem>
                  <SelectItem value="1to10">$1-10/M</SelectItem>
                  <SelectItem value="over10">Over $10/M</SelectItem>
                </SelectContent>
              </Select>
              <Select value={filterFreeTier} onValueChange={setFilterFreeTier}>
                <SelectTrigger className="flex-1">
                  <SelectValue placeholder="Free tier" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All</SelectItem>
                  <SelectItem value="has_free">Has free tier</SelectItem>
                  <SelectItem value="no_free">No free tier</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <ScrollArea className="flex-1">
            <div className="p-2 space-y-1">
              {loading ? (
                <p className="text-sm text-muted-foreground p-4">Loading...</p>
              ) : filteredModels.length === 0 ? (
                <p className="text-sm text-muted-foreground p-4">No models found</p>
              ) : (
                filteredModels.map((model) => {
                  const modelKey = `${model.provider_instance}/${model.model_id}`
                  const ftStatus = freeTierStatuses[model.provider_instance]
                  return (
                    <div
                      key={modelKey}
                      onClick={() => onSelect(modelKey)}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedId === modelKey ? "bg-accent" : "hover:bg-muted"
                      )}
                    >
                      <Cpu className="h-4 w-4 text-muted-foreground flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate text-sm">{model.model_id}</p>
                        <p className="text-xs text-muted-foreground truncate">
                          {model.provider_instance}
                        </p>
                      </div>
                      <ModelPricingBadge
                        inputPricePerMillion={model.input_price_per_million}
                        outputPricePerMillion={model.output_price_per_million}
                        freeTierKind={ftStatus?.free_tier}
                      />
                    </div>
                  )
                })
              )}
            </div>
          </ScrollArea>
          <div className="p-3 border-t text-xs text-muted-foreground text-center">
            {filteredModels.length} of {models.length} models
          </div>
        </div>
      </ResizablePanel>

      <ResizableHandle withHandle />

      {/* Detail Panel */}
      <ResizablePanel defaultSize={65}>
        {selectedModel ? (
          <ScrollArea className="h-full">
            <div className="p-6 space-y-6">
              <div className="flex items-start justify-between">
                <div>
                  <h2 className="text-xl font-bold">{selectedModel.model_id}</h2>
                  <p className="text-sm text-muted-foreground">
                    {selectedModel.provider_instance} ({selectedModel.provider_type})
                  </p>
                </div>
                {onViewChange && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => onViewChange("resources", `providers/${selectedModel.provider_instance}`)}
                  >
                    <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
                    View Provider
                  </Button>
                )}
              </div>

              {/* Model Info */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-sm">Model Information</CardTitle>
                </CardHeader>
                <CardContent>
                  <div className="grid grid-cols-2 gap-4 text-sm">
                    <div>
                      <p className="text-muted-foreground">Context Window</p>
                      <p className="font-medium">
                        {formatContextWindow(selectedModel.context_window)} tokens
                      </p>
                    </div>
                    <div>
                      <p className="text-muted-foreground">Streaming</p>
                      <p className="font-medium">
                        {selectedModel.supports_streaming ? "Supported" : "Not supported"}
                      </p>
                    </div>
                    {selectedModel.parameter_count && (
                      <div>
                        <p className="text-muted-foreground">Parameters</p>
                        <p className="font-medium">{selectedModel.parameter_count}</p>
                      </div>
                    )}
                    {selectedModel.pricing_source && (
                      <div>
                        <p className="text-muted-foreground">Pricing Source</p>
                        <p className="font-medium capitalize">{selectedModel.pricing_source}</p>
                      </div>
                    )}
                  </div>
                </CardContent>
              </Card>

              {/* Pricing */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-sm">Pricing</CardTitle>
                </CardHeader>
                <CardContent>
                  <ModelPricingBadge
                    inputPricePerMillion={selectedModel.input_price_per_million}
                    outputPricePerMillion={selectedModel.output_price_per_million}
                    freeTierKind={selectedFreeTier?.free_tier}
                    variant="full"
                  />
                </CardContent>
              </Card>

              {/* Capabilities */}
              {selectedModel.capabilities.length > 0 && (
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm">Capabilities</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <div className="flex flex-wrap gap-1">
                      {selectedModel.capabilities.map((cap) => (
                        <Badge key={cap} variant="secondary">
                          {cap}
                        </Badge>
                      ))}
                    </div>
                  </CardContent>
                </Card>
              )}

              {/* Provider Free Tier */}
              {selectedFreeTier && selectedFreeTier.free_tier.kind !== "none" && (
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm">Provider Free Tier</CardTitle>
                  </CardHeader>
                  <CardContent className="text-sm space-y-2">
                    <div className="flex items-center gap-2">
                      <span className="inline-block h-2 w-2 rounded-full bg-green-500" />
                      <span className="font-medium">{FREE_TIER_LABELS[selectedFreeTier.free_tier.kind]}</span>
                    </div>
                    <p className="text-muted-foreground">{selectedFreeTier.status_message}</p>
                    {selectedFreeTier.free_tier.kind === "rate_limited_free" && (
                      <div className="grid grid-cols-2 gap-2 text-xs mt-2">
                        {selectedFreeTier.rate_rpm_limit != null && (
                          <div>
                            <span className="text-muted-foreground">RPM: </span>
                            <span>{selectedFreeTier.rate_rpm_used ?? 0}/{selectedFreeTier.rate_rpm_limit}</span>
                          </div>
                        )}
                        {selectedFreeTier.rate_rpd_limit != null && (
                          <div>
                            <span className="text-muted-foreground">RPD: </span>
                            <span>{selectedFreeTier.rate_rpd_used ?? 0}/{selectedFreeTier.rate_rpd_limit}</span>
                          </div>
                        )}
                      </div>
                    )}
                    {selectedFreeTier.free_tier.kind === "credit_based" && selectedFreeTier.credit_budget_usd != null && (
                      <div className="text-xs">
                        <span className="text-muted-foreground">Credits: </span>
                        <span>${(selectedFreeTier.credit_remaining_usd ?? 0).toFixed(2)} / ${selectedFreeTier.credit_budget_usd.toFixed(2)} remaining</span>
                      </div>
                    )}
                  </CardContent>
                </Card>
              )}

            </div>
          </ScrollArea>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a model to view details</p>
          </div>
        )}
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
