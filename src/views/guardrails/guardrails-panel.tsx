import { useState } from "react"
import { Plus } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { SafetyModelPicker, type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import { GuardrailsTab as GuardrailsTryItOut } from "@/views/try-it-out/guardrails-tab"
import { cn } from "@/lib/utils"
import type {
  SafetyModelConfig,
} from "@/types/tauri-commands"

interface GuardrailsPanelProps {
  models: SafetyModelConfig[]
  loadErrors: Record<string, string>
  onPickerSelect: (selection: PickerSelection) => void
  onRemoveModel: (modelId: string) => void
}

export function GuardrailsPanel({
  models,
  loadErrors,
  onPickerSelect,
  onRemoveModel,
}: GuardrailsPanelProps) {
  const [search, setSearch] = useState("")
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null)
  const [detailTab, setDetailTab] = useState("info")
  const [pickerOpen, setPickerOpen] = useState(false)

  // Build keys for already-added models so the picker can disable them
  const existingModelKeys = models.map(m => `provider:${m.model_type}:${m.provider_id}`)

  const filteredModels = models.filter((m) =>
    m.label.toLowerCase().includes(search.toLowerCase()) ||
    (m.provider_id && m.provider_id.toLowerCase().includes(search.toLowerCase())) ||
    (m.model_name && m.model_name.toLowerCase().includes(search.toLowerCase()))
  )

  const selectedModel = models.find((m) => m.id === selectedModelId)

  const handlePickerSelect = (selection: PickerSelection) => {
    onPickerSelect(selection)
    setPickerOpen(false)
  }

  return (
    <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
      {/* List Panel */}
      <ResizablePanel defaultSize={21} minSize={15}>
        <div className="flex flex-col h-full">
          <div className="p-4 border-b">
            <div className="flex items-center gap-2">
              <Input
                placeholder="Search models..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="flex-1"
              />
              <Button size="icon" onClick={() => setPickerOpen(true)}>
                <Plus className="h-4 w-4" />
              </Button>
            </div>
          </div>
          <ScrollArea className="flex-1">
            <div className="p-2 space-y-1">
              {filteredModels.length === 0 ? (
                <p className="text-sm text-muted-foreground p-4">
                  {models.length === 0 ? "No safety models configured" : "No models match search"}
                </p>
              ) : (
                filteredModels.map((model) => {
                  const hasError = !!loadErrors[model.id]
                  return (
                    <div
                      key={model.id}
                      onClick={() => {
                        setSelectedModelId(model.id)
                        setDetailTab("info")
                      }}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedModelId === model.id ? "bg-accent" : "hover:bg-muted"
                      )}
                    >
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate">{model.label}</p>
                        <p className="text-xs text-muted-foreground truncate">
                          {model.provider_id}/{model.model_name}
                        </p>
                      </div>
                      {hasError && (
                        <Badge variant="destructive" className="text-[10px] shrink-0">Error</Badge>
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
      <ResizablePanel defaultSize={79}>
        {selectedModel ? (
          <ScrollArea className="h-full">
            <div className="p-6 space-y-6">
              <div>
                <h2 className="text-xl font-bold">{selectedModel.label}</h2>
                <p className="text-sm text-muted-foreground">
                  {selectedModel.provider_id}/{selectedModel.model_name}
                </p>
              </div>

              <Tabs value={detailTab} onValueChange={setDetailTab}>
                <TabsList>
                  <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
                  <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
                </TabsList>

                <TabsContent value="info">
                  <div className="space-y-4">
                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">Configuration</CardTitle>
                      </CardHeader>
                      <CardContent className="space-y-3">
                        <div className="grid grid-cols-2 gap-3 text-sm">
                          <div>
                            <span className="text-muted-foreground">Provider:</span>{" "}
                            <span className="font-medium">{selectedModel.provider_id}</span>
                          </div>
                          <div>
                            <span className="text-muted-foreground">Model:</span>{" "}
                            <span className="font-medium">{selectedModel.model_name}</span>
                          </div>
                          <div>
                            <span className="text-muted-foreground">Type:</span>{" "}
                            <span className="font-medium capitalize">{selectedModel.model_type}</span>
                          </div>
                          {selectedModel.confidence_threshold != null && (
                            <div>
                              <span className="text-muted-foreground">Confidence Threshold:</span>{" "}
                              <span className="font-medium">{selectedModel.confidence_threshold}</span>
                            </div>
                          )}
                        </div>

                        {selectedModel.enabled_categories && selectedModel.enabled_categories.length > 0 && (
                          <div>
                            <p className="text-sm text-muted-foreground mb-1.5">Enabled Categories</p>
                            <div className="flex flex-wrap gap-1.5">
                              {selectedModel.enabled_categories.map((cat) => (
                                <Badge key={cat} variant="outline" className="text-[10px] capitalize">
                                  {cat.replace(/_/g, " ")}
                                </Badge>
                              ))}
                            </div>
                          </div>
                        )}

                        {loadErrors[selectedModel.id] && (
                          <Card className="border-destructive/50">
                            <CardContent className="pt-4">
                              <p className="text-sm text-destructive">{loadErrors[selectedModel.id]}</p>
                            </CardContent>
                          </Card>
                        )}
                      </CardContent>
                    </Card>

                    {/* Unlink */}
                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">Unlink Model</CardTitle>
                      </CardHeader>
                      <CardContent className="space-y-2">
                        <p className="text-xs text-muted-foreground">
                          Remove this model from GuardRails. The model itself remains available on the provider and can be re-added later.
                        </p>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            onRemoveModel(selectedModel.id)
                            setSelectedModelId(null)
                          }}
                        >
                          Unlink
                        </Button>
                      </CardContent>
                    </Card>
                  </div>
                </TabsContent>

                <TabsContent value="try-it-out">
                  <GuardrailsTryItOut
                    forcedMode="specific_model"
                    forcedModelId={selectedModel.id}
                    hideModeSwitcher
                  />
                </TabsContent>
              </Tabs>
            </div>
          </ScrollArea>
        ) : (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
            <FEATURES.guardrails.icon className="h-12 w-12 opacity-30" />
            <div className="text-center">
              <p className="font-medium">Select a model to view details</p>
              <p className="text-sm">or add a new one with the + button</p>
            </div>
          </div>
        )}
      </ResizablePanel>

      {/* Add Model Dialog */}
      <Dialog open={pickerOpen} onOpenChange={setPickerOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Safety Model</DialogTitle>
          </DialogHeader>
          <SafetyModelPicker
            existingModelIds={existingModelKeys}
            onSelect={handlePickerSelect}
          />
        </DialogContent>
      </Dialog>
    </ResizablePanelGroup>
  )
}
