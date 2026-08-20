import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Power, Copy } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/Toggle"
import type {
  RequestDedupeConfig,
  SetRequestDedupeEnabledParams,
  SetStartOnBootParams,
} from "@/types/tauri-commands"

export function GeneralTab() {
  const [startOnBoot, setStartOnBoot] = useState<boolean>(true)
  const [startOnBootBusy, setStartOnBootBusy] = useState(false)
  const [dedupeEnabled, setDedupeEnabled] = useState<boolean>(true)
  const [dedupeBusy, setDedupeBusy] = useState(false)

  useEffect(() => {
    loadStartOnBoot()
    loadDedupe()
  }, [])

  const loadStartOnBoot = async () => {
    try {
      const enabled = await invoke<boolean>("get_start_on_boot")
      setStartOnBoot(enabled)
    } catch (error) {
      console.error("Failed to load start-on-boot setting:", error)
    }
  }

  const toggleStartOnBoot = async (enabled: boolean) => {
    setStartOnBootBusy(true)
    // Optimistic update; reverted on failure
    setStartOnBoot(enabled)
    try {
      await invoke("set_start_on_boot", { enabled } satisfies SetStartOnBootParams)
      toast.success(enabled ? "LocalRouter will start at login" : "Start at login disabled")
    } catch (error: any) {
      setStartOnBoot(!enabled)
      toast.error(`Failed to update start at login: ${error.message || error}`)
    } finally {
      setStartOnBootBusy(false)
    }
  }

  const loadDedupe = async () => {
    try {
      const cfg = await invoke<RequestDedupeConfig>("get_request_dedupe_config")
      setDedupeEnabled(cfg.enabled)
    } catch (error) {
      console.error("Failed to load duplicate request detection setting:", error)
    }
  }

  const toggleDedupe = async (enabled: boolean) => {
    setDedupeBusy(true)
    setDedupeEnabled(enabled)
    try {
      await invoke("set_request_dedupe_enabled", { enabled } satisfies SetRequestDedupeEnabledParams)
      toast.success(enabled ? "Duplicate request detection enabled" : "Duplicate request detection disabled")
    } catch (error: any) {
      setDedupeEnabled(!enabled)
      toast.error(`Failed to update: ${error.message || error}`)
    } finally {
      setDedupeBusy(false)
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      {/* Startup */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Power className="h-4 w-4" />
            Startup
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between gap-4">
            <div className="space-y-1">
              <Label htmlFor="start-on-boot" className="text-sm">
                Start LocalRouter at login
              </Label>
              <p className="text-xs text-muted-foreground">
                Launches LocalRouter automatically when you log in, so connected
                apps can always reach the LLM API and MCP gateway.
              </p>
            </div>
            <Switch
              id="start-on-boot"
              checked={startOnBoot}
              disabled={startOnBootBusy}
              onCheckedChange={toggleStartOnBoot}
            />
          </div>
        </CardContent>
      </Card>

      {/* Duplicate request detection */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Copy className="h-4 w-4" />
            Duplicate request detection
          </CardTitle>
          <CardDescription>
            One request can pass through LocalRouter several times — for example an
            app → a gateway client → the HTTPS inspection proxy → a reverse-proxied
            local provider.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between gap-4">
            <div className="space-y-1">
              <Label htmlFor="request-dedupe" className="text-sm">
                Detect and pass through repeated hops
              </Label>
              <p className="text-xs text-muted-foreground">
                Every forwarded request is tagged with an{" "}
                <code className="font-mono">X-LocalRouter-Trace</code> header. A later
                LocalRouter hop that sees it forwards the request unchanged (no
                guardrails, compression, JSON repair or approval prompts) and still
                logs it in the Monitor — flagged as a duplicate and excluded from
                usage stats and rate limits so nothing is counted twice.
              </p>
            </div>
            <Switch
              id="request-dedupe"
              checked={dedupeEnabled}
              disabled={dedupeBusy}
              onCheckedChange={toggleDedupe}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
