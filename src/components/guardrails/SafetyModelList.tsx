import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Trash2, Download, Cloud, FolderOpen, RotateCcw, AlertTriangle } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { Progress } from "@/components/ui/progress"
import type { SafetyModelConfig, SafetyModelDownloadStatus } from "@/types/tauri-commands"

interface SafetyModelListProps {
  models: SafetyModelConfig[]
  downloadStatuses: Record<string, SafetyModelDownloadStatus>
  downloadProgress: Record<string, number>
  loadErrors?: Record<string, string>
  onRemove?: (modelId: string) => void
  onRetryDownload?: (modelId: string) => void
  onRetryCorruptModel?: (modelId: string) => void
  readOnly?: boolean
}

function formatFileSize(bytes: number): string {
  if (bytes < 1_048_576) return `${(bytes / 1024).toFixed(0)} KB`
  if (bytes < 1_073_741_824) return `${(bytes / 1_048_576).toFixed(0)} MB`
  return `${(bytes / 1_073_741_824).toFixed(1)} GB`
}

export function SafetyModelList({
  models,
  downloadStatuses,
  downloadProgress,
  loadErrors = {},
  onRemove,
  onRetryDownload,
  onRetryCorruptModel,
  readOnly = false,
}: SafetyModelListProps) {
  const [modelsDir, setModelsDir] = useState<string | null>(null)

  useEffect(() => {
    if (!readOnly) {
      invoke<string>("get_safety_models_dir").then(setModelsDir).catch(() => {})
    }
  }, [readOnly])

  if (models.length === 0) {
    return (
      <p className="text-sm text-muted-foreground py-4 text-center">
        No safety models configured. Add one to get started.
      </p>
    )
  }

  const openModelsDir = () => {
    if (modelsDir) {
      invoke("open_path", { path: modelsDir }).catch(() => {})
    }
  }

  return (
    <div className="space-y-2">
      {models.map((model) => {
        const status = downloadStatuses[model.id]
        const progress = downloadProgress[model.id]
        const isDownloaded = status?.downloaded
        const isDownloading = progress !== undefined && progress < 100
        const isDirectDownload = model.execution_mode === "direct_download" || model.execution_mode === "custom_download"
        const loadError = loadErrors[model.id]

        // Model identifier: HF repo/filename for direct, provider/model for provider
        const modelIdentifier = isDirectDownload
          ? (model.hf_repo_id && model.gguf_filename
              ? `${model.hf_repo_id}/${model.gguf_filename}`
              : model.hf_repo_id || null)
          : (model.provider_id && model.model_name
              ? `${model.provider_id}/${model.model_name}`
              : model.model_name || model.provider_id || null)

        return (
          <div
            key={model.id}
            className="py-2 px-3 rounded-md border bg-card space-y-1"
          >
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-sm font-medium truncate">{model.label}</span>
                <Badge variant="outline" className="text-xs shrink-0">
                  {isDirectDownload ? (
                    <><Download className="h-3 w-3 mr-1" />Direct</>
                  ) : (
                    <><Cloud className="h-3 w-3 mr-1" />Provider</>
                  )}
                </Badge>
                {loadError ? (
                  <Badge variant="destructive" className="text-xs shrink-0" title={loadError}>
                    <AlertTriangle className="h-3 w-3 mr-1" />Load failed
                  </Badge>
                ) : isDownloaded ? (
                  <Badge variant="secondary" className="text-xs shrink-0">Ready</Badge>
                ) : null}
              </div>

              <div className="flex items-center gap-2">
                {isDownloading && (
                  <div className="flex items-center gap-2 w-32">
                    <Progress value={progress} className="h-1.5" />
                    <span className="text-xs text-muted-foreground">{Math.round(progress)}%</span>
                  </div>
                )}
                {!readOnly && isDirectDownload && loadError && onRetryCorruptModel && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onRetryCorruptModel(model.id)}
                    title="Delete corrupt file and re-download"
                  >
                    <RotateCcw className="h-4 w-4 text-destructive" />
                  </Button>
                )}
                {!readOnly && isDirectDownload && !isDownloaded && !isDownloading && !loadError && onRetryDownload && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onRetryDownload(model.id)}
                    title="Retry download"
                  >
                    <RotateCcw className="h-4 w-4 text-muted-foreground" />
                  </Button>
                )}
                {!readOnly && onRemove && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onRemove(model.id)}
                    disabled={isDownloading}
                  >
                    <Trash2 className="h-4 w-4 text-muted-foreground" />
                  </Button>
                )}
              </div>
            </div>

            {/* Model identifier and file info */}
            {(modelIdentifier || (isDownloaded && status?.file_size)) && (
              <div className="text-[11px] text-muted-foreground font-mono truncate pl-0.5">
                {modelIdentifier}
                {isDownloaded && status?.file_size != null && (
                  <span className="ml-2 text-[10px]">({formatFileSize(status.file_size)})</span>
                )}
              </div>
            )}
          </div>
        )
      })}

      {/* Models directory link */}
      {!readOnly && modelsDir && Object.values(downloadStatuses).some(s => s.downloaded) && (
        <button
          onClick={openModelsDir}
          className="flex items-center gap-1.5 text-[11px] text-muted-foreground hover:text-foreground transition-colors pt-1"
        >
          <FolderOpen className="h-3 w-3" />
          <span className="truncate">{modelsDir}</span>
        </button>
      )}
    </div>
  )
}
