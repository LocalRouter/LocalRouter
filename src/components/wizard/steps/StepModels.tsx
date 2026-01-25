/**
 * Step 2: Select Models
 *
 * Model routing configuration with two modes:
 * - Allowed Models: Client sees and chooses from selected models
 * - Auto Route: Client sees only the auto router model, LocalRouter picks the best
 *
 * Also allows adding a provider if none exist.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2, Info, Plus, Bot, Brain, Server } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/Toggle"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  AllowedModelsSelector,
  type AllowedModelsSelection,
  type Model,
} from "@/components/strategy/AllowedModelsSelector"
import { DragThresholdModelSelector } from "@/components/strategy/DragThresholdModelSelector"
import { ThresholdSlider } from "@/components/routellm/ThresholdSlider"
import ProviderForm, { ProviderType } from "@/components/ProviderForm"

type RoutingMode = "allowed" | "auto"

interface StepModelsProps {
  routingMode: RoutingMode
  allowedModels: AllowedModelsSelection
  autoModelName: string
  prioritizedModels: [string, string][]
  routeLLMEnabled: boolean
  routeLLMThreshold: number
  weakModels: [string, string][]
  onRoutingModeChange: (mode: RoutingMode) => void
  onAllowedModelsChange: (selection: AllowedModelsSelection) => void
  onAutoModelNameChange: (name: string) => void
  onPrioritizedModelsChange: (models: [string, string][]) => void
  onRouteLLMEnabledChange: (enabled: boolean) => void
  onRouteLLMThresholdChange: (threshold: number) => void
  onWeakModelsChange: (models: [string, string][]) => void
}

export function StepModels({
  routingMode,
  allowedModels,
  autoModelName,
  prioritizedModels,
  routeLLMEnabled,
  routeLLMThreshold,
  weakModels,
  onRoutingModeChange,
  onAllowedModelsChange,
  onAutoModelNameChange,
  onPrioritizedModelsChange,
  onRouteLLMEnabledChange,
  onRouteLLMThresholdChange,
  onWeakModelsChange,
}: StepModelsProps) {
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)

  // Provider creation state
  const [showAddProvider, setShowAddProvider] = useState(false)
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [selectedProviderType, setSelectedProviderType] = useState<string>("")
  const [isSubmitting, setIsSubmitting] = useState(false)

  useEffect(() => {
    loadModels()
  }, [])

  const loadModels = async () => {
    try {
      setLoading(true)
      const modelList = await invoke<Array<{ id: string; provider: string }>>("list_all_models")
      setModels(modelList.map(m => ({ id: m.id, provider: m.provider })))
    } catch (error) {
      console.error("Failed to load models:", error)
      setModels([])
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

  const handleOpenAddProvider = async () => {
    await loadProviderTypes()
    setShowAddProvider(true)
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
      setShowAddProvider(false)
      setSelectedProviderType("")
      // Reload models after adding provider
      await loadModels()
    } catch (error) {
      toast.error(`Failed to create provider: ${error}`)
    } finally {
      setIsSubmitting(false)
    }
  }

  const selectedTypeForCreate = providerTypes.find((t) => t.provider_type === selectedProviderType)

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (models.length === 0) {
    return (
      <div className="space-y-4">
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-4">
          <div className="flex items-start gap-3">
            <Info className="h-5 w-5 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-amber-700 dark:text-amber-300">
                No models available
              </p>
              <p className="text-sm text-amber-600/90 dark:text-amber-400/90">
                Add a provider to get started with models. You can also continue
                creating this client and configure models later.
              </p>
            </div>
          </div>
        </div>

        <Button onClick={handleOpenAddProvider} className="w-full">
          <Plus className="h-4 w-4 mr-2" />
          Add Provider
        </Button>

        <p className="text-xs text-muted-foreground text-center">
          Default: All future models will be allowed.
        </p>

        {/* Add Provider Dialog */}
        <Dialog open={showAddProvider} onOpenChange={setShowAddProvider}>
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
                  setShowAddProvider(false)
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
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {/* Routing Mode Selector */}
      <div className="space-y-2">
        <Label>Model Routing Mode</Label>
        <Select
          value={routingMode}
          onValueChange={(value) => onRoutingModeChange(value as RoutingMode)}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select routing mode" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="allowed">
              <div className="flex items-center gap-2">
                <Server className="h-4 w-4" />
                <span>Allowed Models</span>
              </div>
            </SelectItem>
            <SelectItem value="auto">
              <div className="flex items-center gap-2">
                <Bot className="h-4 w-4" />
                <span>Auto Route</span>
              </div>
            </SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {routingMode === "allowed"
            ? "Client can choose from the models you select below."
            : "Client sees only the auto router model. LocalRouter picks the best model automatically."}
        </p>
      </div>

      {/* Mode-specific content */}
      {routingMode === "allowed" ? (
        <div className="space-y-4">
          <AllowedModelsSelector
            models={models}
            value={allowedModels}
            onChange={onAllowedModelsChange}
          />
          <div className="flex items-center justify-between pt-2 border-t">
            <p className="text-xs text-muted-foreground">
              Need more models? Add another provider.
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={handleOpenAddProvider}
            >
              <Plus className="h-3 w-3 mr-1" />
              Add Provider
            </Button>
          </div>
        </div>
      ) : (
        <div className="space-y-4">
          {/* Auto Router Model Name */}
          <div className="space-y-2">
            <Label htmlFor="auto-model-name">Auto Router Model Name</Label>
            <Input
              id="auto-model-name"
              value={autoModelName}
              onChange={(e) => onAutoModelNameChange(e.target.value)}
              placeholder="localrouter/auto"
              className="font-mono text-sm"
            />
            <p className="text-xs text-muted-foreground">
              This is the model name clients will see in the models list.
            </p>
          </div>

          {/* Prioritized Models */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Bot className="h-4 w-4 text-primary" />
              <Label>Prioritized Models</Label>
            </div>
            <p className="text-xs text-muted-foreground">
              Models to try in order. Falls back to next on failures (outage, context limit, policy violation).
            </p>
            <DragThresholdModelSelector
              availableModels={models}
              enabledModels={prioritizedModels}
              onChange={onPrioritizedModelsChange}
              disableDragOverlay
            />
          </div>

          {/* Weak Model (RouteLLM) */}
          <div className="space-y-3 rounded-lg border p-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Brain className="h-4 w-4 text-purple-500" />
                <div>
                  <Label className="flex items-center gap-2">
                    Weak Model
                    <span className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-700 dark:text-purple-300 font-medium">
                      EXPERIMENTAL
                    </span>
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    Use weaker models for simpler prompts for faster and cheaper results.
                  </p>
                </div>
              </div>
              <Switch
                checked={routeLLMEnabled}
                onCheckedChange={onRouteLLMEnabledChange}
              />
            </div>

            {routeLLMEnabled && (
              <div className="space-y-4 pt-2">
                <DragThresholdModelSelector
                  availableModels={models}
                  enabledModels={weakModels}
                  onChange={onWeakModelsChange}
                  disableDragOverlay
                />
                <ThresholdSlider
                  value={routeLLMThreshold}
                  onChange={onRouteLLMThresholdChange}
                />
              </div>
            )}
          </div>

          {/* Add Provider button */}
          <div className="flex items-center justify-between pt-2 border-t">
            <p className="text-xs text-muted-foreground">
              Need more models? Add another provider.
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={handleOpenAddProvider}
            >
              <Plus className="h-3 w-3 mr-1" />
              Add Provider
            </Button>
          </div>
        </div>
      )}

      {/* Add Provider Dialog - available when models exist */}
      <Dialog open={showAddProvider} onOpenChange={setShowAddProvider}>
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
                setShowAddProvider(false)
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
    </div>
  )
}
