import { useState, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { CategoryActionButton, type CategoryActionState } from "@/components/permissions/CategoryActionButton"
import { PermissionTreeSelector } from "@/components/permissions/PermissionTreeSelector"
import type { TreeNode } from "@/components/permissions/types"
import type {
  ClientGuardrailsConfig,
  GuardrailsConfig,
  SafetyCategoryInfo,
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

export function ClientGuardrailsTab({ client, onUpdate }: ClientGuardrailsTabProps) {
  const [guardrailsConfig, setGuardrailsConfig] = useState<ClientGuardrailsConfig>({
    category_actions: null,
  })
  const [globalConfig, setGlobalConfig] = useState<GuardrailsConfig | null>(null)
  const [categories, setCategories] = useState<SafetyCategoryInfo[]>([])
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

  const hasOverride = guardrailsConfig.category_actions !== null

  const handleOverrideToggle = (enabled: boolean) => {
    if (enabled) {
      // Initialize with a copy of the global category actions
      saveConfig({
        ...guardrailsConfig,
        category_actions: globalConfig?.category_actions ? [...globalConfig.category_actions] : [],
      })
    } else {
      // Clear per-client overrides → inherit global
      saveConfig({ ...guardrailsConfig, category_actions: null })
    }
  }

  // Use per-client actions when overriding, otherwise show global for read-only display
  const displayActions = guardrailsConfig.category_actions
    ?? (globalConfig?.category_actions ?? [])

  // Build category tree nodes (grouped by model type), filtered to only enabled models
  const categoryTreeNodes = useMemo((): TreeNode[] => {
    if (!globalConfig || categories.length === 0) return []

    // Only show categories for model types that have at least one enabled safety model
    const enabledModelTypes = new Set(
      globalConfig.safety_models.filter(m => m.enabled).map(m => m.model_type)
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
  }, [globalConfig, categories])

  // Build permissions map from the displayed actions
  const categoryPermissionsMap = useMemo((): Record<string, CategoryActionState> => {
    const map: Record<string, CategoryActionState> = {}
    for (const entry of displayActions) {
      if (entry.category !== "__global" && entry.action !== "allow") {
        map[entry.category] = entry.action as CategoryActionState
      }
    }
    return map
  }, [displayActions])

  const globalCategoryAction = useMemo((): CategoryActionState => {
    const global = displayActions.find(e => e.category === "__global")
    return (global?.action as CategoryActionState) || "allow"
  }, [displayActions])

  const handleCategoryActionChange = (id: string, action: CategoryActionState) => {
    const actions = (guardrailsConfig.category_actions ?? []).filter(a => a.category !== id)
    actions.push({ category: id, action })
    saveConfig({ ...guardrailsConfig, category_actions: actions })
  }

  const handleGlobalCategoryActionChange = (action: CategoryActionState) => {
    const actions = (guardrailsConfig.category_actions ?? []).filter(a => a.category !== "__global")
    actions.push({ category: "__global", action })
    saveConfig({ ...guardrailsConfig, category_actions: actions })
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FEATURES.guardrails.icon className={`h-5 w-5 ${FEATURES.guardrails.color}`} />
            <CardTitle>GuardRails</CardTitle>
          </div>
          <div className="flex items-center gap-2">
            <Label className="text-xs text-muted-foreground">Override</Label>
            <Switch
              checked={hasOverride}
              onCheckedChange={handleOverrideToggle}
            />
          </div>
        </div>
        <CardDescription>
          {hasOverride
            ? "Custom category actions for this client. Disable the override to revert to global defaults."
            : "Using global default category actions. Enable the override to customize for this client."}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <PermissionTreeSelector<CategoryActionState>
          nodes={categoryTreeNodes}
          permissions={categoryPermissionsMap}
          globalPermission={globalCategoryAction}
          onPermissionChange={handleCategoryActionChange}
          onGlobalChange={handleGlobalCategoryActionChange}
          renderButton={(props) => <CategoryActionButton {...props} />}
          globalLabel="All Categories"
          emptyMessage="No categories available. Add safety models in GuardRails first."
          disabled={!hasOverride}
        />
        <div className="flex items-center justify-between border-t pt-4">
          <div>
            <span className="text-sm font-medium">Approval Popup Preview</span>
            <p className="text-xs text-muted-foreground mt-0.5">
              Preview the popup shown when a guardrail flags content with an &ldquo;Ask&rdquo; action
            </p>
          </div>
          <SamplePopupButton popupType="guardrail" />
        </div>
      </CardContent>
    </Card>
  )
}
