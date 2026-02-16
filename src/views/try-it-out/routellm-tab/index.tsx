import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Download, Cpu } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Progress } from "@/components/ui/progress"
import { ROUTELLM_REQUIREMENTS } from "@/components/routellm/types"
import { ThresholdSelector } from "@/components/routellm/ThresholdSelector"
import type { RouteLLMStatus, RouteLLMState } from "@/types/tauri-commands"

export function RouteLLMTryItOutTab() {
  const [status, setStatus] = useState<RouteLLMStatus | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [testThreshold, setTestThreshold] = useState(0.3)

  useEffect(() => {
    loadStatus()

    const unlistenProgress = listen("routellm-download-progress", (event: any) => {
      const { progress } = event.payload
      setDownloadProgress(progress * 100)
    })

    const unlistenComplete = listen("routellm-download-complete", () => {
      setIsDownloading(false)
      setDownloadProgress(100)
      loadStatus()
      toast.success("Strong/Weak models downloaded successfully!")
    })

    const unlistenFailed = listen("routellm-download-failed", (event: any) => {
      setIsDownloading(false)
      toast.error(`Download failed: ${event.payload.error}`)
    })

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenComplete.then((fn) => fn())
      unlistenFailed.then((fn) => fn())
    }
  }, [])

  const loadStatus = async () => {
    try {
      const routellmStatus = await invoke<RouteLLMStatus>("routellm_get_status")
      setStatus(routellmStatus)
    } catch (error) {
      console.error("Failed to load RouteLLM status:", error)
    }
  }

  const handleDownload = async () => {
    setIsDownloading(true)
    setDownloadProgress(0)

    try {
      await invoke("routellm_download_models")
    } catch (error: any) {
      console.error("Failed to start download:", error)
      toast.error(`Download failed: ${error.message || error}`)
      setIsDownloading(false)
    }
  }

  const getStatusInfo = (state: RouteLLMState) => {
    switch (state) {
      case "not_downloaded":
        return { label: "Not Downloaded", variant: "secondary" as const }
      case "downloading":
        return { label: "Downloading...", variant: "default" as const }
      case "downloaded_not_running":
        return { label: "Model not loaded", variant: "outline" as const }
      case "initializing":
        return { label: "Loading...", variant: "default" as const }
      case "started":
        return { label: "Model loaded", variant: "success" as const }
      default:
        return { label: "Unknown", variant: "secondary" as const }
    }
  }

  const isReady =
    status?.state !== "not_downloaded" && status?.state !== "downloading"

  return (
    <div className="space-y-6">
      {/* Status Banner */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Cpu className="h-5 w-5" />
              <div>
                <CardTitle className="text-sm">Strong/Weak Intelligent Routing</CardTitle>
                <CardDescription>
                  Test prompts to see how the model classifies complexity and routes between strong and weak models
                </CardDescription>
              </div>
            </div>
            {status && (
              <Badge variant={getStatusInfo(status.state).variant}>
                {getStatusInfo(status.state).label}
              </Badge>
            )}
          </div>
        </CardHeader>
        {status?.state === "not_downloaded" && !isDownloading && (
          <CardContent className="pt-0">
            <div className="space-y-3">
              <div className="p-3 bg-yellow-500/10 border border-yellow-600/50 rounded-lg">
                <p className="text-xs text-yellow-900 dark:text-yellow-400">
                  <strong>Download Required:</strong> Models ({ROUTELLM_REQUIREMENTS.DISK_GB} GB) will be downloaded to{" "}
                  <code className="bg-yellow-500/20 px-1 rounded">~/.localrouter/routellm/</code>
                </p>
              </div>
              <Button variant="outline" size="sm" onClick={handleDownload}>
                <Download className="h-3 w-3 mr-1" />
                Download Models
              </Button>
            </div>
          </CardContent>
        )}
      </Card>

      {/* Download Progress */}
      {isDownloading && (
        <Card>
          <CardContent className="pt-6">
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>Downloading Strong/Weak Models...</span>
                <span>{downloadProgress.toFixed(0)}%</span>
              </div>
              <Progress value={downloadProgress} />
            </div>
          </CardContent>
        </Card>
      )}

      {/* Try It Out - Only when downloaded */}
      {isReady && (
        <Card>
          <CardContent className="pt-6">
            <ThresholdSelector
              value={testThreshold}
              onChange={setTestThreshold}
              showTryItOut
            />
          </CardContent>
        </Card>
      )}
    </div>
  )
}
