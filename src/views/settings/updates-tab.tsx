import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { check, Update } from "@tauri-apps/plugin-updater"
import { relaunch } from "@tauri-apps/plugin-process"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import { RefreshCw, Download, SkipForward, Info, Heart, Code, Cpu, ChevronDown, ChevronRight, ExternalLink } from "lucide-react"
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
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import ReactMarkdown from "react-markdown"

interface Inspiration {
  name: string
  license: string
  url: string
  description: string
}

interface Dependency {
  name: string
  license: string
  url: string
}

const inspirations: Inspiration[] = [
  {
    name: "RouteLLM",
    license: "Apache-2.0",
    url: "https://github.com/lm-sys/RouteLLM",
    description: "ML-based intelligent routing framework. LocalRouter's RouteLLM feature is a Rust reimplementation of their approach.",
  },
  {
    name: "Microsoft MCP Gateway",
    license: "MIT",
    url: "https://github.com/microsoft/mcp-gateway",
    description: "Inspiration for MCP gateway architecture and unified proxy design patterns.",
  },
]

const rustDependencies: Dependency[] = [
  { name: "Tauri", license: "MIT/Apache-2.0", url: "https://tauri.app" },
  { name: "Axum", license: "MIT", url: "https://github.com/tokio-rs/axum" },
  { name: "Tokio", license: "MIT", url: "https://tokio.rs" },
  { name: "Reqwest", license: "MIT/Apache-2.0", url: "https://github.com/seanmonstar/reqwest" },
  { name: "Serde", license: "MIT/Apache-2.0", url: "https://serde.rs" },
  { name: "Candle", license: "MIT/Apache-2.0", url: "https://github.com/huggingface/candle" },
  { name: "Tokenizers", license: "Apache-2.0", url: "https://github.com/huggingface/tokenizers" },
  { name: "Ring", license: "ISC", url: "https://github.com/briansmith/ring" },
  { name: "rusqlite", license: "MIT", url: "https://github.com/rusqlite/rusqlite" },
  { name: "utoipa", license: "MIT/Apache-2.0", url: "https://github.com/juhaku/utoipa" },
  { name: "Tower", license: "MIT", url: "https://github.com/tower-rs/tower" },
  { name: "Tracing", license: "MIT", url: "https://github.com/tokio-rs/tracing" },
  { name: "Chrono", license: "MIT/Apache-2.0", url: "https://github.com/chronotope/chrono" },
  { name: "UUID", license: "MIT/Apache-2.0", url: "https://github.com/uuid-rs/uuid" },
  { name: "OAuth2", license: "MIT/Apache-2.0", url: "https://github.com/ramosbugs/oauth2-rs" },
  { name: "Keyring", license: "MIT/Apache-2.0", url: "https://github.com/hwchen/keyring-rs" },
]

