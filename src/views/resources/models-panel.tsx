
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Cpu } from "lucide-react"
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { cn } from "@/lib/utils"

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
}

interface ModelsPanelProps {
  selectedId: string | null
  onSelect: (id: string | null) => void
}

export function ModelsPanel({
  selectedId,
  onSelect,
}: ModelsPanelProps) {
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState("")
  const [filterCapability, setFilterCapability] = useState<string>("all")

  useEffect(() => {
    loadModels()

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
      // Fallback to basic list
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

  const allCapabilities = Array.from(new Set(models.flatMap((m) => m.capabilities)))

  const filteredModels = models.filter((model) => {
    const matchesSearch =
      model.model_id.toLowerCase().includes(search.toLowerCase()) ||
      model.provider_instance.toLowerCase().includes(search.toLowerCase())

    if (filterCapability === "all") return matchesSearch
    return matchesSearch && model.capabilities.includes(filterCapability)
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

  const formatPrice = (price?: number) => {
    if (!price) return "N/A"
    return `$${price.toFixed(2)}`
  }

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
                  return (
                    <div
                      key={modelKey}
                      onClick={() => onSelect(modelKey)}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedId === modelKey ? "bg-accent" : "hover:bg-muted"
                      )}
                    >
                      <Cpu className="h-4 w-4 text-muted-foreground" />
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate text-sm">{model.model_id}</p>
                        <p className="text-xs text-muted-foreground truncate">
                          {model.provider_instance}
                        </p>
                      </div>
                      {model.context_window > 0 && (
                        <span className="text-xs text-muted-foreground">
                          {formatContextWindow(model.context_window)}
                        </span>
                      )}
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
              <div>
                <h2 className="text-xl font-bold">{selectedModel.model_id}</h2>
                <p className="text-sm text-muted-foreground">
                  {selectedModel.provider_instance} ({selectedModel.provider_type})
                </p>
              </div>

              {/* Model Info */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-sm">Model Information</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
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
                    {(selectedModel.input_price_per_million || selectedModel.output_price_per_million) && (
                      <div>
                        <p className="text-muted-foreground">Price/M tokens</p>
                        <p className="font-medium">
                          {formatPrice(selectedModel.input_price_per_million)} in / {formatPrice(selectedModel.output_price_per_million)} out
                        </p>
                      </div>
                    )}
                  </div>

                  {selectedModel.capabilities.length > 0 && (
                    <div>
                      <p className="text-muted-foreground text-sm mb-2">Capabilities</p>
                      <div className="flex flex-wrap gap-1">
                        {selectedModel.capabilities.map((cap) => (
                          <Badge key={cap} variant="secondary">
                            {cap}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}
                </CardContent>
              </Card>

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
