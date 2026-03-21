import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Monitor } from "lucide-react"
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

/** Static tray icon preview — LocalRouter logo in a rounded frame */
function StaticIconPreview() {
  return (
    <svg width="36" height="36" viewBox="0 0 32 32" fill="none" className="shrink-0">
      {/* Rounded border frame */}
      <rect x="0.5" y="0.5" width="31" height="31" rx="5.5" stroke="currentColor" strokeOpacity={0.5} fill="none" />
      {/* LocalRouter logo scaled to fit */}
      <g transform="translate(4, 4) scale(0.24)">
        <circle cx="20" cy="20" r="12" stroke="currentColor" strokeWidth="10" fill="none" />
        <circle cx="80" cy="80" r="12" stroke="currentColor" strokeWidth="10" fill="none" />
        <path
          d="M 32 22 C 75 15, 90 40, 50 50 C 10 60, 25 85, 68 78"
          stroke="currentColor"
          strokeWidth="10"
          strokeLinecap="round"
          fill="none"
        />
      </g>
    </svg>
  )
}

/** Animated activity graph preview — scrolling sparkline bars */
function GraphIconPreview() {
  const BAR_COUNT = 20
  const [bars, setBars] = useState<number[]>(() =>
    Array.from({ length: BAR_COUNT }, () => Math.random() * 0.8 + 0.1)
  )

  const tick = useCallback(() => {
    setBars(prev => {
      const next = prev.slice(1)
      // Generate next bar influenced by the previous value for smoother movement
      const last = prev[prev.length - 1]
      const delta = (Math.random() - 0.5) * 0.35
      const newVal = Math.max(0.05, Math.min(1, last + delta))
      next.push(newVal)
      return next
    })
  }, [])

  useEffect(() => {
    const id = setInterval(tick, 180)
    return () => clearInterval(id)
  }, [tick])

  const padding = 3
  const barAreaWidth = 36 - padding * 2
  const barAreaHeight = 32 - padding * 2
  const barWidth = barAreaWidth / BAR_COUNT

  return (
    <svg width="36" height="36" viewBox="0 0 36 36" fill="none" className="shrink-0">
      {/* Rounded border frame */}
      <rect x="0.5" y="0.5" width="35" height="35" rx="5.5" stroke="currentColor" strokeOpacity={0.5} fill="none" />
      {/* Animated bars */}
      {bars.map((h, i) => {
        const barH = h * barAreaHeight
        return (
          <rect
            key={i}
            x={padding + i * barWidth}
            y={padding + barAreaHeight - barH}
            width={Math.max(barWidth - 0.5, 0.5)}
            height={barH}
            fill="currentColor"
            opacity={0.7}
          />
        )
      })}
    </svg>
  )
}

export function AppearanceTab() {
  const [settings, setSettings] = useState<TrayGraphSettings>({
    enabled: false,
    refresh_rate_secs: 10,
  })
  const loaded = useRef(false)

  useEffect(() => {
    loadSettings()
  }, [])

  // Auto-save whenever settings change (skip initial load)
  useEffect(() => {
    if (!loaded.current) return
    saveSettings(settings)
  }, [settings.enabled, settings.refresh_rate_secs])

  const loadSettings = async () => {
    try {
      const result = await invoke<TrayGraphSettings>("get_tray_graph_settings")
      setSettings(result)
      // Mark as loaded after state is set so the effect doesn't fire for initial load
      setTimeout(() => { loaded.current = true }, 0)
    } catch (error) {
      console.error("Failed to load tray graph settings:", error)
    }
  }

  const saveSettings = async (s: TrayGraphSettings) => {
    try {
      await invoke("update_tray_graph_settings", {
        enabled: s.enabled,
        refreshRateSecs: s.refresh_rate_secs,
      })
    } catch (error: any) {
      console.error("Failed to save appearance settings:", error)
      toast.error(`Failed to save: ${error.message || error}`)
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
    <div className="space-y-6 max-w-2xl">
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
                <div className="flex items-start gap-3">
                  <StaticIconPreview />
                  <div>
                    <Label htmlFor="mode-static" className="font-medium cursor-pointer">Static Icon</Label>
                    <p className="text-xs text-muted-foreground">
                      Clean icon with notification overlays for approvals, health issues, and updates
                    </p>
                  </div>
                </div>
              </div>
              <div className="flex items-start space-x-3">
                <RadioGroupItem value="graph" id="mode-graph" className="mt-0.5" />
                <div className="flex items-start gap-3">
                  <GraphIconPreview />
                  <div>
                    <Label htmlFor="mode-graph" className="font-medium cursor-pointer">Activity Graph</Label>
                    <p className="text-xs text-muted-foreground">
                      Live token usage sparkline that updates in real-time as requests flow through
                    </p>
                  </div>
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
        </CardContent>
      </Card>
    </div>
  )
}
