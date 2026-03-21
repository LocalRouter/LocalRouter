import { useState } from "react"
import { Plus, Loader2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Progress } from "@/components/ui/progress"
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
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Switch } from "@/components/ui/switch"
import { Label } from "@/components/ui/label"
import { SafetyModelPicker, type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import { GuardrailsTab as GuardrailsTryItOut } from "@/views/try-it-out/guardrails-tab"
import { cn } from "@/lib/utils"
import type {
  SafetyModelConfig,
} from "@/types/tauri-commands"

interface GuardrailsPanelProps {
  models: SafetyModelConfig[]
  loadErrors: Record<string, string>
  pullProgress?: Record<string, { progress: number; status: string }>
  onPickerSelect: (selection: PickerSelection) => void
  onRemoveModel: (modelId: string) => void
  onToggleModel: (modelId: string, enabled: boolean) => void
}

export function GuardrailsPanel({
  models,
  loadErrors,
  pullProgress = {},
  onPickerSelect,
  onRemoveModel,
  onToggleModel,
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
                  const pullKey = `${model.provider_id}:${model.model_name}`
                  const pulling = pullProgress[pullKey]
                  return (
                    <div
                      key={model.id}
                      onClick={() => {
                        setSelectedModelId(model.id)
                        setDetailTab("info")
                      }}
                      className={cn(
                        "flex flex-col gap-1 p-3 rounded-md cursor-pointer",
                        selectedModelId === model.id ? "bg-accent" : "hover:bg-muted",
                        !model.enabled && !pulling && "opacity-50"
                      )}
                    >
                      <div className="flex items-center gap-3">
                        <div className="flex-1 min-w-0">
                          <p className="font-medium truncate">{model.label}</p>
                          <p className="text-xs text-muted-foreground truncate">
                            {model.provider_id}/{model.model_name}
                          </p>
                        </div>
                        {pulling ? (
                          <Badge variant="default" className="text-[10px] shrink-0 gap-1">
                            <Loader2 className="h-2.5 w-2.5 animate-spin" />
                            Pulling
                          </Badge>
                        ) : !model.enabled ? (
                          <Badge variant="secondary" className="text-[10px] shrink-0">Disabled</Badge>
                        ) : null}
                        {hasError && (
                          <Badge variant="destructive" className="text-[10px] shrink-0">Error</Badge>
                        )}
                      </div>
                      {pulling && (
                        <div className="space-y-1">
                          <Progress value={pulling.progress >= 0 ? pulling.progress : undefined} className="h-1" />
                          <p className="text-[10px] text-muted-foreground truncate">{pulling.status}</p>
                        </div>
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
                  <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
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
                  </div>
                </TabsContent>

                <TabsContent value="try-it-out">
                  <GuardrailsTryItOut
                    forcedMode="specific_model"
                    forcedModelId={selectedModel.id}
                    hideModeSwitcher
                  />
                </TabsContent>

                <TabsContent value="settings">
                  <div className="space-y-4">
                    {/* Enable/Disable */}
                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">Model Status</CardTitle>
                      </CardHeader>
                      <CardContent>
                        <div className="flex items-center justify-between">
                          <div>
                            <Label>Enabled</Label>
                            <p className="text-sm text-muted-foreground">
                              {selectedModel.enabled
                                ? "This model is active and used for guardrails checks."
                                : "This model is disabled and will not be used for guardrails checks."}
                            </p>
                          </div>
                          <Switch
                            checked={selectedModel.enabled}
                            onCheckedChange={(checked) => onToggleModel(selectedModel.id, checked)}
                          />
                        </div>
                      </CardContent>
                    </Card>

                    {/* Danger Zone */}
                    <Card className="border-red-200 dark:border-red-900">
                      <CardHeader>
                        <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                        <CardDescription>
                          Irreversible actions for this safety model
                        </CardDescription>
                      </CardHeader>
                      <CardContent>
                        <div className="flex items-center justify-between">
                          <div>
                            <p className="text-sm font-medium">Remove model</p>
                            <p className="text-sm text-muted-foreground">
                              Remove this model from GuardRails. The model itself remains available on the provider and can be re-added later.
                            </p>
                          </div>
                          <AlertDialog>
                            <AlertDialogTrigger asChild>
                              <Button
                                variant="outline"
                                size="sm"
                                className="border-red-200 text-red-600 hover:bg-red-50 dark:border-red-900 dark:text-red-400 dark:hover:bg-red-950"
                              >
                                Remove
                              </Button>
                            </AlertDialogTrigger>
                            <AlertDialogContent>
                              <AlertDialogHeader>
                                <AlertDialogTitle>Remove Safety Model?</AlertDialogTitle>
                                <AlertDialogDescription>
                                  This will remove &ldquo;{selectedModel.label}&rdquo; from GuardRails. The model itself remains available on the provider and can be re-added later.
                                </AlertDialogDescription>
                              </AlertDialogHeader>
                              <AlertDialogFooter>
                                <AlertDialogCancel>Cancel</AlertDialogCancel>
                                <AlertDialogAction
                                  onClick={() => {
                                    onRemoveModel(selectedModel.id)
                                    setSelectedModelId(null)
                                  }}
                                  className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                >
                                  Remove
                                </AlertDialogAction>
                              </AlertDialogFooter>
                            </AlertDialogContent>
                          </AlertDialog>
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
