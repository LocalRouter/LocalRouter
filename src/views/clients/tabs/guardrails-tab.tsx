import { useState, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Shield, Info } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Badge } from "@/components/ui/Badge"
import { CategoryActionButton, type CategoryActionState } from "@/components/permissions/CategoryActionButton"
import { PermissionTreeSelector } from "@/components/permissions/PermissionTreeSelector"
import type { TreeNode } from "@/components/permissions/types"
import { SafetyModelList } from "@/components/guardrails/SafetyModelList"
import { ResourceRequirements } from "@/components/guardrails/ResourceRequirements"
import type {
  ClientGuardrailsConfig,
  GuardrailsConfig,
  SafetyModelConfig,
  SafetyCategoryInfo,
  SafetyModelDownloadStatus,
  GetClientGuardrailsConfigParams,
  UpdateClientGuardrailsConfigParams,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
}

interface ClientGuardrailsTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientGuardrailsTab({ client, onUpdate, onViewChange }: ClientGuardrailsTabProps) {
  const [guardrailsConfig, setGuardrailsConfig] = useState<ClientGuardrailsConfig>({
    enabled: false,
    category_actions: [],
  })
  const [globalConfig, setGlobalConfig] = useState<GuardrailsConfig | null>(null)
  const [categories, setCategories] = useState<SafetyCategoryInfo[]>([])
  const [downloadStatuses, setDownloadStatuses] = useState<Record<string, SafetyModelDownloadStatus>>({})
  const [loading, setLoading] = useState(true)

  const loadConfig = useCallback(async () => {
    try {
      const [clientConfig, global, cats] = await Promise.all([
        invoke<ClientGuardrailsConfig>("get_client_guardrails_config", {
          clientId: client.id,
        } satisfies GetClientGuardrailsConfigParams as Record<string, unknown>),
        invoke<GuardrailsConfig>("get_guardrails_config"),
        invoke<SafetyCategoryInfo[]>("get_all_safety_categories"),
      ])
      setGuardrailsConfig(clientConfig)
      setGlobalConfig(global)
      setCategories(cats)

      // Load download statuses for models
      const statuses: Record<string, SafetyModelDownloadStatus> = {}
      for (const model of global.safety_models) {
        if (model.gguf_filename) {
          try {
            const status = await invoke<SafetyModelDownloadStatus>(
              "get_safety_model_download_status",
              { modelId: model.id } as Record<string, unknown>
            )
            statuses[model.id] = status
          } catch {
            // ignore
          }
        }
      }
      setDownloadStatuses(statuses)
    } catch (err) {
      console.error("Failed to load guardrails config:", err)
      toast.error("Failed to load guardrails configuration")
    } finally {
      setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  const saveConfig = async (newConfig: ClientGuardrailsConfig) => {
    setGuardrailsConfig(newConfig)
    try {
      await invoke("update_client_guardrails_config", {
        clientId: client.id,
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateClientGuardrailsConfigParams as Record<string, unknown>)
      onUpdate()
    } catch (err) {
      toast.error("Failed to save guardrails configuration")
      loadConfig()
    }
  }

  const toggleEnabled = (enabled: boolean) => {
    saveConfig({ ...guardrailsConfig, enabled })
  }

  // Build category tree nodes (grouped by model type)
  const categoryTreeNodes = useMemo((): TreeNode[] => {
    if (!globalConfig || categories.length === 0) return []

    // Group categories by the model types that support them
    const modelTypeGroups: Record<string, TreeNode[]> = {}

    for (const cat of categories) {
      for (const modelType of cat.supported_by) {
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
    }

    return Object.entries(modelTypeGroups).map(([modelType, children]) => ({
      id: `__model:${modelType}`,
      label: MODEL_TYPE_LABELS[modelType] || modelType,
      children,
    }))
  }, [globalConfig, categories])

  // Build permissions map from category_actions
  const categoryPermissionsMap = useMemo((): Record<string, CategoryActionState> => {
    const map: Record<string, CategoryActionState> = {}
    for (const entry of guardrailsConfig.category_actions) {
      if (entry.category !== "__global" && entry.action !== "ask") {
        map[entry.category] = entry.action as CategoryActionState
      }
    }
    return map
  }, [guardrailsConfig.category_actions])

  const globalCategoryAction = useMemo((): CategoryActionState => {
    const global = guardrailsConfig.category_actions.find(e => e.category === "__global")
    return (global?.action as CategoryActionState) || "ask"
  }, [guardrailsConfig.category_actions])

  const handleCategoryActionChange = (id: string, action: CategoryActionState) => {
    const actions = guardrailsConfig.category_actions.filter(a => a.category !== id)
    actions.push({ category: id, action })
    saveConfig({ ...guardrailsConfig, category_actions: actions })
  }

  const handleGlobalCategoryActionChange = (action: CategoryActionState) => {
    const actions = guardrailsConfig.category_actions.filter(a => a.category !== "__global")
    actions.push({ category: "__global", action })
    saveConfig({ ...guardrailsConfig, category_actions: actions })
  }

  // Determine which models will run based on selected categories
  const activeModels = useMemo((): SafetyModelConfig[] => {
    if (!globalConfig) return []

    // Find which model types have categories configured (not all set to "allow")
    const activeModelTypes = new Set<string>()
    for (const node of categoryTreeNodes) {
      const modelType = node.id.replace("__model:", "")
      // A model type is "active" if any of its children have a non-allow action
      const hasNonAllowChild = node.children?.some(child => {
        const action = categoryPermissionsMap[child.id] || globalCategoryAction
        return action !== "allow"
      })
      if (hasNonAllowChild) {
        activeModelTypes.add(modelType)
      }
    }

    return globalConfig.safety_models.filter(m => activeModelTypes.has(m.model_type))
  }, [globalConfig, categoryTreeNodes, categoryPermissionsMap, globalCategoryAction])

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {/* Header Card */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Shield className="h-5 w-5 text-red-500" />
              <CardTitle>GuardRails</CardTitle>
              <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
            </div>
            <Switch checked={guardrailsConfig.enabled} onCheckedChange={toggleEnabled} />
          </div>
          <CardDescription>
            Safety scanning for this client's requests. Flagged content is handled based on
            your category action settings below.
            {onViewChange && (
              <button
                className="text-blue-500 hover:underline ml-1"
                onClick={() => onViewChange("try-it-out", `guardrails/init/client/${client.client_id}`)}
              >
                Test in Try It Out
              </button>
            )}
          </CardDescription>
        </CardHeader>
      </Card>

      {guardrailsConfig.enabled && (
        <>
          {/* Available Models */}
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Available Models</CardTitle>
              <CardDescription className="flex items-start gap-1.5">
                <Info className="h-3.5 w-3.5 mt-0.5 shrink-0" />
                Models are managed in Settings. Categories you select below determine which models run.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <SafetyModelList
                models={globalConfig?.safety_models || []}
                downloadStatuses={downloadStatuses}
                downloadProgress={{}}
                readOnly
              />
            </CardContent>
          </Card>

          {/* Category Actions */}
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Category Actions</CardTitle>
              <CardDescription>
                Configure the action for each safety category. Selecting categories for a model type
                means that model will run for this client.
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
                emptyMessage="No categories available. Download safety models in Settings first."
                defaultExpanded
              />
            </CardContent>
          </Card>

          {/* Resource Requirements */}
          {activeModels.length > 0 && (
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-base">Resource Requirements</CardTitle>
              </CardHeader>
              <CardContent>
                <ResourceRequirements models={activeModels} />
              </CardContent>
            </Card>
          )}
        </>
      )}
    </div>
  )
}
