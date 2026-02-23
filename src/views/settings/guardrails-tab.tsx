import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import {
  Shield,
} from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/Badge"
import { SafetyModelList } from "@/components/guardrails/SafetyModelList"
import { SafetyModelPicker, type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import type {
  GuardrailsConfig,
  SafetyModelConfig,
  UpdateGuardrailsConfigParams,
  AddSafetyModelParams,
  RemoveSafetyModelParams,
  ProviderModelPullProgress,
} from "@/types/tauri-commands"

interface GuardrailsTabProps {
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function GuardrailsTab({ onTabChange }: GuardrailsTabProps) {
  const [config, setConfig] = useState<GuardrailsConfig>({
    scan_requests: true,
    safety_models: [],
    default_confidence_threshold: 0.5,
    parallel_guardrails: true,
  })
  const [isLoading, setIsLoading] = useState(true)

  // Ollama pull progress: providerId:modelName → progress
  const [pullProgress, setPullProgress] = useState<Record<string, ProviderModelPullProgress>>({})

  // Load errors (model_id → error message)
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({})

  const loadConfig = useCallback(async () => {
    try {
      const result = await invoke<GuardrailsConfig>("get_guardrails_config")
      setConfig(result)
    } catch (err) {
      console.error("Failed to load guardrails config:", err)
      toast.error("Failed to load guardrails configuration")
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  // Listen for Ollama pull progress and load error events
  useEffect(() => {
    const unlisteners: (() => void)[] = []

    listen<ProviderModelPullProgress & { provider_id: string; model_name: string }>("provider-model-pull-progress", (event) => {
      const { provider_id, model_name, status, total, completed } = event.payload
      const key = `${provider_id}:${model_name}`
      setPullProgress(prev => ({ ...prev, [key]: { status, total, completed } }))
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ provider_id: string; model_name: string }>("provider-model-pull-complete", (event) => {
      const { provider_id, model_name } = event.payload
      const key = `${provider_id}:${model_name}`
      setPullProgress(prev => { const next = { ...prev }; delete next[key]; return next })
      toast.success(`Model "${model_name}" pulled successfully from ${provider_id}`)
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ provider_id: string; model_name: string; error: string }>("provider-model-pull-failed", (event) => {
      const { provider_id, model_name, error } = event.payload
      const key = `${provider_id}:${model_name}`
      setPullProgress(prev => { const next = { ...prev }; delete next[key]; return next })
      toast.error(`Failed to pull "${model_name}" from ${provider_id}: ${error}`)
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ model_id: string; error: string }>("safety-model-load-failed", (event) => {
      const { model_id, error } = event.payload
      setLoadErrors(prev => ({ ...prev, [model_id]: error }))
      toast.error(`Safety model "${model_id}" failed to load: ${error}`)
    }).then(unlisten => unlisteners.push(unlisten))

    return () => { unlisteners.forEach(fn => fn()) }
  }, [])

  const rebuildEngine = useCallback(async () => {
    try {
      await invoke("rebuild_safety_engine")
    } catch (err) {
      console.error("Failed to rebuild safety engine:", err)
    }
  }, [])

  const saveConfig = async (newConfig: GuardrailsConfig) => {
    try {
      await invoke("update_guardrails_config", {
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateGuardrailsConfigParams as Record<string, unknown>)
      setConfig(newConfig)
      toast.success("GuardRails configuration saved")
      rebuildEngine()
    } catch (err) {
      console.error("Failed to save guardrails config:", err)
      toast.error("Failed to save configuration")
    }
  }

  const handleRemoveModel = async (modelId: string) => {
    try {
      await invoke("remove_safety_model", {
        modelId,
      } satisfies RemoveSafetyModelParams as Record<string, unknown>)
      toast.success("Safety model removed")
      await loadConfig()
      rebuildEngine()
    } catch (err) {
      toast.error(`Failed to remove model: ${err}`)
    }
  }

  const handlePickerSelect = async (selection: PickerSelection) => {
    const modelConfig: SafetyModelConfig = {
      id: "",
      label: selection.label,
      model_type: selection.modelType,
      provider_id: selection.providerId,
      model_name: selection.modelName,
      confidence_threshold: null,
      enabled_categories: null,
      prompt_template: null,
      safe_indicator: null,
      output_regex: null,
      category_mapping: null,
    }

    try {
      await invoke<string>("add_safety_model", {
        configJson: JSON.stringify(modelConfig),
      } satisfies AddSafetyModelParams as Record<string, unknown>)
      await loadConfig()
      rebuildEngine()
      toast.success("Provider model added")
    } catch (err) {
      toast.error(`Failed to add model: ${err}`)
    }
  }

  if (isLoading) {
    return <div className="text-sm text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-4">
      {/* Card: GuardRails */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Shield className="h-5 w-5 text-red-500" />
            <CardTitle>GuardRails</CardTitle>
            <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
          </div>
          <CardDescription>
            Configure safety models via external providers (Ollama, LM Studio, etc.). Enable guardrails per-client in the Clients view,
            then test in{" "}
            {onTabChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onTabChange("try-it-out", "guardrails")}
              >
                Try It Out &rarr; GuardRails
              </button>
            ) : (
              <>Try It Out &rarr; GuardRails</>
            )}.
          </CardDescription>
        </CardHeader>
      </Card>

      {/* Card: Settings */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Settings</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Parallel Scanning */}
          <div className="flex items-center justify-between">
            <div>
              <Label>Parallel Scanning</Label>
              <p className="text-xs text-muted-foreground">
                Run safety checks alongside the LLM request for lower latency. Automatically falls back to sequential scanning for models with side effects (e.g. Perplexity Sonar).
              </p>
            </div>
            <Switch
              checked={config.parallel_guardrails}
              onCheckedChange={(checked) => saveConfig({ ...config, parallel_guardrails: checked })}
            />
          </div>
        </CardContent>
      </Card>

      {/* Card: Safety Models */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Safety Models</CardTitle>
          <CardDescription>
            Add a safety model from a configured provider (Ollama, LM Studio, or any OpenAI-compatible API).
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <SafetyModelPicker
            existingModelIds={config.safety_models.map(m => m.id)}
            onSelect={handlePickerSelect}
          />

          <SafetyModelList
            models={config.safety_models}
            pullProgress={pullProgress}
            loadErrors={loadErrors}
            onRemove={handleRemoveModel}
          />
        </CardContent>
      </Card>

    </div>
  )
}
