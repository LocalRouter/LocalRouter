import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Checkbox } from "@/components/ui/checkbox"
import type { SetPeriodicHealthEnabledParams } from "@/types/tauri-commands"

export function HealthChecksTab() {
  const [periodicHealthEnabled, setPeriodicHealthEnabled] = useState(false)

  useEffect(() => {
    loadPeriodicHealthEnabled()
  }, [])

  const loadPeriodicHealthEnabled = async () => {
    try {
      const enabled = await invoke<boolean>("get_periodic_health_enabled")
      setPeriodicHealthEnabled(enabled)
    } catch (error) {
      console.error("Failed to load periodic health setting:", error)
    }
  }

  const togglePeriodicHealth = async (checked: boolean) => {
    try {
      await invoke("set_periodic_health_enabled", { enabled: checked } satisfies SetPeriodicHealthEnabledParams)
      setPeriodicHealthEnabled(checked)
      toast.success(checked ? "Periodic health checks enabled" : "Periodic health checks disabled")
    } catch (error: any) {
      toast.error(`Failed to update: ${error.message || error}`)
    }
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">Health Checks</CardTitle>
          <CardDescription>
            Configure automatic health monitoring for LLM providers and MCP servers
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center space-x-2">
            <Checkbox
              id="periodic-health"
              checked={periodicHealthEnabled}
              onCheckedChange={(checked) => togglePeriodicHealth(checked === true)}
            />
            <Label htmlFor="periodic-health" className="text-sm font-normal cursor-pointer">
              Enable periodic health checks
            </Label>
          </div>
          <p className="text-xs text-muted-foreground">
            When enabled, provider and MCP server health is checked automatically on a schedule.
          </p>
          <p className="text-xs text-amber-600 dark:text-amber-400">
            Note: Health checks make API calls to each provider. Some providers count these
            against free-tier rate limits, which may exhaust your quota over time.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
