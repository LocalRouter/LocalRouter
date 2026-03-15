import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import {
  Shield,
  Globe,
  ChevronDown,
  ChevronUp,
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
  PullProviderModelParams,
  ProviderModelPullProgress,
} from "@/types/tauri-commands"

interface GuardrailsTabProps {
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function GuardrailsTab({ onTabChange }: GuardrailsTabProps) {
  const [config, setConfig] = useState<GuardrailsConfig>({
    scan_requests: true,
    safety_models: [],
    category_actions: [],
    default_confidence_threshold: 0.5,
    parallel_guardrails: true,
    moderation_api_enabled: false,
  })
  const [showExtraCategories, setShowExtraCategories] = useState(false)
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

      if (selection.needsPull) {
        // Trigger pull in background — progress is tracked via events
        toast.info(`Pulling "${selection.modelName}" from ${selection.providerId}...`)
        invoke("pull_provider_model", {
          providerId: selection.providerId,
          modelName: selection.modelName,
        } satisfies PullProviderModelParams as Record<string, unknown>).catch((err) => {
          toast.error(`Failed to start pull: ${err}`)
        })
      } else {
        toast.success("Provider model added")
      }
    } catch (err) {
      toast.error(`Failed to add model: ${err}`)
    }
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
    return <div className="text-sm text-muted-foreground">Loading...</div>
  }

  return (
    <div className="space-y-4 max-w-2xl">
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
                onClick={() => onTabChange("guardrails", "try-it-out")}
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
                Run safety checks alongside the LLM request for lower latency. Automatically falls back to sequential scanning for requests with side effects (e.g. Perplexity Sonar, non-function tools). For MCP via LLM, guardrails run in parallel but must complete before any tool execution.
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

      {/* Card: Moderation API Endpoint */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Globe className="h-4 w-4 text-blue-500" />
            <CardTitle className="text-base">Moderation API Endpoint</CardTitle>
          </div>
          <CardDescription>
            Expose your configured safety models via the standard <code className="text-xs bg-muted px-1 py-0.5 rounded">/v1/moderations</code> endpoint.
            Clients can classify content using the OpenAI-compatible moderation API format. Auth required.
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
            <Switch
              checked={config.moderation_api_enabled}
              onCheckedChange={(checked) => saveConfig({ ...config, moderation_api_enabled: checked })}
            />
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
  )
}
