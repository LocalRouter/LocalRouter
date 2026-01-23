
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { check, Update } from "@tauri-apps/plugin-updater"
import { relaunch } from "@tauri-apps/plugin-process"
import { toast } from "sonner"
import { RefreshCw, Download, SkipForward, Info } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import { Switch } from "@/components/ui/Toggle"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import ReactMarkdown from "react-markdown"

interface UpdateConfig {
  mode: "manual" | "automatic"
  check_interval_days: number
  last_check?: string
  skipped_version?: string
}

export function UpdatesTab() {
  const [currentVersion, setCurrentVersion] = useState<string>("")
  const [updateConfig, setUpdateConfig] = useState<UpdateConfig>({
    mode: "automatic",
    check_interval_days: 7,
  })
  const [isChecking, setIsChecking] = useState(false)
  const [checkError, setCheckError] = useState<string | null>(null)
  const [updateAvailable, setUpdateAvailable] = useState<Update | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)

  useEffect(() => {
    loadCurrentVersion()
    loadUpdateConfig()
    checkForUpdatesOnMount()

    const unlistenUpdateCheck = listen("check-for-updates", () => {
      handleCheckForUpdates()
    })

    return () => {
      unlistenUpdateCheck.then((fn) => fn())
    }
  }, [])

  const loadCurrentVersion = async () => {
    try {
      const version = await invoke<string>("get_app_version")
      setCurrentVersion(version)
    } catch (err) {
      console.error("Failed to get app version:", err)
    }
  }

  const loadUpdateConfig = async () => {
    try {
      const config = await invoke<UpdateConfig>("get_update_config")
      setUpdateConfig(config)
    } catch (err) {
      console.error("Failed to load update config:", err)
    }
  }

  const checkForUpdatesOnMount = async () => {
    try {
      const config = await invoke<UpdateConfig>("get_update_config")
      if (config.mode === "automatic") {
        setTimeout(() => {
          handleCheckForUpdates()
        }, 500)
      }
    } catch (err) {
      console.error("Failed to check update config:", err)
    }
  }

  const handleCheckForUpdates = async () => {
    setIsChecking(true)
    setCheckError(null)

    try {
      const update = await check()

      await invoke("mark_update_check_performed")

      if (update?.available) {
        if (updateConfig.skipped_version === update.version) {
          toast.success("Already up to date (skipped version ignored)")
          setUpdateAvailable(null)
          await invoke("set_update_notification", { available: false })
        } else {
          setUpdateAvailable(update)
          toast.success(`New version ${update.version} available!`)
          await invoke("set_update_notification", { available: true })
        }
      } else {
        setUpdateAvailable(null)
        toast.success("Already up to date")
        await invoke("set_update_notification", { available: false })
      }
    } catch (err: any) {
      setCheckError(err.message || "Failed to check for updates")
      toast.error(`Check failed: ${err.message}`)
    } finally {
      setIsChecking(false)
    }
  }

  const handleUpdateNow = async () => {
    if (!updateAvailable) return

    setIsDownloading(true)

    try {
      await updateAvailable.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            setDownloadProgress(0)
            break
          case "Progress":
            setDownloadProgress((prev) => Math.min(prev + 5, 95))
            break
          case "Finished":
            setDownloadProgress(100)
            break
        }
      })

      toast.success("Update installed! Restarting...")
      await invoke("set_update_notification", { available: false })

      setTimeout(async () => {
        await relaunch()
      }, 2000)
    } catch (err: any) {
      toast.error(`Update failed: ${err.message}`)
      setIsDownloading(false)
    }
  }

  const handleSkipVersion = async () => {
    if (!updateAvailable) return

    try {
      await invoke("skip_update_version", { version: updateAvailable.version })
      setUpdateAvailable(null)
      toast.success(`Skipped version ${updateAvailable.version}`)
      loadUpdateConfig()
    } catch (err: any) {
      toast.error(`Failed to skip version: ${err.message}`)
    }
  }

  const handleUpdateConfig = async (updates: Partial<UpdateConfig>) => {
    const newConfig = { ...updateConfig, ...updates }

    try {
      await invoke("update_update_config", {
        mode: newConfig.mode,
        checkIntervalDays: newConfig.check_interval_days,
      })
      setUpdateConfig(newConfig)
      toast.success("Settings saved")
    } catch (err: any) {
      toast.error(`Failed to save settings: ${err.message}`)
    }
  }

  const formatLastCheck = (lastCheck?: string | null) => {
    if (!lastCheck) return "Never"

    try {
      const date = new Date(lastCheck)
      const now = new Date()
      const diffMs = now.getTime() - date.getTime()
      const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24))

      if (diffDays === 0) return "Today"
      if (diffDays === 1) return "1 day ago"
      if (diffDays < 7) return `${diffDays} days ago`
      if (diffDays < 30) {
        const weeks = Math.floor(diffDays / 7)
        return weeks === 1 ? "1 week ago" : `${weeks} weeks ago`
      }
      const months = Math.floor(diffDays / 30)
      return months === 1 ? "1 month ago" : `${months} months ago`
    } catch {
      return "Never"
    }
  }

  return (
    <div className="space-y-6">
      {/* Version Info */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">App Version</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4">
              <div>
                <p className="text-xs text-muted-foreground">Current Version</p>
                <p className="text-lg font-bold">{currentVersion || "Loading..."}</p>
              </div>
              {updateAvailable && (
                <div>
                  <p className="text-xs text-muted-foreground">Latest Version</p>
                  <p className="text-lg font-bold text-blue-600">
                    {updateAvailable.version}
                  </p>
                </div>
              )}
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={handleCheckForUpdates}
              disabled={isChecking || isDownloading}
            >
              <RefreshCw className={`h-3 w-3 mr-1 ${isChecking ? "animate-spin" : ""}`} />
              {isChecking ? "Checking..." : "Check Now"}
            </Button>
          </div>
          {checkError && (
            <p className="text-xs text-red-500 mt-2">{checkError}</p>
          )}
        </CardContent>
      </Card>

      {/* Update Available */}
      {updateAvailable && !isDownloading && (
        <Card className="border-blue-500/50">
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <CardTitle className="text-sm text-blue-600">
                Update Available: {updateAvailable.version}
              </CardTitle>
              {updateAvailable.date && (
                <Badge variant="outline">
                  {new Date(updateAvailable.date).toLocaleDateString()}
                </Badge>
              )}
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {updateAvailable.body && (
              <div className="prose prose-sm dark:prose-invert max-w-none p-3 bg-muted rounded-lg max-h-48 overflow-y-auto">
                <ReactMarkdown>{updateAvailable.body}</ReactMarkdown>
              </div>
            )}

            <div className="flex gap-2">
              <Button onClick={handleUpdateNow}>
                <Download className="h-4 w-4 mr-2" />
                Update Now
              </Button>
              <Button variant="outline" onClick={handleSkipVersion}>
                <SkipForward className="h-4 w-4 mr-2" />
                Skip This Version
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Download Progress */}
      {isDownloading && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm">Installing Update...</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <Progress value={downloadProgress} />
            <p className="text-xs text-muted-foreground text-center">
              {Math.round(downloadProgress)}%
            </p>
          </CardContent>
        </Card>
      )}

      {/* Update Settings */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Update Settings</CardTitle>
          <CardDescription>
            Configure how LocalRouter checks for updates
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label>Automatically check for updates</Label>
              <p className="text-xs text-muted-foreground">
                Check for new versions in the background
              </p>
            </div>
            <Switch
              checked={updateConfig.mode === "automatic"}
              onCheckedChange={(checked) =>
                handleUpdateConfig({ mode: checked ? "automatic" : "manual" })
              }
            />
          </div>

          {updateConfig.mode === "automatic" && (
            <div className="space-y-2">
              <Label>Check Interval</Label>
              <Select
                value={updateConfig.check_interval_days.toString()}
                onValueChange={(value) =>
                  handleUpdateConfig({ check_interval_days: parseInt(value) })
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="1">1 day</SelectItem>
                  <SelectItem value="7">7 days (recommended)</SelectItem>
                  <SelectItem value="14">14 days</SelectItem>
                  <SelectItem value="30">30 days</SelectItem>
                </SelectContent>
              </Select>
            </div>
          )}

          <div className="p-3 bg-muted rounded-lg flex items-center gap-2">
            <Info className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <p className="text-xs text-muted-foreground">
              Last checked: {formatLastCheck(updateConfig.last_check)}
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
