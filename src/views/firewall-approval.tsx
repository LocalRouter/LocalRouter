import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { Shield, Clock, AlertTriangle } from "lucide-react"
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
  const [remainingSeconds, setRemainingSeconds] = useState<number | null>(null)

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
        setRemainingSeconds(Math.max(0, result.timeout_seconds - result.created_at_secs_ago))
      } catch (err) {
        console.error("Failed to load approval details:", err)
        setError(typeof err === "string" ? err : "Failed to load approval details")
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  // Countdown timer
  useEffect(() => {
    if (remainingSeconds === null || remainingSeconds <= 0) return

    const interval = setInterval(() => {
      setRemainingSeconds((prev) => {
        if (prev === null || prev <= 1) {
          clearInterval(interval)
          // Auto-close on timeout
          getCurrentWebviewWindow().close()
          return 0
        }
        return prev - 1
      })
    }, 1000)

    return () => clearInterval(interval)
  }, [remainingSeconds !== null])

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

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen bg-background">
        <div className="text-muted-foreground text-sm">Loading...</div>
      </div>
    )
  }

  if (error || !details) {
    return (
      <div className="flex flex-col items-center justify-center h-screen bg-background p-4 gap-3">
        <AlertTriangle className="h-8 w-8 text-destructive" />
        <p className="text-sm text-destructive text-center">{error || "Request not found"}</p>
        <Button
          size="sm"
          variant="outline"
          onClick={() => getCurrentWebviewWindow().close()}
        >
          Close
        </Button>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-screen bg-background p-4 gap-3">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Shield className="h-5 w-5 text-amber-500" />
        <h1 className="text-sm font-semibold">Tool Approval Required</h1>
        {remainingSeconds !== null && remainingSeconds > 0 && (
          <span className="ml-auto flex items-center gap-1 text-xs text-muted-foreground">
            <Clock className="h-3 w-3" />
            {remainingSeconds}s
          </span>
        )}
      </div>

      {/* Details */}
      <div className="space-y-2 flex-1 overflow-auto">
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

      {/* Timeout progress bar */}
      {remainingSeconds !== null && details.timeout_seconds > 0 && (
        <div className="h-1 bg-muted rounded-full overflow-hidden">
          <div
            className="h-full bg-amber-500 transition-all duration-1000 ease-linear"
            style={{
              width: `${(remainingSeconds / details.timeout_seconds) * 100}%`,
            }}
          />
        </div>
      )}

      {/* Action Buttons */}
      <div className="flex gap-2">
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
