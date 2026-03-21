import { Download, CheckCircle2, XCircle, Loader2, RefreshCw } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Progress } from "@/components/ui/progress"
import type { DownloadStatus } from "@/hooks/useModelDownload"

interface ModelDownloadCardProps {
  /** Card title */
  title: string
  /** Card description (shown in all states) */
  description?: string
  /** Model name shown on success */
  modelName?: string
  /** Additional info badge on success (e.g. "80 MB") */
  modelInfo?: string
  /** Current download status */
  status: DownloadStatus
  /** Progress 0-100 */
  progress: number
  /** Error message when failed */
  error: string | null
  /** Start download */
  onDownload: () => void
  /** Retry after failure */
  onRetry: () => void
  /** Label for the download button */
  downloadLabel?: string
  /** Extra content below status (e.g. benchmark tables when downloaded) */
  children?: React.ReactNode
}

export function ModelDownloadCard({
  title,
  description,
  modelName,
  modelInfo,
  status,
  progress,
  error,
  onDownload,
  onRetry,
  downloadLabel = "Download",
  children,
}: ModelDownloadCardProps) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <CardTitle className="text-sm">{title}</CardTitle>
          {status === "downloaded" && (
            <Badge variant="success" className="text-[10px] px-1.5 py-0">
              {modelInfo || "ready"}
            </Badge>
          )}
        </div>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-3">
        {/* Idle — show download button */}
        {status === "idle" && (
          <Button variant="outline" size="sm" onClick={onDownload}>
            <Download className="h-3.5 w-3.5 mr-1.5" />
            {downloadLabel}
          </Button>
        )}

        {/* Downloading — show progress bar */}
        {status === "downloading" && (
          <div className="space-y-2">
            <div className="flex justify-between text-sm text-muted-foreground">
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3 w-3 animate-spin" />
                Downloading...
              </span>
              <span>{progress.toFixed(0)}%</span>
            </div>
            <Progress value={progress} />
          </div>
        )}

        {/* Downloaded — show success status */}
        {status === "downloaded" && (
          <div className="flex items-center gap-2 text-sm">
            <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
            {modelName && <span className="font-medium">{modelName}</span>}
          </div>
        )}

        {/* Failed — show error and retry */}
        {status === "failed" && (
          <div className="space-y-2">
            <div className="flex items-center gap-2 text-sm text-destructive">
              <XCircle className="h-4 w-4 shrink-0" />
              <span className="text-xs">{error || "Download failed"}</span>
            </div>
            <Button variant="outline" size="sm" onClick={onRetry}>
              <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
              Retry
            </Button>
          </div>
        )}

        {children}
      </CardContent>
    </Card>
  )
}
