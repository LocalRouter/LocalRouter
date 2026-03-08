import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/Button"
import { ClientCreationWizard } from "@/components/wizard/ClientCreationWizard"
import type { DebugSetTrayOverlayParams } from "@/types/tauri-commands"

interface DebugViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

type FirewallPopupType = "mcp_tool" | "llm_model" | "skill" | "marketplace" | "free_tier_fallback"

type TrayOverlayOption = NonNullable<DebugSetTrayOverlayParams["overlay"]>

const TRAY_OVERLAY_OPTIONS: { value: TrayOverlayOption | "auto"; label: string }[] = [
  { value: "auto", label: "Auto (normal)" },
  { value: "none", label: "None" },
  { value: "warning_yellow", label: "Warning (yellow)" },
  { value: "warning_red", label: "Warning (red)" },
  { value: "update_available", label: "Update Available" },
  { value: "firewall_pending", label: "Firewall Pending" },
]

export function DebugView({ activeSubTab: _activeSubTab, onTabChange }: DebugViewProps) {
  const [showWizard, setShowWizard] = useState(false)
  const [triggeringFirewall, setTriggeringFirewall] = useState<FirewallPopupType | null>(null)
  const [sendMultiple, setSendMultiple] = useState(false)
  const [activeTrayOverlay, setActiveTrayOverlay] = useState<TrayOverlayOption | "auto">("auto")

  const handleTriggerFirewall = async (popupType: FirewallPopupType) => {
    setTriggeringFirewall(popupType)
    try {
      await invoke("debug_trigger_firewall_popup", { popupType, sendMultiple })
    } catch (error) {
      console.error("Failed to trigger firewall popup:", error)
    } finally {
      setTriggeringFirewall(null)
    }
  }

  const handleSetTrayOverlay = async (value: TrayOverlayOption | "auto") => {
    setActiveTrayOverlay(value)
    try {
      const overlay = value === "auto" ? null : value
      await invoke("debug_set_tray_overlay", { overlay } satisfies DebugSetTrayOverlayParams)
    } catch (error) {
      console.error("Failed to set tray overlay:", error)
    }
  }

  const handleWizardComplete = async (clientId: string) => {
    setShowWizard(false)
    onTabChange("clients", `${clientId}/config`)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight">Debug</h1>
        <p className="text-sm text-muted-foreground">
          Development-only tools for testing UI flows
        </p>
      </div>

      <div className="flex-1 overflow-auto">
        <div className="space-y-4 max-w-2xl">
          {/* First-time wizard */}
          <div className="border rounded-lg p-4 space-y-2">
            <h2 className="text-sm font-medium">First-Time Setup Wizard</h2>
            <p className="text-xs text-muted-foreground">
              Opens the client creation wizard that appears on first launch.
            </p>
            <Button size="sm" onClick={() => setShowWizard(true)}>
              Open Wizard
            </Button>
          </div>

          {/* Firewall popups */}
          <div className="border rounded-lg p-4 space-y-3">
            <h2 className="text-sm font-medium">Firewall Approval Popups</h2>
            <p className="text-xs text-muted-foreground">
              Test different types of firewall approval popups. Each button creates a fake
              approval request and opens the popup immediately.
            </p>

            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={sendMultiple}
                onChange={(e) => setSendMultiple(e.target.checked)}
              />
              Send multiple (3 popups: 2 same resource + 1 different)
            </label>

            <div className="grid grid-cols-2 gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("mcp_tool")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "mcp_tool" ? "Opening..." : "MCP Tool"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("llm_model")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "llm_model" ? "Opening..." : "LLM Model"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("skill")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "skill" ? "Opening..." : "Skill"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("marketplace")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "marketplace" ? "Opening..." : "Marketplace"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("free_tier_fallback")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "free_tier_fallback" ? "Opening..." : "Free-Tier Fallback"}
              </Button>
            </div>
          </div>

          {/* Tray icon overlay */}
          <div className="border rounded-lg p-4 space-y-3">
            <h2 className="text-sm font-medium">Tray Icon Overlay</h2>
            <p className="text-xs text-muted-foreground">
              Force a specific tray icon overlay state. "Auto" returns to normal
              priority-based behavior.
            </p>
            <div className="grid grid-cols-3 gap-2">
              {TRAY_OVERLAY_OPTIONS.map((opt) => (
                <Button
                  key={opt.value}
                  size="sm"
                  variant={activeTrayOverlay === opt.value ? "default" : "outline"}
                  onClick={() => handleSetTrayOverlay(opt.value)}
                >
                  {opt.label}
                </Button>
              ))}
            </div>
          </div>
        </div>
      </div>

      <ClientCreationWizard
        open={showWizard}
        onOpenChange={setShowWizard}
        onComplete={handleWizardComplete}
      />
    </div>
  )
}
