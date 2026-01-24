
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import { Download, FolderOpen, Cpu, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Input } from "@/components/ui/Input"
import { ROUTELLM_REQUIREMENTS } from "@/components/routellm/types"

type RouteLLMState =
  | "not_downloaded"
  | "downloading"
  | "downloaded_not_running"
  | "initializing"
  | "started"

interface RouteLLMStatus {
  state: RouteLLMState
  memory_usage_mb?: number
}

interface RouteLLMTestResult {
  win_rate: number
  is_strong: boolean
  latency_ms: number
}

interface TestHistoryItem {
  prompt: string
  score: number
  isStrong: boolean
  latencyMs: number
  threshold: number
}

const THRESHOLD_PRESETS = [
  { name: "Cost Saving", value: 0.5, description: "Maximize cost savings (more weak model usage)" },
  { name: "Balanced", value: 0.3, description: "Default balanced approach (recommended)" },
  { name: "Quality Optimized", value: 0.1, description: "Prioritize quality (more strong model usage)" },
]

export function RouteLLMTab() {
  const [status, setStatus] = useState<RouteLLMStatus | null>(null)
  const [idleTimeout, setIdleTimeout] = useState(600)
  const [isDownloading, setIsDownloading] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)

  // Threshold testing state
  const [testPrompt, setTestPrompt] = useState("")
  const [testThreshold, setTestThreshold] = useState(0.3)
  const [testHistory, setTestHistory] = useState<TestHistoryItem[]>([])
  const [isTesting, setIsTesting] = useState(false)

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
      toast.success("RouteLLM models downloaded successfully!")
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

  const handleUnload = async () => {
    try {
      await invoke("routellm_unload")
      await loadStatus()
      toast.success("RouteLLM models unloaded from memory")
    } catch (error: any) {
      toast.error(`Unload failed: ${error.message || error}`)
    }
  }

  const updateSettings = async () => {
    try {
      await invoke("routellm_update_settings", {
        idleTimeoutSecs: idleTimeout,
      })
      toast.success("RouteLLM settings updated")
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    }
  }

  const openFolder = async () => {
    try {
      const homeDir = await invoke<string>("get_home_dir")
      await open(`${homeDir}/.localrouter/routellm`)
    } catch (error) {
      console.error("Failed to open folder:", error)
    }
  }

  const handleTest = async () => {
    if (!testPrompt.trim()) return

    setIsTesting(true)
    try {
      const result = await invoke<RouteLLMTestResult>("routellm_test_prediction", {
        prompt: testPrompt.trim(),
        threshold: testThreshold,
      })

      const historyItem: TestHistoryItem = {
        prompt: testPrompt.trim(),
        score: result.win_rate,
        isStrong: result.is_strong,
        latencyMs: result.latency_ms,
        threshold: testThreshold,
      }

      setTestHistory((prev) => [historyItem, ...prev].slice(0, 10))
      setTestPrompt("")
    } catch (err: any) {
      toast.error(`Test failed: ${err.toString()}`)
    } finally {
      setIsTesting(false)
    }
  }

  const getStatusInfo = (state: RouteLLMState) => {
    switch (state) {
      case "not_downloaded":
        return { label: "Not Downloaded", variant: "secondary" as const, icon: "⬇️" }
      case "downloading":
        return { label: "Downloading...", variant: "default" as const, icon: "⏳" }
      case "downloaded_not_running":
        return { label: "Ready (Not Loaded)", variant: "outline" as const, icon: "⏸️" }
      case "initializing":
        return { label: "Initializing...", variant: "default" as const, icon: "🔄" }
      case "started":
        return { label: "Active in Memory", variant: "success" as const, icon: "✓" }
      default:
        return { label: "Unknown", variant: "secondary" as const, icon: "?" }
    }
  }

  const isReady =
    status?.state !== "not_downloaded" && status?.state !== "downloading"

  return (
    <div className="space-y-6">
      {/* Header */}
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Cpu className="h-5 w-5" />
              <div>
                <CardTitle className="text-sm">RouteLLM Intelligent Routing</CardTitle>
                <CardDescription>
                  ML-based routing to optimize costs while maintaining quality
                </CardDescription>
              </div>
            </div>
            <Badge variant="outline" className="bg-purple-500/10 text-purple-600">
              EXPERIMENTAL
            </Badge>
          </div>
        </CardHeader>
        <CardContent>
          {status && (
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <span className="text-2xl">{getStatusInfo(status.state).icon}</span>
                <div>
                  <Badge variant={getStatusInfo(status.state).variant}>
                    {getStatusInfo(status.state).label}
                  </Badge>
                  {status.memory_usage_mb && (
                    <p className="text-xs text-muted-foreground mt-1">
                      Memory: {(status.memory_usage_mb / 1024).toFixed(2)} GB
                    </p>
                  )}
                </div>
              </div>
              <div className="flex gap-2">
                {status.state === "started" && (
                  <Button variant="outline" size="sm" onClick={handleUnload}>
                    <Trash2 className="h-3 w-3 mr-1" />
                    Unload
                  </Button>
                )}
                <Button variant="ghost" size="sm" onClick={openFolder}>
                  <FolderOpen className="h-3 w-3 mr-1" />
                  Open Folder
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Download Section */}
      {status?.state === "not_downloaded" && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm">Download Models</CardTitle>
            <CardDescription>
              RouteLLM uses machine learning to analyze prompts and route to the most
              cost-effective model while maintaining quality.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-3 gap-3 text-center">
              <div className="p-3 bg-green-500/10 rounded-lg">
                <p className="text-lg font-bold text-green-600">30-60%</p>
                <p className="text-xs text-muted-foreground">Cost Savings</p>
              </div>
              <div className="p-3 bg-blue-500/10 rounded-lg">
                <p className="text-lg font-bold text-blue-600">85-95%</p>
                <p className="text-xs text-muted-foreground">Quality Retained</p>
              </div>
              <div className="p-3 bg-purple-500/10 rounded-lg">
                <p className="text-lg font-bold text-purple-600">{ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms</p>
                <p className="text-xs text-muted-foreground">Routing Time</p>
              </div>
            </div>

            <div className="p-3 bg-yellow-500/10 border border-yellow-500/20 rounded-lg">
              <p className="text-xs text-yellow-600 dark:text-yellow-400">
                <strong>Download Required:</strong> Models ({ROUTELLM_REQUIREMENTS.DISK_GB} GB) will be downloaded to{" "}
                <code className="bg-yellow-500/20 px-1 rounded">~/.localrouter/routellm/</code>
              </p>
            </div>

            <Button onClick={handleDownload} disabled={isDownloading}>
              <Download className="h-4 w-4 mr-2" />
              {isDownloading ? `Downloading... ${downloadProgress.toFixed(0)}%` : "Download Models"}
            </Button>
          </CardContent>
        </Card>
      )}

      {/* Download Progress */}
      {isDownloading && (
        <Card>
          <CardContent className="pt-6">
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>Downloading RouteLLM Models...</span>
                <span>{downloadProgress.toFixed(0)}%</span>
              </div>
              <Progress value={downloadProgress} />
            </div>
          </CardContent>
        </Card>
      )}

      {/* Settings - Only when downloaded */}
      {isReady && (
        <>
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm">Memory Management</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <Label>Auto-Unload After Idle</Label>
                <Select
                  value={idleTimeout.toString()}
                  onValueChange={(value) => setIdleTimeout(parseInt(value))}
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

              <Button size="sm" onClick={updateSettings}>
                Save Settings
              </Button>
            </CardContent>
          </Card>

          {/* Resource Info */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm">Resource Requirements</CardTitle>
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

          {/* Threshold Testing */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm">Threshold Testing</CardTitle>
              <CardDescription>
                Test prompts to see routing decisions and confidence scores
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {/* Threshold Slider */}
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <Label>Threshold</Label>
                  <span className="text-sm font-mono text-blue-600">
                    {testThreshold.toFixed(2)}
                  </span>
                </div>

                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={testThreshold}
                  onChange={(e) => setTestThreshold(parseFloat(e.target.value))}
                  className="w-full h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-blue-500"
                />

                <div className="flex gap-2">
                  {THRESHOLD_PRESETS.map((preset) => (
                    <Button
                      key={preset.name}
                      variant={Math.abs(preset.value - testThreshold) < 0.01 ? "default" : "outline"}
                      size="sm"
                      className="flex-1"
                      onClick={() => setTestThreshold(preset.value)}
                    >
                      {preset.name}
                    </Button>
                  ))}
                </div>

                {THRESHOLD_PRESETS.find((p) => Math.abs(p.value - testThreshold) < 0.01)?.description && (
                  <p className="text-xs text-muted-foreground italic">
                    {THRESHOLD_PRESETS.find((p) => Math.abs(p.value - testThreshold) < 0.01)?.description}
                  </p>
                )}
              </div>

              {/* Test Input */}
              <div className="flex gap-2">
                <Input
                  value={testPrompt}
                  onChange={(e) => setTestPrompt(e.target.value)}
                  placeholder="Type a prompt and press Enter..."
                  onKeyDown={(e) => e.key === "Enter" && !isTesting && handleTest()}
                />
                <Button onClick={handleTest} disabled={isTesting || !testPrompt.trim()}>
                  {isTesting ? "Testing..." : "Test"}
                </Button>
              </div>

              {/* Test History */}
              {testHistory.length > 0 && (
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label>Test History</Label>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setTestHistory([])}
                    >
                      Clear
                    </Button>
                  </div>

                  <div className="space-y-2 max-h-64 overflow-y-auto">
                    {testHistory.map((item, idx) => (
                      <div
                        key={idx}
                        className="p-3 bg-muted rounded-lg space-y-2"
                      >
                        <p className="text-sm">
                          <span className="text-muted-foreground">&gt;</span> {item.prompt}
                        </p>
                        <div className="flex items-center justify-between text-xs">
                          <span className="text-muted-foreground">
                            Score:{" "}
                            <span className="font-mono text-blue-600">
                              {item.score.toFixed(3)}
                            </span>
                          </span>
                          <Badge variant={item.isStrong ? "default" : "secondary"}>
                            {item.isStrong ? "STRONG" : "weak"} model
                          </Badge>
                        </div>
                        <div className="w-full bg-background rounded h-2 overflow-hidden">
                          <div
                            className="h-full bg-gradient-to-r from-green-500 to-orange-500"
                            style={{ width: `${item.score * 100}%` }}
                          />
                        </div>
                        <div className="flex gap-4 text-xs text-muted-foreground">
                          <span>Threshold: {item.threshold.toFixed(2)}</span>
                          <span>Latency: {item.latencyMs}ms</span>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  )
}
