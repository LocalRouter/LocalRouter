import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { FlaskConical } from "lucide-react"
import { Button } from "@/components/ui/Button"

type FirewallPopupType = "mcp_tool" | "llm_model" | "skill" | "marketplace" | "free_tier_fallback" | "coding_agent" | "guardrail" | "secret_scan"

interface SamplePopupButtonProps {
  popupType: FirewallPopupType
}

export function SamplePopupButton({ popupType }: SamplePopupButtonProps) {
  const [triggering, setTriggering] = useState(false)

  const handleClick = async () => {
    setTriggering(true)
    try {
      await invoke("debug_trigger_firewall_popup", { popupType, sendMultiple: false })
    } catch (error) {
      console.error("Failed to trigger firewall popup:", error)
    } finally {
      setTriggering(false)
    }
  }

  return (
    <Button variant="outline" size="sm" onClick={handleClick} disabled={triggering}>
      <FlaskConical className="h-3.5 w-3.5 mr-1.5" />
      {triggering ? "Opening..." : "Sample Popup"}
    </Button>
  )
}
