import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { Shield, AlertTriangle, X } from "lucide-react"
import { Button } from "@/components/ui/Button"

interface ApprovalDetails {
  request_id: string
  client_id: string
  client_name: string
  tool_name: string
  server_name: string
  arguments_preview: string
  timeout_seconds: number
  created_at_secs_ago: number
}

type ApprovalAction = "deny" | "allow_once" | "allow_session"

export function FirewallApproval() {
  const [details, setDetails] = useState<ApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [showArgs, setShowArgs] = useState(false)

  useEffect(() => {
    const loadDetails = async () => {
      try {
        // Extract request_id from window label: "firewall-approval-{request_id}"
        const window = getCurrentWebviewWindow()
        const label = window.label
        const requestId = label.replace("firewall-approval-", "")

        const result = await invoke<ApprovalDetails>("get_firewall_approval_details", {
          requestId,
        })
        setDetails(result)
      } catch (err) {
        console.error("Failed to load approval details:", err)
        setError(typeof err === "string" ? err : "Failed to load approval details")
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  const handleAction = async (action: ApprovalAction) => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_firewall_approval", {
        requestId: details.request_id,
        action,
      })
      // Close the popup window
      await getCurrentWebviewWindow().close()
    } catch (err) {
      console.error("Failed to submit approval:", err)
      setError(typeof err === "string" ? err : "Failed to submit response")
      setSubmitting(false)
    }
  }

  const handleClose = () => {
    getCurrentWebviewWindow().close()
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen bg-background rounded-lg border border-border">
        <div className="text-muted-foreground text-sm">Loading...</div>
      </div>
    )
  }

  if (error || !details) {
    return (
      <div className="flex flex-col h-screen bg-background rounded-lg border border-border">
        {/* Draggable header with close button */}
        <div
          data-tauri-drag-region
          className="flex items-center justify-between p-3 border-b border-border cursor-move"
        >
          <div className="flex items-center gap-2" data-tauri-drag-region>
            <AlertTriangle className="h-5 w-5 text-destructive" />
            <h1 className="text-sm font-semibold">Error</h1>
          </div>
          <button
            type="button"
            onClick={handleClose}
            className="p-1 rounded hover:bg-muted transition-colors"
            title="Close"
          >
            <X className="h-4 w-4 text-muted-foreground" />
          </button>
        </div>
        <div className="flex flex-col items-center justify-center flex-1 p-4 gap-3">
          <p className="text-sm text-destructive text-center">{error || "Request not found"}</p>
          <Button size="sm" variant="outline" onClick={handleClose}>
            Close
          </Button>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-screen bg-background rounded-lg border border-border">
      {/* Draggable header with close button */}
      <div
        data-tauri-drag-region
        className="flex items-center justify-between p-3 border-b border-border cursor-move"
      >
        <div className="flex items-center gap-2" data-tauri-drag-region>
          <Shield className="h-5 w-5 text-amber-500" />
          <h1 className="text-sm font-semibold">Tool Approval Required</h1>
        </div>
        <button
          type="button"
          onClick={handleClose}
          className="p-1 rounded hover:bg-muted transition-colors"
          title="Close"
        >
          <X className="h-4 w-4 text-muted-foreground" />
        </button>
      </div>

      {/* Details */}
      <div className="space-y-2 flex-1 overflow-auto p-4">
        <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
          <span className="text-muted-foreground">Client:</span>
          <span className="font-medium truncate">{details.client_name}</span>

          <span className="text-muted-foreground">Tool:</span>
          <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
            {details.tool_name}
          </code>

          <span className="text-muted-foreground">Server:</span>
          <span className="truncate">{details.server_name}</span>
        </div>

        {/* Arguments Preview */}
        {details.arguments_preview && (
          <div>
            <button
              type="button"
              onClick={() => setShowArgs(!showArgs)}
              className="text-xs text-muted-foreground hover:text-foreground transition-colors"
            >
              {showArgs ? "Hide" : "Show"} arguments
            </button>
            {showArgs && (
              <pre className="mt-1 text-xs bg-muted p-2 rounded overflow-auto max-h-24 font-mono">
                {details.arguments_preview}
              </pre>
            )}
          </div>
        )}
      </div>

      {/* Action Buttons */}
      <div className="flex gap-2 p-4 pt-0">
        <Button
          variant="destructive"
          size="sm"
          className="flex-1"
          onClick={() => handleAction("deny")}
          disabled={submitting}
        >
          Deny
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="flex-1"
          onClick={() => handleAction("allow_once")}
          disabled={submitting}
        >
          Allow Once
        </Button>
        <Button
          size="sm"
          className="flex-1 bg-emerald-600 hover:bg-emerald-700 text-white"
          onClick={() => handleAction("allow_session")}
          disabled={submitting}
        >
          Allow Session
        </Button>
      </div>
    </div>
  )
}
