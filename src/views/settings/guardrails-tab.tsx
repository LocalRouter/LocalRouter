import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import {
  Shield,
  Eye,
  EyeOff,
} from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { SafetyModelList } from "@/components/guardrails/SafetyModelList"
import { SafetyModelPicker, type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { findVariant } from "@/constants/safety-model-variants"
import type {
  GuardrailsConfig,
  SafetyModelConfig,
  SafetyModelDownloadStatus,
  UpdateGuardrailsConfigParams,
  DownloadSafetyModelParams,
  AddSafetyModelParams,
  RemoveSafetyModelParams,
  CheckSafetyModelFileExistsParams,
  DeleteSafetyModelFilesParams,
} from "@/types/tauri-commands"

interface GuardrailsTabProps {
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function GuardrailsTab({ onTabChange }: GuardrailsTabProps) {
  const [config, setConfig] = useState<GuardrailsConfig>({
    scan_requests: true,
    safety_models: [],
    hf_token: null,
    default_confidence_threshold: 0.5,
    idle_timeout_secs: 600,
    context_size: 512,
    parallel_guardrails: true,
  })
  const [isLoading, setIsLoading] = useState(true)

  // HF token visibility
  const [showHfToken, setShowHfToken] = useState(false)

  // Download state
  const [downloadStatuses, setDownloadStatuses] = useState<Record<string, SafetyModelDownloadStatus>>({})
  const [downloadProgress, setDownloadProgress] = useState<Record<string, number>>({})

  // Load errors (model_id → error message)
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({})

  // Model cache state
  const [loadedModelCount, setLoadedModelCount] = useState(0)

  const hasDownloadedGgufModels = Object.values(downloadStatuses).some(s => s.downloaded)

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

  const loadDownloadStatuses = useCallback(async (models: SafetyModelConfig[]) => {
    for (const model of models) {
      if (model.gguf_filename) {
        try {
          const status = await invoke<SafetyModelDownloadStatus>("get_safety_model_download_status", {
            modelId: model.id,
          } as Record<string, unknown>)
          setDownloadStatuses(prev => ({ ...prev, [model.id]: status }))
        } catch {
          // Download status may not be available
        }
      }
    }
  }, [])

  const refreshLoadedModelCount = useCallback(async () => {
    try {
      const count = await invoke<number>("get_guardrails_loaded_model_count")
      setLoadedModelCount(count)
    } catch {
      // ignore
    }
  }, [])

  useEffect(() => {
    loadConfig()
    refreshLoadedModelCount()
  }, [loadConfig, refreshLoadedModelCount])

  useEffect(() => {
    if (config.safety_models.length > 0) {
      loadDownloadStatuses(config.safety_models)
    }
  }, [config.safety_models, loadDownloadStatuses])

  // Listen for download events
  useEffect(() => {
    const unlisteners: (() => void)[] = []

    listen<{ model_id: string; progress: number; downloaded_bytes: number; total_bytes: number; speed_bytes_per_sec: number }>("safety-model-download-progress", (event) => {
      const { model_id, progress } = event.payload
      setDownloadProgress(prev => ({ ...prev, [model_id]: progress * 100 }))
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ model_id: string; file_path: string; file_size: number }>("safety-model-download-complete", (event) => {
      const { model_id, file_path, file_size } = event.payload
      setDownloadProgress(prev => { const next = { ...prev }; delete next[model_id]; return next })
      setDownloadStatuses(prev => ({ ...prev, [model_id]: { downloaded: true, file_path, file_size } }))
      toast.success(`Safety model downloaded successfully`)
      invoke("rebuild_safety_engine").catch(() => {})
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ model_id: string; error: string }>("safety-model-download-failed", (event) => {
      setDownloadProgress(prev => { const next = { ...prev }; delete next[event.payload.model_id]; return next })
      toast.error(`Download failed: ${event.payload.error}`)
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

  const handleDownloadModel = async (modelId: string) => {
    setDownloadProgress(prev => ({ ...prev, [modelId]: 0 }))
    try {
      await invoke("download_safety_model", {
        modelId,
      } satisfies DownloadSafetyModelParams as Record<string, unknown>)
    } catch (err) {
      setDownloadProgress(prev => { const next = { ...prev }; delete next[modelId]; return next })
      toast.error(`Failed to start download: ${err}`)
    }
  }

  const handleRetryCorruptModel = async (modelId: string) => {
    try {
      await invoke("delete_safety_model_files", {
        modelId,
      } satisfies DeleteSafetyModelFilesParams as Record<string, unknown>)
      setLoadErrors(prev => { const next = { ...prev }; delete next[modelId]; return next })
      setDownloadStatuses(prev => { const next = { ...prev }; delete next[modelId]; return next })
      await handleDownloadModel(modelId)
    } catch (err) {
      toast.error(`Failed to delete corrupt model files: ${err}`)
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
    if (selection.type === "provider") {
      // Add a provider-based model
      const modelConfig: SafetyModelConfig = {
        id: "",
        label: selection.label,
        model_type: selection.modelType,
        execution_mode: "provider",
        hf_repo_id: null,
        gguf_filename: null,
        predefined: false,
        provider_id: selection.providerId,
        model_name: selection.modelName,
        requires_auth: false,
        confidence_threshold: null,
        enabled_categories: null,
        prompt_template: null,
        safe_indicator: null,
        output_regex: null,
        category_mapping: null,
        memory_mb: null,
        latency_ms: null,
        disk_size_mb: null,
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
      return
    }

    // Direct download
    const variant = findVariant(selection.variantKey)
    if (!variant) return

    const isDuplicate = config.safety_models.some(
      m => m.hf_repo_id === variant.hfRepoId && m.gguf_filename === variant.ggufFilename
    )
    if (isDuplicate) {
      toast.warning("This model variant is already added")
      return
    }

    const modelConfig: SafetyModelConfig = {
      id: variant.key,
      label: variant.label,
      model_type: variant.modelType,
      execution_mode: "direct_download",
      hf_repo_id: variant.hfRepoId,
      gguf_filename: variant.ggufFilename,
      predefined: false,
      provider_id: null,
      model_name: null,
      requires_auth: false,
      confidence_threshold: null,
      enabled_categories: null,
      prompt_template: null,
      safe_indicator: null,
      output_regex: null,
      category_mapping: null,
      memory_mb: variant.memoryMb ?? null,
      latency_ms: variant.latencyMs ?? null,
      disk_size_mb: variant.diskSizeMb ?? null,
    }

    try {
      const newId = await invoke<string>("add_safety_model", {
        configJson: JSON.stringify(modelConfig),
      } satisfies AddSafetyModelParams as Record<string, unknown>)

      await loadConfig()

      const modelId = newId || variant.key

      // Check if the file is already downloaded on disk before starting a download
      try {
        const status = await invoke<SafetyModelDownloadStatus>("check_safety_model_file_exists", {
          modelId,
          ggufFilename: variant.ggufFilename,
        } satisfies CheckSafetyModelFileExistsParams as Record<string, unknown>)
        if (status.downloaded) {
          setDownloadStatuses(prev => ({ ...prev, [modelId]: status }))
          toast.success("Model already downloaded, added to configuration")
          rebuildEngine()
          return
        }
      } catch {
        // Fall through to download
      }

      handleDownloadModel(modelId)
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
            Download and manage safety models here. Enable guardrails per-client in the Clients view,
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
          {/* HuggingFace Token */}
          <div className="flex items-center justify-between">
            <div>
              <Label>HuggingFace Token</Label>
              <p className="text-xs text-muted-foreground">
                Required for gated models (e.g. Llama Guard).{" "}
                <a
                  href="https://huggingface.co/settings/tokens"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-500 hover:underline"
                >
                  Get token
                </a>
              </p>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type={showHfToken ? "text" : "password"}
                placeholder="hf_..."
                value={config.hf_token || ""}
                onChange={(e) => saveConfig({ ...config, hf_token: e.target.value || null })}
                className="h-8 text-xs font-mono w-48"
              />
              <Button
                variant="ghost"
                size="sm"
                className="h-8 w-8 p-0"
                onClick={() => setShowHfToken(!showHfToken)}
              >
                {showHfToken ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
              </Button>
            </div>
          </div>

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

          {/* Memory Management — only when GGUF models downloaded */}
          {hasDownloadedGgufModels && (
            <div className="border-t pt-4 space-y-4">
              <div className="flex items-center justify-between text-sm">
                <span className="text-muted-foreground">Models in memory:</span>
                <span className="font-medium">{loadedModelCount}</span>
              </div>
              <div className="space-y-2">
                <Label>Auto-Unload After Idle</Label>
                <Select
                  value={config.idle_timeout_secs.toString()}
                  onValueChange={(value) => {
                    saveConfig({ ...config, idle_timeout_secs: parseInt(value) })
                  }}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="300">5 minutes</SelectItem>
                    <SelectItem value="600">10 minutes (recommended)</SelectItem>
                    <SelectItem value="1800">30 minutes</SelectItem>
                    <SelectItem value="3600">1 hour</SelectItem>
                    <SelectItem value="0">Never</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  Automatically unload GGUF models after inactivity to free memory
                </p>
              </div>
              <div className="space-y-2">
                <Label>Context Window Size</Label>
                <Select
                  value={config.context_size.toString()}
                  onValueChange={(value) => {
                    saveConfig({ ...config, context_size: parseInt(value) })
                  }}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="256">256 tokens (fastest, short input only)</SelectItem>
                    <SelectItem value="512">512 tokens (recommended)</SelectItem>
                    <SelectItem value="1024">1024 tokens (longer input)</SelectItem>
                    <SelectItem value="2048">2048 tokens (max input, slower)</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  Max tokens per inference. Larger values handle longer input but use more GPU memory. Requires engine rebuild.
                </p>
              </div>
              {loadedModelCount > 0 && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={async () => {
                    try {
                      await invoke("unload_all_safety_models")
                      setLoadedModelCount(0)
                      toast.success("All safety models unloaded from memory")
                    } catch (err) {
                      toast.error(`Failed to unload models: ${err}`)
                    }
                  }}
                >
                  Unload All Now
                </Button>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Card: Safety Models */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Safety Models</CardTitle>
          <CardDescription>
            Download a model directly or use one via an existing provider.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <SafetyModelPicker
            existingModelIds={config.safety_models.map(m => m.id)}
            downloadStatuses={downloadStatuses}
            onSelect={handlePickerSelect}
          />

          <SafetyModelList
            models={config.safety_models}
            downloadStatuses={downloadStatuses}
            downloadProgress={downloadProgress}
            loadErrors={loadErrors}
            onRemove={handleRemoveModel}
            onRetryDownload={handleDownloadModel}
            onRetryCorruptModel={handleRetryCorruptModel}
          />
        </CardContent>
      </Card>

    </div>
  )
}
