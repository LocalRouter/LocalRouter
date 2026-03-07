import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Shield } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Switch } from "@/components/ui/switch"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/Badge"
import { type PickerSelection } from "@/components/guardrails/SafetyModelPicker"
import { GuardrailsTab as GuardrailsTryItOut } from "@/views/try-it-out/guardrails-tab"
import { GuardrailsPanel } from "./guardrails-panel"
import type {
  GuardrailsConfig,
  SafetyModelConfig,
  UpdateGuardrailsConfigParams,
  AddSafetyModelParams,
  RemoveSafetyModelParams,
  PullProviderModelParams,
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
  if (!subTab) return { tab: "models" }
  const parts = subTab.split("/")
  const tab = parts[0] || "models"
  if (parts[1] === "init" && parts[2] === "client" && parts[3]) {
    return { tab, initClientId: parts[3] }
  }
  return { tab }
}

export function GuardrailsView({ activeSubTab, onTabChange }: GuardrailsViewProps) {
  const [config, setConfig] = useState<GuardrailsConfig>({
    scan_requests: true,
    safety_models: [],
    default_confidence_threshold: 0.5,
    parallel_guardrails: true,
  })
  const [isLoading, setIsLoading] = useState(true)
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({})

  const { tab, initClientId } = parseInitPath(activeSubTab)

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

  // Listen for load error events
  useEffect(() => {
    const unlisteners: (() => void)[] = []

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

  const handleTabChange = (newTab: string) => {
    onTabChange("guardrails", newTab)
  }

  if (isLoading) {
    return (
      <div className="flex flex-col h-full min-h-0">
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Shield className="h-6 w-6" />
            GuardRails
          </h1>
          <p className="text-sm text-muted-foreground">Loading...</p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Shield className="h-6 w-6" />
            GuardRails
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
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
          <TabsTrigger value="models">Models</TabsTrigger>
          <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="models" className="flex-1 min-h-0 mt-4">
          <GuardrailsPanel
            models={config.safety_models}
            loadErrors={loadErrors}
            existingModelIds={config.safety_models.map(m => m.id)}
            onPickerSelect={handlePickerSelect}
            onRemoveModel={handleRemoveModel}
          />
        </TabsContent>

        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <GuardrailsTryItOut forcedMode="all_models" hideModeSwitcher initialClientId={initClientId} />
        </TabsContent>

        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Settings</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
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
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
