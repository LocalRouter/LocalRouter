import { useState, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Globe, ChevronDown, ChevronUp, ExternalLink } from "lucide-react"
import { cn } from "@/lib/utils"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Badge } from "@/components/ui/Badge"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Switch } from "@/components/ui/switch"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import { GuardrailsTab as GuardrailsTryItOut } from "@/views/try-it-out/guardrails-tab"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { CategoryActionButton, type CategoryActionState } from "@/components/permissions/CategoryActionButton"
import { PermissionTreeSelector } from "@/components/permissions/PermissionTreeSelector"
import type { TreeNode } from "@/components/permissions/types"
import { GuardrailsPanel } from "./guardrails-panel"
import type {
  GuardrailsConfig,
  SafetyModelConfig,
  SafetyCategoryInfo,
  UpdateGuardrailsConfigParams,
  AddSafetyModelParams,
  RemoveSafetyModelParams,
  ToggleSafetyModelParams,
  PullProviderModelParams,
  ClientFeatureStatus,
  GetFeatureClientsStatusParams,
} from "@/types/tauri-commands"

interface GuardrailsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

// Parse init path: "try-it-out/init/<mode>/<target>" -> { initMode, initTarget }
function parseInitPath(subTab: string | null): {
  tab: string
  initClientId?: string
} {
  if (!subTab) return { tab: "info" }
  const parts = subTab.split("/")
  const tab = parts[0] || "info"
  if (parts[1] === "init" && parts[2] === "client" && parts[3]) {
    return { tab, initClientId: parts[3] }
  }
  return { tab }
}

