import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Trash2, Download, AlertTriangle } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { Progress } from "@/components/ui/progress"
import type { SafetyModelConfig, ProviderModelPullProgress, PullProviderModelParams } from "@/types/tauri-commands"

interface SafetyModelListProps {
  models: SafetyModelConfig[]
  pullProgress?: Record<string, ProviderModelPullProgress>
  loadErrors?: Record<string, string>
  onRemove?: (modelId: string) => void
  readOnly?: boolean
}

export function SafetyModelList({
  models,
  pullProgress = {},
  loadErrors = {},
  onRemove,
  readOnly = false,
}: SafetyModelListProps) {
  if (models.length === 0) {
    return (
      <p className="text-sm text-muted-foreground py-4 text-center">
        No safety models configured. Add one to get started.
      </p>
    )
  }

  const handlePullModel = async (providerId: string, modelName: string) => {
    try {
      await invoke("pull_provider_model", {
        providerId,
        modelName,
      } satisfies PullProviderModelParams as Record<string, unknown>)
    } catch (err) {
      toast.error(`Failed to pull model: ${err}`)
    }
  }

  return (
    <div className="space-y-2">
      {models.map((model) => {
        const loadError = loadErrors[model.id]
        const pullKey = model.provider_id && model.model_name ? `${model.provider_id}:${model.model_name}` : null
        const pulling = pullKey ? pullProgress[pullKey] : undefined
        const isPulling = pulling !== undefined

        // Model identifier: provider/model
        const modelIdentifier = model.provider_id && model.model_name
          ? `${model.provider_id}/${model.model_name}`
          : model.model_name || model.provider_id || null

        // Calculate pull progress percentage
        const pullPercent = pulling?.total && pulling?.completed
          ? Math.round((pulling.completed / pulling.total) * 100)
          : null

        return (
          <div
            key={model.id}
            className="py-2 px-3 rounded-md border bg-card space-y-1"
          >
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-sm font-medium truncate">{model.label}</span>
                {loadError ? (
                  <Badge variant="destructive" className="text-xs shrink-0" title={loadError}>
                    <AlertTriangle className="h-3 w-3 mr-1" />Load failed
                  </Badge>
                ) : null}
              </div>

              <div className="flex items-center gap-2">
                {isPulling && (
                  <div className="flex items-center gap-2 w-40">
                    {pullPercent !== null ? (
                      <>
                        <Progress value={pullPercent} className="h-1.5" />
                        <span className="text-xs text-muted-foreground">{pullPercent}%</span>
                      </>
                    ) : (
                      <span className="text-xs text-muted-foreground truncate">{pulling?.status}</span>
                    )}
                  </div>
                )}
                {!readOnly && model.provider_id && model.model_name && !isPulling && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handlePullModel(model.provider_id!, model.model_name!)}
                    title="Pull model from provider (Ollama)"
                  >
                    <Download className="h-4 w-4 text-muted-foreground" />
                  </Button>
                )}
                {!readOnly && onRemove && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onRemove(model.id)}
                    disabled={isPulling}
                  >
                    <Trash2 className="h-4 w-4 text-muted-foreground" />
                  </Button>
                )}
              </div>
            </div>

            {/* Model identifier */}
            {modelIdentifier && (
              <div className="text-[11px] text-muted-foreground font-mono truncate pl-0.5">
                {modelIdentifier}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}
