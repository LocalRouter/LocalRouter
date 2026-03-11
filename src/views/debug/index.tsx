import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/Button"
import { ClientCreationWizard } from "@/components/wizard/ClientCreationWizard"
import type { DebugSetTrayOverlayParams, DiscoverProviderResult } from "@/types/tauri-commands"

interface DebugViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

type FirewallPopupType = "mcp_tool" | "llm_model" | "skill" | "marketplace" | "free_tier_fallback" | "coding_agent" | "guardrail"

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
  const [triggeringSampling, setTriggeringSampling] = useState(false)
  const [triggeringElicitation, setTriggeringElicitation] = useState(false)
  const [activeTrayOverlay, setActiveTrayOverlay] = useState<TrayOverlayOption | "auto">("auto")
  const [discovering, setDiscovering] = useState(false)
  const [discoveryResult, setDiscoveryResult] = useState<DiscoverProviderResult | null>(null)

  const handleDiscoverProviders = async () => {
    setDiscovering(true)
    setDiscoveryResult(null)
    try {
      const result = await invoke<DiscoverProviderResult>("debug_discover_providers")
      setDiscoveryResult(result)
    } catch (error) {
      console.error("Failed to discover providers:", error)
    } finally {
      setDiscovering(false)
    }
  }

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

  const handleTriggerSamplingApproval = async () => {
    setTriggeringSampling(true)
    try {
      await invoke("debug_trigger_sampling_approval_popup")
    } catch (error) {
      console.error("Failed to trigger sampling approval popup:", error)
    } finally {
      setTriggeringSampling(false)
    }
  }

  const handleTriggerElicitationForm = async () => {
    setTriggeringElicitation(true)
    try {
      await invoke("debug_trigger_elicitation_form_popup")
    } catch (error) {
      console.error("Failed to trigger elicitation form popup:", error)
    } finally {
      setTriggeringElicitation(false)
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

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("coding_agent")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "coding_agent" ? "Opening..." : "Coding Agent"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={() => handleTriggerFirewall("guardrail")}
                disabled={triggeringFirewall !== null}
              >
                {triggeringFirewall === "guardrail" ? "Opening..." : "Guardrail"}
              </Button>
            </div>
          </div>

          {/* Sampling & Elicitation Popups */}
          <div className="border rounded-lg p-4 space-y-3">
            <h2 className="text-sm font-medium">Sampling & Elicitation Popups</h2>
            <p className="text-xs text-muted-foreground">
              Test MCP sampling approval and elicitation form popups.
            </p>

            <div className="grid grid-cols-2 gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={handleTriggerSamplingApproval}
                disabled={triggeringSampling || triggeringElicitation}
              >
                {triggeringSampling ? "Opening..." : "Sampling Approval"}
              </Button>

              <Button
                size="sm"
                variant="outline"
                onClick={handleTriggerElicitationForm}
                disabled={triggeringSampling || triggeringElicitation}
              >
                {triggeringElicitation ? "Opening..." : "Elicitation Form"}
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

          {/* Local Provider Discovery */}
          <div className="border rounded-lg p-4 space-y-3">
            <h2 className="text-sm font-medium">Local Provider Discovery</h2>
            <p className="text-xs text-muted-foreground">
              Scan for local LLM providers (Ollama, LM Studio, Jan, GPT4All) and
              auto-configure any that are found running.
            </p>
            <Button
              size="sm"
              onClick={handleDiscoverProviders}
              disabled={discovering}
            >
              {discovering ? "Scanning..." : "Discover Providers"}
            </Button>
            {discoveryResult && (
              <div className="text-xs space-y-1 pt-1">
                {discoveryResult.discovered.length === 0 ? (
                  <p className="text-muted-foreground">No local providers detected.</p>
                ) : (
                  <>
                    <p>Found {discoveryResult.discovered.length} provider{discoveryResult.discovered.length !== 1 ? "s" : ""}:</p>
                    {discoveryResult.added.map((name) => (
                      <p key={name} className="text-green-600 dark:text-green-400">+ Added: {name}</p>
                    ))}
                    {discoveryResult.skipped.map((name) => (
                      <p key={name} className="text-muted-foreground">~ Already configured: {name}</p>
                    ))}
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      <ClientCreationWizard
        open={showWizard}
        onOpenChange={setShowWizard}
        onComplete={handleWizardComplete}
        showWelcome={true}
      />
    </div>
  )
}