const frontendDependencies: Dependency[] = [
  { name: "React", license: "MIT", url: "https://react.dev" },
  { name: "Radix UI", license: "MIT", url: "https://radix-ui.com" },
  { name: "Tailwind CSS", license: "MIT", url: "https://tailwindcss.com" },
  { name: "Recharts", license: "MIT", url: "https://recharts.org" },
  { name: "React Flow", license: "MIT", url: "https://reactflow.dev" },
  { name: "Lucide Icons", license: "ISC", url: "https://lucide.dev" },
  { name: "Heroicons", license: "MIT", url: "https://heroicons.com" },
  { name: "cmdk", license: "MIT", url: "https://cmdk.paco.me" },
  { name: "Sonner", license: "MIT", url: "https://sonner.emilkowal.ski" },
  { name: "OpenAI SDK", license: "Apache-2.0", url: "https://github.com/openai/openai-node" },
]

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
  const [licensesExpanded, setLicensesExpanded] = useState(false)

  const handleOpenUrl = (url: string) => {
    open(url)
  }

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
      const errorMessage = err?.message || (typeof err === 'string' ? err : JSON.stringify(err)) || "Unknown error"
      setCheckError(errorMessage)
      toast.error(`Check failed: ${errorMessage}`)
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

      {/* Inspirations & Credits */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Heart className="h-4 w-4" />
            Inspirations & Credits
          </CardTitle>
          <CardDescription>
            This project was inspired by the following projects. No code was directly used, but their ideas influenced the design.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {inspirations.map((inspiration) => (
            <div
              key={inspiration.name}
              className="p-3 bg-muted/50 rounded-lg border"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-sm">{inspiration.name}</span>
                  <Badge variant="secondary" className="text-xs">
                    {inspiration.license}
                  </Badge>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleOpenUrl(inspiration.url)}
                >
                  <ExternalLink className="h-3 w-3" />
                </Button>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                {inspiration.description}
              </p>
            </div>
          ))}
        </CardContent>
      </Card>

      {/* RouteLLM Model Licenses */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Cpu className="h-4 w-4" />
            RouteLLM Model Licenses
          </CardTitle>
          <CardDescription>
            When using RouteLLM intelligent routing, the following model weights are downloaded.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="p-3 bg-amber-50 dark:bg-amber-950/30 border border-amber-200 dark:border-amber-800 rounded-lg">
            <div className="flex items-center gap-2 mb-2">
              <span className="font-medium text-sm">routellm/mf_gpt4_augmented</span>
              <Badge variant="outline" className="text-xs bg-amber-100 dark:bg-amber-900/50">
                Apache-2.0
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground">
              Matrix factorization router model trained on GPT-4 preference data. Hosted on Hugging Face.
            </p>
          </div>
        </CardContent>
      </Card>

      {/* Open Source Dependencies */}
      <Card>
        <Collapsible open={licensesExpanded} onOpenChange={setLicensesExpanded}>
          <CardHeader className="pb-3">
            <CollapsibleTrigger className="flex items-center justify-between w-full">
              <CardTitle className="text-sm flex items-center gap-2">
                <Code className="h-4 w-4" />
                Open Source Dependencies
              </CardTitle>
              {licensesExpanded ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
            </CollapsibleTrigger>
          </CardHeader>
          <CollapsibleContent>
            <CardContent className="space-y-4">
              {/* Rust Dependencies */}
              <div>
                <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">
                  Backend (Rust)
                </h4>
                <div className="grid grid-cols-2 gap-2">
                  {rustDependencies.map((dep) => (
                    <button
                      key={dep.name}
                      onClick={() => handleOpenUrl(dep.url)}
                      className="flex items-center justify-between p-2 bg-muted/50 rounded border hover:bg-muted text-left text-xs"
                    >
                      <span>{dep.name}</span>
                      <span className="text-muted-foreground">{dep.license}</span>
                    </button>
                  ))}
                </div>
              </div>

              {/* Frontend Dependencies */}
              <div>
                <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">
                  Frontend (TypeScript/React)
                </h4>
                <div className="grid grid-cols-2 gap-2">
                  {frontendDependencies.map((dep) => (
                    <button
                      key={dep.name}
                      onClick={() => handleOpenUrl(dep.url)}
                      className="flex items-center justify-between p-2 bg-muted/50 rounded border hover:bg-muted text-left text-xs"
                    >
                      <span>{dep.name}</span>
                      <span className="text-muted-foreground">{dep.license}</span>
                    </button>
                  ))}
                </div>
              </div>
            </CardContent>
          </CollapsibleContent>
        </Collapsible>
      </Card>

      {/* Footer */}
      <div className="pt-4 border-t">
        <p className="text-xs text-muted-foreground text-center">
          LocalRouter AI is open source software licensed under AGPL-3.0-or-later. View the full source code on{" "}
          <button
            onClick={() => handleOpenUrl("https://github.com/mfaro-io/localrouterai")}
            className="text-primary hover:underline"
          >
            GitHub
          </button>
          .
        </p>
      </div>
    </div>
  )
}
