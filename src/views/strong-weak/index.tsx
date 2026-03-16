import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Download, FolderOpen, Trash2, Loader2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { ROUTELLM_REQUIREMENTS } from "@/components/routellm/types"
import { FeatureClientsCard } from "@/components/shared/FeatureClientsCard"
import { ThresholdSelector } from "@/components/routellm/ThresholdSelector"
import type { RouteLLMStatus, RouteLLMState } from "@/types/tauri-commands"

interface StrongWeakViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function StrongWeakView({ activeSubTab, onTabChange }: StrongWeakViewProps) {
  const [status, setStatus] = useState<RouteLLMStatus | null>(null)
  const [idleTimeout, setIdleTimeout] = useState(600)
  const [isDownloading, setIsDownloading] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [testThreshold, setTestThreshold] = useState(0.3)

  const tab = activeSubTab || "model"

  useEffect(() => {
    loadStatus()

    const lProgress = listenSafe("routellm-download-progress", (event: any) => {
      const { progress } = event.payload
      setDownloadProgress(progress * 100)
    })

    const lComplete = listenSafe("routellm-download-complete", () => {
      setIsDownloading(false)
      setDownloadProgress(100)
      loadStatus()
      toast.success("Strong/Weak models downloaded successfully!")
    })

    const lFailed = listenSafe("routellm-download-failed", (event: any) => {
      setIsDownloading(false)
      toast.error(`Download failed: ${event.payload.error}`)
    })

    // Poll status to detect state changes (model loaded/unloaded)
    const interval = setInterval(loadStatus, 3000)

    return () => {
      lProgress.cleanup()
      lComplete.cleanup()
      lFailed.cleanup()
      clearInterval(interval)
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

  const updateSettings = async (newTimeout: number) => {
    try {
      await invoke("routellm_update_settings", {
        idleTimeoutSecs: newTimeout,
      })
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    }
  }

  const handleDeleteModel = async () => {
    setIsDeleting(true)
    try {
      await invoke("routellm_delete_model")
      toast.success("Strong/Weak model deleted from disk")
      loadStatus()
    } catch (error: any) {
      toast.error(`Failed to delete: ${error.message || error}`)
    } finally {
      setIsDeleting(false)
    }
  }

  const openFolder = async () => {
    try {
      await invoke("open_routellm_folder")
    } catch (error) {
      console.error("Failed to open folder:", error)
      toast.error("Failed to open folder")
    }
  }

  const getStatusInfo = (state: RouteLLMState) => {
    switch (state) {
      case "not_downloaded":
        return { label: "Not Downloaded", variant: "secondary" as const }
      case "downloading":
        return { label: "Downloading...", variant: "default" as const }
      case "downloaded_not_running":
        return { label: "Model unloaded", variant: "outline" as const }
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

  const handleTabChange = (newTab: string) => {
    onTabChange("strong-weak", newTab)
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <FEATURES.routing.icon className={`h-6 w-6 ${FEATURES.routing.color}`} />
          Strong/Weak
          {FEATURES.routing.experimental && <ExperimentalBadge />}
        </h1>
        <p className="text-sm text-muted-foreground">
          Intelligent routing that analyzes complexity to select the most cost-effective model — typically saving 30-60% on costs while retaining 85-95% quality, with only {ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms of selection overhead
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="model"><TAB_ICONS.model className={TAB_ICON_CLASS} />Model</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Model Tab */}
        <TabsContent value="model" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {/* Resource Requirements */}
            <Card className="border-yellow-600/50 bg-yellow-500/5">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm text-yellow-900 dark:text-yellow-400">Resource Requirements</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="grid grid-cols-2 gap-4 text-sm">
                  <div>
                    <span className="text-muted-foreground">Cold Start:</span>{" "}
                    <span className="font-medium">{ROUTELLM_REQUIREMENTS.COLD_START_SECS}s</span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">Disk Space:</span>{" "}
                    <span className="font-medium">{ROUTELLM_REQUIREMENTS.DISK_GB} GB</span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">Latency:</span>{" "}
                    <span className="font-medium">{ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms per request</span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">Memory:</span>{" "}
                    <span className="font-medium">{ROUTELLM_REQUIREMENTS.MEMORY_GB} GB (when loaded)</span>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Status */}
            {status && (
              <Card>
                <CardHeader className="pb-3">
                  <CardDescription>
                    Analyzes each request's complexity to route simple queries to faster, cheaper models and complex ones to stronger models
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <Badge variant={getStatusInfo(status.state).variant}>
                        {getStatusInfo(status.state).label}
                      </Badge>
                      <span className="text-xs text-muted-foreground font-mono">
                        {status.model_name}
                      </span>
                    </div>
                    <div className="flex gap-2">
                      {status.state === "not_downloaded" && !isDownloading && (
                        <Button variant="outline" size="sm" onClick={handleDownload}>
                          <Download className="h-3 w-3 mr-1" />
                          Download
                        </Button>
                      )}
                      <Button variant="ghost" size="sm" onClick={openFolder}>
                        <FolderOpen className="h-3 w-3 mr-1" />
                        Open Folder
                      </Button>
                    </div>
                  </div>
                </CardContent>
              </Card>
            )}

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

            <FeatureClientsCard feature="strong_weak" onNavigateToClient={onTabChange} />
          </div>
        </TabsContent>

        {/* Try It Out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <div className="max-w-2xl">
            {!isReady ? (
              <Card>
                <CardContent className="pt-6">
                  <p className="text-sm text-muted-foreground text-center py-4">
                    Download the model first to use Try It Out
                  </p>
                </CardContent>
              </Card>
            ) : (
              <Card>
                <CardContent className="pt-6">
                  <ThresholdSelector
                    value={testThreshold}
                    onChange={setTestThreshold}
                    showTryItOut
                    disabled={!isReady}
                  />
                </CardContent>
              </Card>
            )}
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Memory Management</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-2">
                  <Label>Auto-Unload After Idle</Label>
                  <Select
                    value={idleTimeout.toString()}
                    onValueChange={(value) => {
                      const timeout = parseInt(value)
                      setIdleTimeout(timeout)
                      updateSettings(timeout)
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="300">5 minutes</SelectItem>
                      <SelectItem value="600">10 minutes (recommended)</SelectItem>
                      <SelectItem value="1800">30 minutes</SelectItem>
                      <SelectItem value="3600">1 hour</SelectItem>
                      <SelectItem value="0">Never</SelectItem>
                    </SelectContent>
                  </Select>
                  <p className="text-xs text-muted-foreground">
                    Automatically unload models after inactivity to save RAM ({ROUTELLM_REQUIREMENTS.MEMORY_GB} GB)
                  </p>
                </div>
              </CardContent>
            </Card>

            {/* Delete Model */}
            <Card className="border-destructive/50">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm text-destructive">Delete Model</CardTitle>
                <CardDescription>
                  Permanently delete the downloaded model files from disk ({ROUTELLM_REQUIREMENTS.DISK_GB} GB). You can re-download later.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <Button
                      variant="destructive"
                      size="sm"
                      disabled={isDeleting || status?.state === "not_downloaded" || isDownloading}
                    >
                      {isDeleting ? (
                        <><Loader2 className="h-3 w-3 mr-1 animate-spin" />Deleting...</>
                      ) : (
                        <><Trash2 className="h-3 w-3 mr-1" />Delete from Disk</>
                      )}
                    </Button>
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>Delete Strong/Weak model?</AlertDialogTitle>
                      <AlertDialogDescription>
                        This will permanently delete the model files from disk. You will need to download them again to use Strong/Weak routing.
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>Cancel</AlertDialogCancel>
                      <AlertDialogAction onClick={handleDeleteModel}>Delete</AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
