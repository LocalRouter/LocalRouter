import { useState, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import {
  Shield,
  Loader2,
  FlaskConical,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Brain,
  Eye,
  EyeOff,
  Plus,
  Trash2,
  Download,
  HardDrive,
  Cloud,
} from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/Toggle"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Textarea } from "@/components/ui/textarea"
import { CategoryActionButton, type CategoryActionState } from "@/components/permissions/CategoryActionButton"
import { AddSafetyModelDialog } from "@/components/guardrails/AddSafetyModelDialog"
import type {
  GuardrailsConfig,
  SafetyModelConfig,
  SafetyCheckResult,
  SafetyVerdict,
  SafetyCategoryInfo,
  CategoryActionEntry,
  SafetyModelDownloadStatus,
  UpdateGuardrailsConfigParams,
  TestSafetyCheckParams,
  GetSafetyModelStatusParams,
  TestSafetyModelParams,
  UpdateCategoryActionsParams,
  DownloadSafetyModelParams,
  GetSafetyModelDownloadStatusParams,
  RemoveSafetyModelParams,
} from "@/types/tauri-commands"

/** Status returned by get_safety_model_status */
interface SafetyModelStatus {
  id: string
  label: string
  model_type: string
  enabled: boolean
  provider_configured: boolean
  model_available: boolean
  downloaded: boolean
  execution_mode: string
}

/** Model type labels for display */
const MODEL_TYPE_LABELS: Record<string, string> = {
  llama_guard: "Llama Guard",
  shield_gemma: "ShieldGemma",
  nemotron: "Nemotron",
  granite_guardian: "Granite Guardian",
  custom: "Custom",
}

function modelTypeLabel(modelType: string): string {
  return MODEL_TYPE_LABELS[modelType] || modelType
}

export function GuardrailsTab() {
  const [config, setConfig] = useState<GuardrailsConfig>({
    enabled: false,
    scan_requests: true,
    scan_responses: false,
    safety_models: [],
    category_actions: [],
    hf_token: null,
    default_confidence_threshold: 0.5,
  })
  const [isLoading, setIsLoading] = useState(true)
  const [modelStatuses, setModelStatuses] = useState<Record<string, SafetyModelStatus>>({})

  // Categories
  const [categories, setCategories] = useState<SafetyCategoryInfo[]>([])

  // HF token visibility
  const [showHfToken, setShowHfToken] = useState(false)

  // Test panel state
  const [testText, setTestText] = useState("")
  const [testResult, setTestResult] = useState<SafetyCheckResult | null>(null)
  const [testing, setTesting] = useState(false)
  const [testRan, setTestRan] = useState(false)

  // Per-model test state
  const [testingModel, setTestingModel] = useState<string | null>(null)
  const [modelTestResult, setModelTestResult] = useState<Record<string, SafetyCheckResult>>({})

  // Add model dialog
  const [addModelOpen, setAddModelOpen] = useState(false)

  // Download state
  const [downloadStatuses, setDownloadStatuses] = useState<Record<string, SafetyModelDownloadStatus>>({})
  const [downloadingModels, setDownloadingModels] = useState<Set<string>>(new Set())
  const [downloadProgress, setDownloadProgress] = useState<Record<string, number>>({})

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

  const loadCategories = useCallback(async () => {
    try {
      const result = await invoke<SafetyCategoryInfo[]>("get_all_safety_categories")
      setCategories(result)
    } catch {
      // Categories may not be available yet
    }
  }, [])

  const loadModelStatuses = useCallback(async (models: SafetyModelConfig[]) => {
    for (const model of models) {
      try {
        const status = await invoke<SafetyModelStatus>("get_safety_model_status", {
          modelId: model.id,
        } satisfies GetSafetyModelStatusParams as Record<string, unknown>)
        setModelStatuses(prev => ({ ...prev, [model.id]: status }))
      } catch {
        // Model status may not be available
      }
    }
  }, [])

  const loadDownloadStatuses = useCallback(async (models: SafetyModelConfig[]) => {
    for (const model of models) {
      if (model.gguf_filename) {
        try {
          const status = await invoke<SafetyModelDownloadStatus>("get_safety_model_download_status", {
            modelId: model.id,
          } satisfies GetSafetyModelDownloadStatusParams as Record<string, unknown>)
          setDownloadStatuses(prev => ({ ...prev, [model.id]: status }))
        } catch {
          // Download status may not be available
        }
      }
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadCategories()
  }, [loadConfig, loadCategories])

  useEffect(() => {
    if (config.safety_models.length > 0) {
      loadModelStatuses(config.safety_models)
      loadDownloadStatuses(config.safety_models)
    }
  }, [config.safety_models, loadModelStatuses, loadDownloadStatuses])

  // Listen for download events
  useEffect(() => {
    const unlisteners: (() => void)[] = []

    listen<{ model_id: string; progress: number }>("safety-model-download-progress", (event) => {
      setDownloadProgress(prev => ({ ...prev, [event.payload.model_id]: event.payload.progress }))
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ model_id: string; file_path: string; file_size: number }>("safety-model-download-complete", (event) => {
      const { model_id, file_path, file_size } = event.payload
      setDownloadingModels(prev => { const next = new Set(prev); next.delete(model_id); return next })
      setDownloadProgress(prev => { const next = { ...prev }; delete next[model_id]; return next })
      setDownloadStatuses(prev => ({ ...prev, [model_id]: { downloaded: true, file_path, file_size } }))
      toast.success(`Safety model downloaded successfully`)
    }).then(unlisten => unlisteners.push(unlisten))

    listen<{ model_id: string; error: string }>("safety-model-download-failed", (event) => {
      setDownloadingModels(prev => { const next = new Set(prev); next.delete(event.payload.model_id); return next })
      setDownloadProgress(prev => { const next = { ...prev }; delete next[event.payload.model_id]; return next })
      toast.error(`Download failed: ${event.payload.error}`)
    }).then(unlisten => unlisteners.push(unlisten))

    return () => { unlisteners.forEach(fn => fn()) }
  }, [])

  const saveConfig = async (newConfig: GuardrailsConfig) => {
    try {
      await invoke("update_guardrails_config", {
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateGuardrailsConfigParams as Record<string, unknown>)
      setConfig(newConfig)
      toast.success("GuardRails configuration saved")
    } catch (err) {
      console.error("Failed to save guardrails config:", err)
      toast.error("Failed to save configuration")
    }
  }

  const toggleEnabled = (enabled: boolean) => {
    saveConfig({ ...config, enabled })
  }

  const toggleScanRequests = (scan_requests: boolean) => {
    saveConfig({ ...config, scan_requests })
  }

  const toggleScanResponses = (scan_responses: boolean) => {
    saveConfig({ ...config, scan_responses })
  }

  const setDefaultThreshold = (value: number) => {
    saveConfig({ ...config, default_confidence_threshold: value })
  }

  const setHfToken = (hf_token: string) => {
    saveConfig({ ...config, hf_token: hf_token || null })
  }

  const toggleModel = (modelId: string, enabled: boolean) => {
    const newModels = config.safety_models.map(m =>
      m.id === modelId ? { ...m, enabled } : m
    )
    saveConfig({ ...config, safety_models: newModels })
  }

  const updateModelField = (modelId: string, field: keyof SafetyModelConfig, value: unknown) => {
    const newModels = config.safety_models.map(m =>
      m.id === modelId ? { ...m, [field]: value } : m
    )
    saveConfig({ ...config, safety_models: newModels })
  }

  // Global default action for all categories (stored as "__global" in category_actions)
  const globalCategoryAction: CategoryActionState = useMemo(() => {
    const entry = config.category_actions.find(a => a.category === "__global")
    return (entry?.action as CategoryActionState) ?? "ask"
  }, [config.category_actions])

  // Filter categories to only those supported by enabled models
  const enabledModelTypes = useMemo(() => {
    return new Set(config.safety_models.filter(m => m.enabled).map(m => m.model_type))
  }, [config.safety_models])

  const enabledCategories = useMemo(() => {
    return categories.filter(cat =>
      cat.supported_by?.some(modelType => enabledModelTypes.has(modelType))
    )
  }, [categories, enabledModelTypes])

  const getCategoryAction = (category: string): CategoryActionState => {
    const entry = config.category_actions.find(a => a.category === category)
    return (entry?.action as CategoryActionState) ?? undefined!
  }

  const isCategoryExplicitlySet = (category: string): boolean => {
    return config.category_actions.some(a => a.category === category && a.category !== "__global")
  }

  // Compute child rollup states for the global header
  const categoryChildRollupStates = useMemo(() => {
    const states = new Set<CategoryActionState>()
    for (const cat of enabledCategories) {
      const entry = config.category_actions.find(a => a.category === cat.category)
      if (entry && entry.category !== "__global") {
        states.add(entry.action as CategoryActionState)
      }
    }
    return states
  }, [enabledCategories, config.category_actions])

  const saveCategoryActions = async (newActions: CategoryActionEntry[]) => {
    try {
      await invoke("update_category_actions", {
        actions: newActions,
      } satisfies UpdateCategoryActionsParams as Record<string, unknown>)
      setConfig(prev => ({ ...prev, category_actions: newActions }))
    } catch (err) {
      console.error("Failed to update category action:", err)
      toast.error("Failed to update category action")
    }
  }

  const handleGlobalCategoryActionChange = (action: CategoryActionState) => {
    const existingIndex = config.category_actions.findIndex(a => a.category === "__global")
    let newActions: CategoryActionEntry[]
    if (existingIndex >= 0) {
      newActions = config.category_actions.map((a, i) =>
        i === existingIndex ? { ...a, action } : a
      )
    } else {
      newActions = [...config.category_actions, { category: "__global", action }]
    }
    saveCategoryActions(newActions)
  }

  const handleCategoryActionChange = (category: string, action: CategoryActionState) => {
    // If setting to the same as global, remove the explicit override (inherit)
    if (action === globalCategoryAction) {
      const newActions = config.category_actions.filter(a => a.category !== category)
      saveCategoryActions(newActions)
      return
    }

    const existingIndex = config.category_actions.findIndex(a => a.category === category)
    let newActions: CategoryActionEntry[]
    if (existingIndex >= 0) {
      newActions = config.category_actions.map((a, i) =>
        i === existingIndex ? { ...a, action } : a
      )
    } else {
      newActions = [...config.category_actions, { category, action }]
    }
    saveCategoryActions(newActions)
  }

  const handleTestSafetyCheck = async () => {
    if (!testText.trim()) return
    setTesting(true)
    setTestResult(null)
    setTestRan(true)
    try {
      const result = await invoke<SafetyCheckResult>("test_safety_check", {
        text: testText,
      } satisfies TestSafetyCheckParams as Record<string, unknown>)
      setTestResult(result)
    } catch (err) {
      toast.error(`Test failed: ${err}`)
    } finally {
      setTesting(false)
    }
  }

  const handleTestModel = async (modelId: string) => {
    if (!testText.trim()) {
      toast.error("Enter test text first")
      return
    }
    setTestingModel(modelId)
    try {
      const result = await invoke<SafetyCheckResult>("test_safety_model", {
        modelId,
        text: testText,
      } satisfies TestSafetyModelParams as Record<string, unknown>)
      setModelTestResult(prev => ({ ...prev, [modelId]: result }))
    } catch (err) {
      toast.error(`Model test failed: ${err}`)
    } finally {
      setTestingModel(null)
    }
  }

  const handleDownloadModel = async (modelId: string) => {
    setDownloadingModels(prev => new Set(prev).add(modelId))
    try {
      await invoke("download_safety_model", {
        modelId,
      } satisfies DownloadSafetyModelParams as Record<string, unknown>)
    } catch (err) {
      setDownloadingModels(prev => { const next = new Set(prev); next.delete(modelId); return next })
      toast.error(`Failed to start download: ${err}`)
    }
  }

  const handleRemoveModel = async (modelId: string) => {
    try {
      await invoke("remove_safety_model", {
        modelId,
      } satisfies RemoveSafetyModelParams as Record<string, unknown>)
      toast.success("Safety model removed")
      loadConfig()
    } catch (err) {
      toast.error(`Failed to remove model: ${err}`)
    }
  }

  const renderModelStatusBadge = (model: SafetyModelConfig) => {
    const status = modelStatuses[model.id]
    if (!status) return null

    if (status.provider_configured && status.model_available) {
      return (
        <Badge className="text-[10px] bg-emerald-500 text-white">
          <CheckCircle2 className="h-2.5 w-2.5 mr-1" />
          Ready
        </Badge>
      )
    }

    if (status.provider_configured && !status.model_available) {
      return (
        <Badge className="text-[10px] bg-amber-500 text-white">
          <AlertTriangle className="h-2.5 w-2.5 mr-1" />
          Model Unavailable
        </Badge>
      )
    }

    return (
      <Badge variant="secondary" className="text-[10px]">
        Not Configured
      </Badge>
    )
  }

  if (isLoading) {
    return <div className="text-sm text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-4">
      {/* Section 1: Global Controls */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Shield className="h-5 w-5 text-red-500" />
              <CardTitle>GuardRails</CardTitle>
            </div>
            <Switch checked={config.enabled} onCheckedChange={toggleEnabled} />
          </div>
          <CardDescription>
            LLM-based safety scanning for requests and responses. Uses models like Llama Guard,
            ShieldGemma, and others to detect harmful content categories.
          </CardDescription>
        </CardHeader>
      </Card>

      {/* Scan Settings */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Scan Settings</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <Label>Scan Requests</Label>
              <p className="text-xs text-muted-foreground">
                Inspect outgoing prompts before sending to provider
              </p>
            </div>
            <Switch
              checked={config.scan_requests}
              onCheckedChange={toggleScanRequests}
            />
          </div>
          <div className="flex items-center justify-between">
            <div>
              <Label>Scan Responses</Label>
              <p className="text-xs text-muted-foreground">
                Inspect provider responses for harmful content
              </p>
            </div>
            <Switch
              checked={config.scan_responses}
              onCheckedChange={toggleScanResponses}
            />
          </div>
          <div className="flex items-center justify-between">
            <div>
              <Label>Default Confidence Threshold</Label>
              <p className="text-xs text-muted-foreground">
                Minimum confidence score to trigger a safety action (0.0 - 1.0)
              </p>
            </div>
            <div className="flex items-center gap-3">
              <input
                type="range"
                min="0.0"
                max="1.0"
                step="0.05"
                value={config.default_confidence_threshold}
                onChange={(e) => setDefaultThreshold(parseFloat(e.target.value))}
                className="w-28 h-1.5 accent-blue-500"
              />
              <span className="font-mono text-sm text-muted-foreground w-10 text-right">
                {config.default_confidence_threshold.toFixed(2)}
              </span>
            </div>
          </div>
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
                onChange={(e) => setHfToken(e.target.value)}
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
        </CardContent>
      </Card>

      {/* Section 2: Safety Models */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base">Safety Models</CardTitle>
              <CardDescription>
                Configure LLM-based safety models. Each model runs independently and
                produces verdicts for detected content categories.
              </CardDescription>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              onClick={() => setAddModelOpen(true)}
            >
              <Plus className="h-3.5 w-3.5 mr-1" />
              Add Model
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <div className="space-y-3">
            {config.safety_models.map((model) => {
              const execMode = model.execution_mode || "provider"
              const isDownloading = downloadingModels.has(model.id)
              const dlStatus = downloadStatuses[model.id]
              const dlProgress = downloadProgress[model.id]

              return (
                <div
                  key={model.id}
                  className="border rounded-lg p-3 space-y-3"
                >
                  {/* Model header */}
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2 flex-1 min-w-0">
                      <Brain className="h-4 w-4 text-muted-foreground shrink-0" />
                      <span className="font-medium text-sm">{model.label}</span>
                      <Badge variant="outline" className="text-[10px]">
                        {modelTypeLabel(model.model_type)}
                      </Badge>
                      {model.requires_auth && (
                        <Badge variant="secondary" className="text-[10px]">
                          <AlertTriangle className="h-2.5 w-2.5 mr-0.5" />
                          Gated
                        </Badge>
                      )}
                      {model.predefined && (
                        <Badge variant="secondary" className="text-[10px]">
                          Built-in
                        </Badge>
                      )}
                      <Badge variant="outline" className="text-[10px]">
                        {execMode === "local" ? (
                          <><HardDrive className="h-2.5 w-2.5 mr-0.5" />Local</>
                        ) : (
                          <><Cloud className="h-2.5 w-2.5 mr-0.5" />Provider</>
                        )}
                      </Badge>
                      {renderModelStatusBadge(model)}
                    </div>
                    <div className="flex items-center gap-2">
                      {/* Remove button for custom models */}
                      {!model.predefined && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive"
                          onClick={() => handleRemoveModel(model.id)}
                          title="Remove this model"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 px-2"
                        onClick={() => handleTestModel(model.id)}
                        disabled={testingModel === model.id || !testText.trim()}
                        title="Test this model (enter text in Test Panel first)"
                      >
                        {testingModel === model.id ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <FlaskConical className="h-3.5 w-3.5" />
                        )}
                      </Button>
                      <Switch
                        checked={model.enabled}
                        onCheckedChange={(checked) => toggleModel(model.id, checked)}
                      />
                    </div>
                  </div>

                  {/* Model configuration fields */}
                  {model.enabled && (
                    <div className="space-y-3 pl-6">
                      {/* Execution mode selector */}
                      <div className="flex items-center gap-4">
                        <Label className="text-xs text-muted-foreground">Execution:</Label>
                        <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                          <input
                            type="radio"
                            name={`exec-${model.id}`}
                            value="provider"
                            checked={execMode === "provider"}
                            onChange={() => updateModelField(model.id, "execution_mode", "provider")}
                            className="accent-blue-500"
                          />
                          <Cloud className="h-3 w-3" /> Provider
                        </label>
                        <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                          <input
                            type="radio"
                            name={`exec-${model.id}`}
                            value="local"
                            checked={execMode === "local"}
                            onChange={() => updateModelField(model.id, "execution_mode", "local")}
                            className="accent-blue-500"
                          />
                          <HardDrive className="h-3 w-3" /> Local (Built-in)
                        </label>
                      </div>

                      {/* Provider mode fields */}
                      {execMode === "provider" && (
                        <div className="grid grid-cols-2 gap-3">
                          <div>
                            <Label className="text-xs">Provider ID</Label>
                            <Input
                              placeholder="e.g. ollama, together-ai"
                              value={model.provider_id || ""}
                              onChange={(e) =>
                                updateModelField(model.id, "provider_id", e.target.value || null)
                              }
                              className="mt-1 h-8 text-xs"
                            />
                          </div>
                          <div>
                            <Label className="text-xs">Model Name</Label>
                            <Input
                              placeholder="e.g. llama-guard-4"
                              value={model.model_name || ""}
                              onChange={(e) =>
                                updateModelField(model.id, "model_name", e.target.value || null)
                              }
                              className="mt-1 h-8 text-xs"
                            />
                          </div>
                        </div>
                      )}

                      {/* Local mode: download section */}
                      {execMode === "local" && (
                        <div className="space-y-2">
                          {model.hf_repo_id && model.gguf_filename ? (
                            <>
                              <div className="text-[10px] text-muted-foreground font-mono">
                                {model.hf_repo_id} / {model.gguf_filename}
                              </div>
                              {isDownloading ? (
                                <div className="space-y-1">
                                  <div className="flex items-center gap-2 text-xs">
                                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                    <span>Downloading... {dlProgress != null ? `${(dlProgress * 100).toFixed(0)}%` : ""}</span>
                                  </div>
                                  {dlProgress != null && (
                                    <div className="w-full bg-muted rounded-full h-1.5">
                                      <div
                                        className="bg-blue-500 h-1.5 rounded-full transition-all"
                                        style={{ width: `${dlProgress * 100}%` }}
                                      />
                                    </div>
                                  )}
                                </div>
                              ) : dlStatus?.downloaded ? (
                                <div className="flex items-center gap-2 text-xs text-emerald-600">
                                  <CheckCircle2 className="h-3.5 w-3.5" />
                                  <span>Downloaded</span>
                                  {dlStatus.file_size != null && (
                                    <span className="text-muted-foreground">
                                      ({(dlStatus.file_size / 1_048_576).toFixed(0)} MB)
                                    </span>
                                  )}
                                </div>
                              ) : (
                                <Button
                                  variant="outline"
                                  size="sm"
                                  className="h-7"
                                  onClick={() => handleDownloadModel(model.id)}
                                >
                                  <Download className="h-3 w-3 mr-1" />
                                  Download GGUF
                                </Button>
                              )}
                            </>
                          ) : (
                            <p className="text-[10px] text-amber-500 flex items-center gap-1">
                              <AlertTriangle className="h-3 w-3" />
                              No HuggingFace repo or GGUF filename configured for local download.
                            </p>
                          )}
                        </div>
                      )}

                      {/* Confidence threshold override */}
                      <div className="flex items-center gap-3">
                        <Label className="text-xs text-muted-foreground whitespace-nowrap">
                          Confidence threshold:
                        </Label>
                        <input
                          type="range"
                          min="0.05"
                          max="0.99"
                          step="0.05"
                          value={model.confidence_threshold ?? config.default_confidence_threshold}
                          onChange={(e) =>
                            updateModelField(model.id, "confidence_threshold", parseFloat(e.target.value))
                          }
                          className="w-24 h-1.5 accent-blue-500"
                        />
                        <span className="font-mono text-xs text-muted-foreground w-8">
                          {(model.confidence_threshold ?? config.default_confidence_threshold).toFixed(2)}
                        </span>
                        {model.confidence_threshold !== null && (
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-5 px-1 text-[10px] text-muted-foreground"
                            onClick={() => updateModelField(model.id, "confidence_threshold", null)}
                          >
                            Reset
                          </Button>
                        )}
                      </div>

                      {/* Auth note for gated models */}
                      {model.requires_auth && !config.hf_token && (
                        <p className="text-[10px] text-amber-500 flex items-center gap-1">
                          <AlertTriangle className="h-3 w-3" />
                          This model requires a HuggingFace token. Set it in Scan Settings above.
                        </p>
                      )}

                      {/* Per-model test result */}
                      {modelTestResult[model.id] && (
                        <div className="border rounded p-2 space-y-1.5">
                          {modelTestResult[model.id].verdicts.map((verdict, i) => (
                            <VerdictDisplay key={i} verdict={verdict} />
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )
            })}
            {config.safety_models.length === 0 && (
              <p className="text-sm text-muted-foreground text-center py-4">
                No safety models configured.
              </p>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Add Model Dialog */}
      <AddSafetyModelDialog
        open={addModelOpen}
        onOpenChange={setAddModelOpen}
        onModelAdded={loadConfig}
      />

      {/* Section 3: Category Actions */}
      <Card>
        <CardHeader>
          <div>
            <CardTitle className="text-base">Category Actions</CardTitle>
            <CardDescription>
              Configure the action to take when content is flagged. Set a global default,
              then override individual categories as needed. Only categories from enabled
              models are shown.
            </CardDescription>
          </div>
        </CardHeader>
        <CardContent>
          {enabledCategories.length > 0 ? (
            <div className="border rounded-lg">
              <div className="max-h-[500px] overflow-y-auto">
                {/* Global default row - sticky header */}
                <div className="flex items-center gap-2 px-3 py-3 border-b bg-background sticky top-0 z-10">
                  <span className="font-semibold text-sm flex-1">All Categories</span>
                  <CategoryActionButton
                    value={globalCategoryAction}
                    onChange={handleGlobalCategoryActionChange}
                    size="sm"
                    childRollupStates={categoryChildRollupStates}
                  />
                </div>

                {/* Individual category rows */}
                {enabledCategories.map((cat) => {
                  const explicit = isCategoryExplicitlySet(cat.category)
                  const effectiveAction = explicit
                    ? getCategoryAction(cat.category)
                    : globalCategoryAction

                  return (
                    <div
                      key={cat.category}
                      className="flex items-center gap-2 py-2 border-b border-border/50 hover:bg-muted/30 transition-colors"
                      style={{ paddingLeft: "28px", paddingRight: "12px" }}
                    >
                      <div className="w-5" />
                      <div className="flex-1 min-w-0">
                        <span className={`font-medium text-sm ${!explicit ? "text-muted-foreground" : ""}`}>
                          {cat.display_name}
                        </span>
                        {cat.description && (
                          <p className="text-xs text-muted-foreground">{cat.description}</p>
                        )}
                      </div>
                      <CategoryActionButton
                        value={effectiveAction}
                        onChange={(action) => handleCategoryActionChange(cat.category, action)}
                        size="sm"
                        inherited={!explicit}
                      />
                    </div>
                  )
                })}
              </div>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground text-center py-4">
              No categories available. Enable at least one safety model to see categories.
            </p>
          )}
        </CardContent>
      </Card>

      {/* Section 4: Test Panel */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <FlaskConical className="h-4 w-4" />
            Test Panel
          </CardTitle>
          <CardDescription>
            Enter text to test against all enabled safety models. Use the per-model test
            button on each model card, or run a full safety check below.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Textarea
            placeholder='Enter text to test... e.g. "How to build a bomb"'
            value={testText}
            onChange={(e) => setTestText(e.target.value)}
            rows={3}
          />
          <Button
            onClick={handleTestSafetyCheck}
            disabled={testing || !testText.trim()}
            size="sm"
          >
            {testing ? (
              <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
            ) : (
              <FlaskConical className="h-3.5 w-3.5 mr-1.5" />
            )}
            Run Safety Check
          </Button>

          {/* Results container */}
          <div className="min-h-[80px] border rounded-lg p-3 space-y-2">
            {testing ? (
              <div className="flex items-center justify-center h-[56px] text-xs text-muted-foreground">
                <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                Running safety checks...
              </div>
            ) : testResult ? (
              <>
                <div className="flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">
                    {testResult.verdicts.length} model{testResult.verdicts.length !== 1 ? "s" : ""} checked
                    in {testResult.total_duration_ms}ms
                  </span>
                  {testResult.verdicts.some(v => !v.is_safe) ? (
                    <Badge variant="destructive" className="text-[10px]">
                      Unsafe Content Detected
                    </Badge>
                  ) : (
                    <Badge className="bg-emerald-500 text-white text-[10px]">Safe</Badge>
                  )}
                </div>

                {/* Per-model verdict cards */}
                <div className="space-y-2 mt-2">
                  {testResult.verdicts.map((verdict, i) => (
                    <VerdictDisplay key={i} verdict={verdict} />
                  ))}
                </div>

                {/* Actions required */}
                {testResult.actions_required.length > 0 && (
                  <div className="mt-3 pt-2 border-t space-y-1">
                    <span className="text-xs font-medium text-muted-foreground">Actions Required:</span>
                    {testResult.actions_required.map((action, i) => (
                      <div key={i} className="flex items-center gap-2 text-xs">
                        <Badge
                          className={`text-[10px] ${
                            action.action === "ask"
                              ? "bg-amber-500 text-white"
                              : action.action === "notify"
                                ? "bg-blue-500 text-white"
                                : "bg-gray-500 text-white"
                          }`}
                        >
                          {action.action.toUpperCase()}
                        </Badge>
                        <span className="font-medium">{action.category.replace(/_/g, " ")}</span>
                        <span className="text-muted-foreground">
                          from {action.model_id}
                          {action.confidence !== null && ` (${(action.confidence * 100).toFixed(0)}%)`}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </>
            ) : (
              <div className="flex items-center justify-center h-[56px] text-xs text-muted-foreground">
                {testRan ? "No results" : "Run a test to see results"}
              </div>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

/** Displays a single model verdict */
function VerdictDisplay({ verdict }: { verdict: SafetyVerdict }) {
  return (
    <div className="bg-muted/50 rounded px-3 py-2 text-xs space-y-1.5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          {verdict.is_safe ? (
            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
          ) : (
            <XCircle className="h-3.5 w-3.5 text-red-500" />
          )}
          <span className="font-medium">{verdict.model_id}</span>
          {verdict.confidence !== null && (
            <span className="text-muted-foreground font-mono">
              {(verdict.confidence * 100).toFixed(0)}%
            </span>
          )}
        </div>
        <span className="text-muted-foreground text-[10px]">
          {verdict.check_duration_ms}ms
        </span>
      </div>

      {/* Flagged categories */}
      {verdict.flagged_categories.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mt-1">
          {verdict.flagged_categories.map((cat, j) => (
            <Badge key={j} variant="destructive" className="text-[10px]">
              {cat.category.replace(/_/g, " ")}
              {cat.confidence !== null && ` ${(cat.confidence * 100).toFixed(0)}%`}
            </Badge>
          ))}
        </div>
      )}

      {/* Raw output (collapsed by default for non-empty) */}
      {verdict.raw_output && (
        <details className="mt-1">
          <summary className="text-[10px] text-muted-foreground cursor-pointer hover:text-foreground">
            Raw output
          </summary>
          <pre className="mt-1 text-[10px] font-mono bg-muted p-2 rounded overflow-x-auto whitespace-pre-wrap">
            {verdict.raw_output}
          </pre>
        </details>
      )}
    </div>
  )
}
