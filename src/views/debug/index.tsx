import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/Button"
import { ClientCreationWizard } from "@/components/wizard/ClientCreationWizard"

interface DebugViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function DebugView({ activeSubTab: _activeSubTab, onTabChange }: DebugViewProps) {
  const [showWizard, setShowWizard] = useState(false)
  const [firewallTriggered, setFirewallTriggered] = useState(false)

  const handleTriggerFirewall = async () => {
    setFirewallTriggered(true)
    try {
      await invoke("debug_trigger_firewall_popup")
    } catch (error) {
      console.error("Failed to trigger firewall popup:", error)
    } finally {
      setFirewallTriggered(false)
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

          {/* Firewall popup */}
          <div className="border rounded-lg p-4 space-y-2">
            <h2 className="text-sm font-medium">Firewall Approval Popup</h2>
            <p className="text-xs text-muted-foreground">
              Creates a fake firewall approval request on the backend and opens the popup window after 3 seconds.
              You can close this window before the popup appears.
            </p>
            <Button
              size="sm"
              onClick={handleTriggerFirewall}
              disabled={firewallTriggered}
            >
              {firewallTriggered ? "Triggering in 3s..." : "Trigger Firewall Popup"}
            </Button>
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
