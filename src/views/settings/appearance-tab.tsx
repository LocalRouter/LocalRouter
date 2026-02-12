import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Monitor } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type { TrayGraphSettings } from "@/types/tauri-commands"

export function AppearanceTab() {
  const [settings, setSettings] = useState<TrayGraphSettings>({
    enabled: false,
    refresh_rate_secs: 10,
  })
  const [isSaving, setIsSaving] = useState(false)

  useEffect(() => {
    loadSettings()
  }, [])

  const loadSettings = async () => {
    try {
      const result = await invoke<TrayGraphSettings>("get_tray_graph_settings")
      setSettings(result)
    } catch (error) {
      console.error("Failed to load tray graph settings:", error)
    }
  }

  const saveSettings = async () => {
    setIsSaving(true)
    try {
      await invoke("update_tray_graph_settings", {
        enabled: settings.enabled,
        refreshRateSecs: settings.refresh_rate_secs,
      })
      toast.success("Appearance settings saved")
    } catch (error: any) {
      console.error("Failed to save appearance settings:", error)
      toast.error(`Failed to save: ${error.message || error}`)
    } finally {
      setIsSaving(false)
    }
  }

  const calculateTimeWindow = (refreshRateSecs: number): string => {
    const totalSecs = 30 * refreshRateSecs
    if (totalSecs < 60) {
      return `${totalSecs} seconds`
    }
    const mins = Math.floor(totalSecs / 60)
    const secs = totalSecs % 60
    return secs > 0 ? `${mins}m ${secs}s` : `${mins} minute${mins > 1 ? "s" : ""}`
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Monitor className="h-4 w-4" />
            Tray Icon
          </CardTitle>
          <CardDescription>
            Choose how the system tray icon displays activity
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-3">
            <Label>Icon Mode</Label>
            <RadioGroup
              value={settings.enabled ? "graph" : "static"}
              onValueChange={(value) =>
                setSettings({ ...settings, enabled: value === "graph" })
              }
            >
              <div className="flex items-start space-x-3">
                <RadioGroupItem value="static" id="mode-static" className="mt-0.5" />
                <div>
                  <Label htmlFor="mode-static" className="font-medium cursor-pointer">Static Icon</Label>
                  <p className="text-xs text-muted-foreground">
                    Clean icon with notification overlays for approvals, health issues, and updates
                  </p>
                </div>
              </div>
              <div className="flex items-start space-x-3">
                <RadioGroupItem value="graph" id="mode-graph" className="mt-0.5" />
                <div>
                  <Label htmlFor="mode-graph" className="font-medium cursor-pointer">Activity Graph</Label>
                  <p className="text-xs text-muted-foreground">
                    Live token usage sparkline that updates in real-time as requests flow through
                  </p>
                </div>
              </div>
            </RadioGroup>
          </div>

          {settings.enabled && (
            <div className="space-y-2 pl-6 border-l-2 border-muted">
              <Label>Graph Refresh Rate</Label>
              <Select
                value={settings.refresh_rate_secs.toString()}
                onValueChange={(value) =>
                  setSettings({ ...settings, refresh_rate_secs: parseInt(value) })
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="1">Fast (1s refresh, 30s window)</SelectItem>
                  <SelectItem value="10">Medium (10s refresh, 5m window)</SelectItem>
                  <SelectItem value="60">Slow (60s refresh, 30m window)</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                Window: {calculateTimeWindow(settings.refresh_rate_secs)}
              </p>
            </div>
          )}

          <Button
            size="sm"
            onClick={saveSettings}
            disabled={isSaving}
          >
            {isSaving ? "Saving..." : "Save"}
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
