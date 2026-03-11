import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { Button } from "@/components/ui/Button"

interface SamplingApprovalDetails {
  request_id: string
  server_id: string
  message_count: number
  system_prompt: string | null
  model_preferences: any | null
  max_tokens: number | null
  timeout_seconds: number
  created_at_secs_ago: number
}

export function SamplingApproval() {
  const [details, setDetails] = useState<SamplingApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const label = (window as any).__TAURI_INTERNALS__?.metadata?.currentWebview?.label || ""
    const requestId = label.replace("sampling-approval-", "")

    if (!requestId) {
      setError("Missing request ID")
      setLoading(false)
      return
    }

    invoke<SamplingApprovalDetails>("get_sampling_approval_details", { requestId })
      .then(setDetails)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false))
  }, [])

  const handleAction = async (action: "allow" | "deny") => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_sampling_approval", {
        requestId: details.request_id,
        action,
      })
      const window = getCurrentWebviewWindow()
      await window.close()
    } catch (e) {
      setError(String(e))
      setSubmitting(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen bg-background text-foreground p-4">
        <p className="text-sm text-muted-foreground">Loading...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-screen bg-background text-foreground p-4">
        <p className="text-sm text-red-500">{error}</p>
      </div>
    )
  }

  if (!details) return null

  return (
    <div className="h-screen bg-background text-foreground p-4 flex flex-col">
      <div className="flex-1 space-y-3">
        <div>
          <h2 className="text-base font-semibold">Sampling Request</h2>
          <p className="text-xs text-muted-foreground mt-1">
            Server <span className="font-medium text-foreground">{details.server_id}</span> is requesting an LLM completion
          </p>
        </div>

        <div className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Messages</span>
            <span>{details.message_count}</span>
          </div>
          {details.system_prompt && (
            <div>
              <span className="text-muted-foreground text-xs">System prompt</span>
              <p className="text-xs mt-0.5 bg-muted/50 rounded p-2 max-h-20 overflow-auto">
                {details.system_prompt.length > 200
                  ? details.system_prompt.slice(0, 200) + "..."
                  : details.system_prompt}
              </p>
            </div>
          )}
          {details.max_tokens && (
            <div className="flex justify-between">
              <span className="text-muted-foreground">Max tokens</span>
              <span>{details.max_tokens}</span>
            </div>
          )}
        </div>
      </div>

      <div className="flex gap-2 pt-3 border-t">
        <Button
          variant="outline"
          className="flex-1"
          onClick={() => handleAction("deny")}
          disabled={submitting}
        >
          Deny
        </Button>
        <Button
          className="flex-1"
          onClick={() => handleAction("allow")}
          disabled={submitting}
        >
          Allow
        </Button>
      </div>
    </div>
  )
}