export function GuardrailsView({ activeSubTab, onTabChange }: GuardrailsViewProps) {
  const [config, setConfig] = useState<GuardrailsConfig>({
    scan_requests: true,
    safety_models: [],
    category_actions: [],
    default_confidence_threshold: 0.5,
    parallel_guardrails: true,
    moderation_api_enabled: true,
  })
  const [categories, setCategories] = useState<SafetyCategoryInfo[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({})
  const [pullProgress, setPullProgress] = useState<Record<string, { progress: number; status: string }>>({})
  const [showExtraCategories, setShowExtraCategories] = useState(false)
  const [clientStatuses, setClientStatuses] = useState<ClientFeatureStatus[]>([])

  const { tab, initClientId } = parseInitPath(activeSubTab)

  const loadConfig = useCallback(async () => {
    try {
      const [result, cats] = await Promise.all([
        invoke<GuardrailsConfig>("get_guardrails_config"),
        invoke<SafetyCategoryInfo[]>("get_all_safety_categories"),
      ])
      setConfig(result)
      setCategories(cats)
    } catch (err) {
      console.error("Failed to load guardrails config:", err)
      toast.error("Failed to load guardrails configuration")
    } finally {
      setIsLoading(false)
    }
  }, [])

  const loadClientStatuses = useCallback(async () => {
    try {
      const data = await invoke<ClientFeatureStatus[]>("get_feature_clients_status", {
        feature: "guardrails",
      } satisfies GetFeatureClientsStatusParams)
      setClientStatuses(data)
    } catch (err) {
      console.error("Failed to load client statuses:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadClientStatuses()
  }, [loadConfig, loadClientStatuses])

  useEffect(() => {
    const listeners = [
      listenSafe("clients-changed", loadClientStatuses),
      listenSafe("config-changed", loadClientStatuses),
    ]
    return () => { listeners.forEach(l => l.cleanup()) }
  }, [loadClientStatuses])

  // Listen for load error events
  useEffect(() => {
    const l = listenSafe<{ model_id: string; error: string }>("safety-model-load-failed", (event) => {
      const { model_id, error } = event.payload
      setLoadErrors(prev => ({ ...prev, [model_id]: error }))
      toast.error(`Safety model "${model_id}" failed to load: ${error}`)
    })

    return () => { l.cleanup() }
  }, [])

  const rebuildEngine = useCallback(async () => {
    try {
      await invoke("rebuild_safety_engine")
    } catch (err) {
      console.error("Failed to rebuild safety engine:", err)
    }
  }, [])

  // Listen for model pull progress/completion/failure events
  useEffect(() => {
    const onProgress = listenSafe<{ provider_id: string; model_name: string; status: string; total: number | null; completed: number | null }>(
      "provider-model-pull-progress",
      (event) => {
        const { provider_id, model_name, status, total, completed } = event.payload
        const key = `${provider_id}:${model_name}`
        const progress = total && total > 0 && completed != null ? (completed / total) * 100 : -1
        setPullProgress(prev => ({ ...prev, [key]: { progress, status } }))
      },
    )
    const onComplete = listenSafe<{ provider_id: string; model_name: string }>(
      "provider-model-pull-complete",
      async (event) => {
        const { provider_id, model_name } = event.payload
        const key = `${provider_id}:${model_name}`
        setPullProgress(prev => {
          const next = { ...prev }
          delete next[key]
          return next
        })
        toast.success(`Pulled "${model_name}" successfully`)

        // Enable the model that was waiting for the pull
        try {
          const freshConfig = await invoke<GuardrailsConfig>("get_guardrails_config")
          const model = freshConfig.safety_models.find(
            m => m.provider_id === provider_id && m.model_name === model_name && !m.enabled
          )
          if (model) {
            await invoke("toggle_safety_model", {
              modelId: model.id,
              enabled: true,
            } satisfies ToggleSafetyModelParams as Record<string, unknown>)
          }
        } catch (err) {
          console.error("Failed to auto-enable model after pull:", err)
        }

        await loadConfig()
        rebuildEngine()
      },
    )
    const onFailed = listenSafe<{ provider_id: string; model_name: string; error: string }>(
      "provider-model-pull-failed",
      (event) => {
        const { provider_id, model_name, error } = event.payload
        const key = `${provider_id}:${model_name}`
        setPullProgress(prev => {
          const next = { ...prev }
          delete next[key]
          return next
        })
        setLoadErrors(prev => ({ ...prev, [key]: error }))
        toast.error(`Failed to pull "${model_name}": ${error}`)
      },
    )

    return () => {
      onProgress.cleanup()
      onComplete.cleanup()
      onFailed.cleanup()
    }
  }, [rebuildEngine, loadConfig])

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

  const handleToggleModel = async (modelId: string, enabled: boolean) => {
    try {
      await invoke("toggle_safety_model", {
        modelId,
        enabled,
      } satisfies ToggleSafetyModelParams as Record<string, unknown>)
      toast.success(enabled ? "Safety model enabled" : "Safety model disabled")
      await loadConfig()
      rebuildEngine()
    } catch (err) {
      toast.error(`Failed to toggle model: ${err}`)
    }
  }

  const handlePickerSelect = async (selection: PickerSelection) => {
    const modelConfig: SafetyModelConfig = {
      id: "",
      label: selection.label,
      model_type: selection.modelType,
      enabled: !selection.needsPull,  // Start disabled if it needs pulling
      provider_id: selection.providerId,
      model_name: selection.modelName,
      confidence_threshold: null,
      safe_indicator: null,
      output_regex: null,
      category_mapping: null,
      enabled_categories: null,
      prompt_template: null,
    }

    try {
      await invoke<string>("add_safety_model", {
        configJson: JSON.stringify(modelConfig),
      } satisfies AddSafetyModelParams as Record<string, unknown>)
      await loadConfig()
      rebuildEngine()

      if (selection.needsPull) {
        const key = `${selection.providerId}:${selection.modelName}`
        setPullProgress(prev => ({ ...prev, [key]: { progress: 0, status: "starting" } }))
        toast.info(`Pulling "${selection.modelName}" from ${selection.providerId}...`)
        invoke("pull_provider_model", {
          providerId: selection.providerId,
          modelName: selection.modelName,
        } satisfies PullProviderModelParams as Record<string, unknown>).catch((err) => {
          setPullProgress(prev => {
            const next = { ...prev }
            delete next[key]
            return next
          })
          toast.error(`Failed to start pull: ${err}`)
        })
      } else {
        toast.success("Provider model added")
      }
    } catch (err) {
      toast.error(`Failed to add model: ${err}`)
    }
  }

  const handleTabChange = (newTab: string) => {
    onTabChange("guardrails", newTab)
  }

  // Build category tree nodes (grouped by model type), filtered to only enabled models
  const categoryTreeNodes = useMemo((): TreeNode[] => {
    if (categories.length === 0) return []

    // Only show categories for model types that have at least one enabled safety model
    const enabledModelTypes = new Set(
      config.safety_models.filter(m => m.enabled).map(m => m.model_type)
    )

    const modelTypeGroups: Record<string, TreeNode[]> = {}

    for (const cat of categories) {
      for (const modelType of cat.supported_by) {
        if (!enabledModelTypes.has(modelType)) continue
        if (!modelTypeGroups[modelType]) {
          modelTypeGroups[modelType] = []
        }
        modelTypeGroups[modelType].push({
          id: cat.category,
          label: cat.display_name,
          description: cat.description,
        })
      }
    }

    const MODEL_TYPE_LABELS: Record<string, string> = {
      llama_guard: "Llama Guard",
      shield_gemma: "ShieldGemma",
      nemotron: "Nemotron",
      granite_guardian: "Granite Guardian",
      openai_moderation: "OpenAI Moderation",
    }

    return Object.entries(modelTypeGroups).map(([modelType, children]) => ({
      id: `__model:${modelType}`,
      label: MODEL_TYPE_LABELS[modelType] || modelType,
      children,
    }))
  }, [categories, config.safety_models])

  // Build permissions map from category_actions
  const categoryPermissionsMap = useMemo((): Record<string, CategoryActionState> => {
    const map: Record<string, CategoryActionState> = {}
    for (const entry of config.category_actions) {
      if (entry.category !== "__global" && entry.action !== "allow") {
        map[entry.category] = entry.action as CategoryActionState
      }
    }
    return map
  }, [config.category_actions])

  const globalCategoryAction = useMemo((): CategoryActionState => {
    const global = config.category_actions.find(e => e.category === "__global")
    return (global?.action as CategoryActionState) || "allow"
  }, [config.category_actions])

  const handleCategoryActionChange = (id: string, action: CategoryActionState) => {
    const actions = config.category_actions.filter(a => a.category !== id)
    actions.push({ category: id, action })
    saveConfig({ ...config, category_actions: actions })
  }

  const handleGlobalCategoryActionChange = (action: CategoryActionState) => {
    const actions = config.category_actions.filter(a => a.category !== "__global")
    actions.push({ category: "__global", action })
    saveConfig({ ...config, category_actions: actions })
  }

  const extraCategories: [string, string][] = [
    ["Non-Violent Crimes", "non_violent_crimes"],
    ["Sex Crimes", "sex_crimes"],
    ["Defamation", "defamation"],
    ["Specialized Advice", "specialized_advice"],
    ["Privacy", "privacy"],
    ["Intellectual Property", "intellectual_property"],
    ["Indiscriminate Weapons", "indiscriminate_weapons"],
    ["Elections", "elections"],
    ["Code Interpreter Abuse", "code_interpreter_abuse"],
    ["Dangerous Content", "dangerous_content"],
    ["Guns & Illegal Weapons", "guns_illegal_weapons"],
    ["Controlled Substances", "controlled_substances"],
    ["Profanity", "profanity"],
    ["Needs Caution", "needs_caution"],
    ["Manipulation", "manipulation"],
    ["Fraud & Deception", "fraud_deception"],
    ["Malware", "malware"],
    ["High Risk Gov Decision", "high_risk_gov_decision"],
    ["Political Misinformation", "political_misinformation"],
    ["Copyright & Plagiarism", "copyright_plagiarism"],
    ["Unauthorized Advice", "unauthorized_advice"],
    ["Immoral & Unethical", "immoral_unethical"],
    ["Social Bias", "social_bias"],
    ["Jailbreak", "jailbreak"],
    ["Unethical Behavior", "unethical_behavior"],
  ]

  if (isLoading) {
    return (
      <div className="flex flex-col h-full min-h-0 max-w-5xl">
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.guardrails.icon className={`h-6 w-6 ${FEATURES.guardrails.color}`} />
            GuardRails
          </h1>
          <p className="text-sm text-muted-foreground">Loading...</p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <FEATURES.guardrails.icon className={`h-6 w-6 ${FEATURES.guardrails.color}`} />
            GuardRails
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          Configure safety models and test content moderation
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="models"><TAB_ICONS.models className={TAB_ICON_CLASS} />Models</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="info" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Default GuardRails */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">GuardRails (Default)</CardTitle>
                <CardDescription>
                  Default actions for each safety category. These apply to all clients unless overridden per-client.
                  GuardRails are active when any category has a non-Allow action.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <PermissionTreeSelector<CategoryActionState>
                  nodes={categoryTreeNodes}
                  permissions={categoryPermissionsMap}
                  globalPermission={globalCategoryAction}
                  onPermissionChange={handleCategoryActionChange}
                  onGlobalChange={handleGlobalCategoryActionChange}
                  renderButton={(props) => <CategoryActionButton {...props} />}
                  globalLabel="All Categories"
                  emptyMessage="No categories available. Add safety models in the Models tab first."
                />
              </CardContent>
              {clientStatuses.length > 0 && (
                <CardContent className="pt-0">
                  <div className="border-t pt-3 space-y-1.5">
                    {clientStatuses.map((s) => (
                      <div
                        key={s.client_id}
                        className="flex items-center justify-between py-1 px-2 rounded-md hover:bg-muted/50 group"
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          {onTabChange ? (
                            <button
                              onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                              className="text-sm font-medium truncate hover:underline text-left"
                            >
                              {s.client_name}
                            </button>
                          ) : (
                            <span className="text-sm font-medium truncate">{s.client_name}</span>
                          )}
                          {s.source === "override" && (
                            <Badge variant="outline" className="text-[10px] px-1 py-0 shrink-0">
                              Override
                            </Badge>
                          )}
                        </div>
                        <div className="flex items-center gap-2 shrink-0">
                          <Badge
                            variant="outline"
                            className={cn(
                              "text-[10px] px-1.5 py-0",
                              s.active
                                ? "border-emerald-500/50 text-emerald-600"
                                : "border-red-500/50 text-red-600",
                            )}
                          >
                            {s.effective_value || (s.active ? "Active" : "Inactive")}
                          </Badge>
                          {onTabChange && (
                            <button
                              onClick={() => onTabChange("clients", `${s.client_id}|optimize`)}
                              className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-foreground transition-opacity"
                              title="Go to client settings"
                            >
                              <ExternalLink className="h-3.5 w-3.5" />
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </CardContent>
              )}
            </Card>
          </div>
        </TabsContent>

        <TabsContent value="models" className="flex-1 min-h-0 mt-4">
          <GuardrailsPanel
            models={config.safety_models}
            loadErrors={loadErrors}
            pullProgress={pullProgress}
            onPickerSelect={handlePickerSelect}
            onRemoveModel={handleRemoveModel}
            onToggleModel={handleToggleModel}
          />
        </TabsContent>

        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="flex items-center justify-between mb-4 pb-4 border-b">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when a guardrail flags content with an &ldquo;Ask&rdquo; action
              </p>
            </div>
            <SamplePopupButton popupType="guardrail" />
          </div>
          <GuardrailsTryItOut forcedMode="all_models" hideModeSwitcher initialClientId={initClientId} />
        </TabsContent>

        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Settings</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <Label>Parallel Scanning</Label>
                    <p className="text-xs text-muted-foreground">
                      Run safety checks alongside the LLM request for lower latency. Automatically falls back to sequential scanning for requests with side effects (e.g. Perplexity Sonar, non-function tools). For MCP via LLM, guardrails run in parallel but must complete before any tool execution.
                    </p>
                  </div>
                  <InfoTooltip content="Runs safety checks in parallel with the LLM request instead of before it. Reduces latency but may allow unsafe content through briefly before being caught.">
                    <Switch
                      checked={config.parallel_guardrails}
                      onCheckedChange={(checked) => saveConfig({ ...config, parallel_guardrails: checked })}
                    />
                  </InfoTooltip>
                </div>
              </CardContent>
            </Card>

            {/* Moderation API Endpoint */}
            <Card>
              <CardHeader>
                <div className="flex items-center gap-2">
                  <Globe className="h-4 w-4 text-blue-500" />
                  <CardTitle className="text-base">Moderation API Endpoint</CardTitle>
                </div>
                <CardDescription>
                  Expose your configured safety models via the OpenAI-compatible endpoint <code className="text-xs bg-muted px-1 py-0.5 rounded">/moderations</code>.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <Label>Enabled</Label>
                    <p className="text-xs text-muted-foreground">
                      {config.moderation_api_enabled
                        ? "Clients can call POST /v1/moderations with their API key."
                        : "Endpoint returns 503 Service Unavailable."}
                    </p>
                  </div>
                  <InfoTooltip content="Exposes safety models via a /v1/moderations endpoint, allowing external clients to run content moderation checks directly.">
                    <Switch
                      checked={config.moderation_api_enabled}
                      onCheckedChange={(checked) => saveConfig({ ...config, moderation_api_enabled: checked })}
                    />
                  </InfoTooltip>
                </div>

                {/* Category Mapping Table */}
                <div>
                  <Label className="text-xs mb-2 block">Category Mapping</Label>
                  <div className="border rounded-md text-xs">
                    <table className="w-full">
                      <thead>
                        <tr className="border-b bg-muted/50">
                          <th className="text-left px-3 py-1.5 font-medium">Safety Category</th>
                          <th className="text-left px-3 py-1.5 font-medium">OpenAI Category</th>
                        </tr>
                      </thead>
                      <tbody>
                        {[
                          ["Hate", "hate, hate/threatening"],
                          ["Harassment", "harassment, harassment/threatening"],
                          ["Self-Harm", "self-harm, self-harm/intent, self-harm/instructions"],
                          ["Sexual Content", "sexual"],
                          ["Child Exploitation", "sexual/minors"],
                          ["Violent Crimes", "violence, violence/graphic"],
                          ["Illegal Activity", "illicit"],
                          ["Criminal Planning", "illicit/violent"],
                        ].map(([safety, openai]) => (
                          <tr key={safety} className="border-b last:border-b-0">
                            <td className="px-3 py-1.5">{safety}</td>
                            <td className="px-3 py-1.5 text-muted-foreground">{openai}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>

                  {/* Extra categories collapsible */}
                  <button
                    className="flex items-center gap-1 text-xs text-muted-foreground mt-2 hover:text-foreground transition-colors"
                    onClick={() => setShowExtraCategories(!showExtraCategories)}
                  >
                    {showExtraCategories ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                    {showExtraCategories ? "Hide" : "Show"} extra categories ({extraCategories.length})
                  </button>
                  {showExtraCategories && (
                    <div className="border rounded-md text-xs mt-1.5">
                      <table className="w-full">
                        <thead>
                          <tr className="border-b bg-muted/50">
                            <th className="text-left px-3 py-1.5 font-medium">Safety Category</th>
                            <th className="text-left px-3 py-1.5 font-medium">Response Key</th>
                          </tr>
                        </thead>
                        <tbody>
                          {extraCategories.map(([display, key]) => (
                            <tr key={key} className="border-b last:border-b-0">
                              <td className="px-3 py-1.5">{display}</td>
                              <td className="px-3 py-1.5 font-mono text-muted-foreground">{key}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                      <div className="px-3 py-2 text-muted-foreground bg-muted/30 border-t">
                        Extra categories are detected by your safety models but not part of the official OpenAI spec.
                        They are returned alongside standard categories in the response.
                      </div>
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
