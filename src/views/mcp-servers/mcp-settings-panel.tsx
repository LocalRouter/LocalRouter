import { useEffect, useState, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"
import type { McpGatewaySettingsResponse, SetMcpGatewaySettingsParams } from "@/types/tauri-commands"

interface ButtonGroupOption {
  value: string
  label: string
  activeClass: string
}

function ButtonGroup({
  value,
  options,
  onChange,
}: {
  value: string
  options: ButtonGroupOption[]
  onChange: (v: string) => void
}) {
  return (
    <div className="inline-flex rounded-md border border-border bg-muted/50">
      {options.map((opt, i) => (
        <button
          key={opt.value}
          type="button"
          onClick={() => onChange(opt.value)}
          className={cn(
            "px-3 py-1.5 text-xs font-medium transition-colors",
            value === opt.value
              ? opt.activeClass
              : "text-muted-foreground hover:text-foreground hover:bg-muted",
            i === 0 && "rounded-l-md",
            i === options.length - 1 && "rounded-r-md"
          )}
        >
          {opt.label}
        </button>
      ))}
    </div>
  )
}

const SAMPLING_OPTIONS: ButtonGroupOption[] = [
  { value: "passthrough", label: "Passthrough", activeClass: "bg-emerald-500 text-white" },
  { value: "direct_allow", label: "Direct", activeClass: "bg-emerald-500 text-white" },
  { value: "direct_ask", label: "Direct (Ask)", activeClass: "bg-amber-500 text-white" },
  { value: "off", label: "Off", activeClass: "bg-zinc-500 text-white" },
]

const ELICITATION_OPTIONS: ButtonGroupOption[] = [
  { value: "passthrough", label: "Passthrough", activeClass: "bg-emerald-500 text-white" },
  { value: "direct", label: "Direct", activeClass: "bg-emerald-500 text-white" },
  { value: "off", label: "Off", activeClass: "bg-zinc-500 text-white" },
]

const SAMPLING_DESCRIPTIONS: Record<string, string> = {
  passthrough: "Forward to the connected external MCP client without popup.",
  direct_allow: "Route through LocalRouter's LLM providers automatically.",
  direct_ask: "Show approval popup before routing through the LLM provider.",
  off: "Reject all sampling requests from MCP servers.",
}

const ELICITATION_DESCRIPTIONS: Record<string, string> = {
  passthrough: "Forward input requests to the connected external MCP client.",
  direct: "Show a form popup to collect user input directly.",
  off: "Disable elicitation. All input requests are declined.",
}

export function McpSettingsPanel() {
  const [settings, setSettings] = useState<McpGatewaySettingsResponse | null>(null)
  const [loading, setLoading] = useState(true)

  const loadSettings = useCallback(async () => {
    try {
      const result = await invoke<McpGatewaySettingsResponse>("get_mcp_gateway_settings")
      setSettings(result)
    } catch (e) {
      console.error("Failed to load MCP gateway settings:", e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadSettings()
  }, [loadSettings])

  const updateSettings = async (params: SetMcpGatewaySettingsParams) => {
    try {
      await invoke("set_mcp_gateway_settings", { ...params })
      toast.success("Settings updated")
      loadSettings()
    } catch (e) {
      console.error("Failed to update settings:", e)
      toast.error("Failed to update settings")
    }
  }

  if (loading || !settings) {
    return <div className="p-4 text-sm text-muted-foreground">Loading settings...</div>
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <Card>
        <CardHeader>
          <CardTitle>Sampling</CardTitle>
          <CardDescription>
            How backend MCP servers request LLM completions
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2">
            <Label>Behavior</Label>
            <div>
              <ButtonGroup
                value={settings.sampling}
                options={SAMPLING_OPTIONS}
                onChange={(v) => updateSettings({ sampling: v })}
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {SAMPLING_DESCRIPTIONS[settings.sampling] || ""}
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Elicitation</CardTitle>
          <CardDescription>
            How backend MCP servers request user input
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2">
            <Label>Mode</Label>
            <div>
              <ButtonGroup
                value={settings.elicitation_mode}
                options={ELICITATION_OPTIONS}
                onChange={(v) => updateSettings({ elicitationMode: v })}
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {ELICITATION_DESCRIPTIONS[settings.elicitation_mode] || ""}
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
