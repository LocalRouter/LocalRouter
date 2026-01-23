import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FolderOpen, Save, FileText } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"

interface LoggingConfig {
  retention_days: number
  log_dir: string
}

export function LoggingTab() {
  const [config, setConfig] = useState<LoggingConfig>({
    retention_days: 31,
    log_dir: "",
  })
  const [editRetention, setEditRetention] = useState(31)
  const [isSaving, setIsSaving] = useState(false)
  const [isOpening, setIsOpening] = useState(false)
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false)

  useEffect(() => {
    loadConfig()
  }, [])

  useEffect(() => {
    setHasUnsavedChanges(editRetention !== config.retention_days)
  }, [editRetention, config.retention_days])

  const loadConfig = async () => {
    try {
      const loggingConfig = await invoke<LoggingConfig>("get_logging_config")
      setConfig(loggingConfig)
      setEditRetention(loggingConfig.retention_days)
    } catch (error) {
      console.error("Failed to load logging config:", error)
      toast.error("Failed to load logging configuration")
    }
  }

  const handleSave = async () => {
    if (editRetention < 1 || editRetention > 365) {
      toast.error("Retention must be between 1 and 365 days")
      return
    }

    setIsSaving(true)
    try {
      await invoke("update_logging_config", { retentionDays: editRetention })
      setConfig({ ...config, retention_days: editRetention })
      toast.success("Logging settings saved")
    } catch (error: any) {
      console.error("Failed to save logging config:", error)
      toast.error(`Failed to save: ${error.message || error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const handleOpenFolder = async () => {
    setIsOpening(true)
    try {
      await invoke("open_logs_folder")
    } catch (error: any) {
      console.error("Failed to open logs folder:", error)
      toast.error(`Failed to open folder: ${error.message || error}`)
    } finally {
      setIsOpening(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* Log Files Location */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <FileText className="h-4 w-4" />
            Log Files
          </CardTitle>
          <CardDescription>
            Access logs are stored in daily rotated files
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label className="text-xs text-muted-foreground">Log Directory</Label>
            <div className="flex items-center gap-2">
              <code className="flex-1 text-xs font-mono bg-muted px-3 py-2 rounded truncate">
                {config.log_dir || "Loading..."}
              </code>
              <Button
                variant="outline"
                size="sm"
                onClick={handleOpenFolder}
                disabled={isOpening || !config.log_dir}
              >
                <FolderOpen className="h-4 w-4 mr-1" />
                {isOpening ? "Opening..." : "Open"}
              </Button>
            </div>
          </div>

          <div className="p-3 bg-muted rounded-lg space-y-1">
            <p className="text-xs font-medium">File Format</p>
            <p className="text-xs text-muted-foreground">
              <code className="bg-background px-1 rounded">localrouter-YYYY-MM-DD.log</code> (LLM requests)
            </p>
            <p className="text-xs text-muted-foreground">
              <code className="bg-background px-1 rounded">localrouter-mcp-YYYY-MM-DD.log</code> (MCP requests)
            </p>
          </div>
        </CardContent>
      </Card>

      {/* Retention Settings */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Retention Settings</CardTitle>
          <CardDescription>
            Configure how long to keep access log files
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="retention">Keep logs for (days)</Label>
            <div className="flex items-center gap-2">
              <Input
                id="retention"
                type="number"
                min={1}
                max={365}
                value={editRetention}
                onChange={(e) => setEditRetention(parseInt(e.target.value) || 1)}
                className="w-24"
              />
              <span className="text-sm text-muted-foreground">days</span>
            </div>
            <p className="text-xs text-muted-foreground">
              Log files older than this will be automatically deleted during rotation (1-365 days)
            </p>
          </div>

          <div className="flex gap-2">
            <Button
              size="sm"
              onClick={handleSave}
              disabled={isSaving || !hasUnsavedChanges}
            >
              <Save className="h-4 w-4 mr-1" />
              {isSaving ? "Saving..." : "Save"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setEditRetention(config.retention_days)}
              disabled={!hasUnsavedChanges}
            >
              Reset
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Info */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">About Access Logs</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="space-y-2 text-xs text-muted-foreground">
            <p>
              Access logs record all API requests including timestamps, client names,
              providers, models, token usage, costs, and latencies.
            </p>
            <p>
              Logs are stored in JSON Lines format (one JSON object per line) for easy
              parsing and analysis with standard tools.
            </p>
            <p>
              <strong>Note:</strong> Changes to retention only affect future cleanup.
              Existing files older than the new retention period will be deleted
              on the next log rotation (midnight UTC).
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
