import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { LogicalSize } from "@tauri-apps/api/dpi"
import { Button } from "@/components/ui/Button"
import { ProvidersIcon } from "@/components/icons/category-icons"

interface SamplingMessagePreview {
  role: string
  content: string
}

interface SamplingApprovalDetails {
  request_id: string
  server_id: string
  message_count: number
  system_prompt: string | null
  messages_preview: SamplingMessagePreview[]
  model_preferences: unknown | null
  max_tokens: number | null
  timeout_seconds: number
  created_at_secs_ago: number
}

export function SamplingApproval() {
  const [details, setDetails] = useState<SamplingApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [buttonsReady, setButtonsReady] = useState(false)

  useEffect(() => {
    const loadDetails = async () => {
      try {
        const window = getCurrentWebviewWindow()
        const label = window.label
        const requestId = label.replace("sampling-approval-", "")

        const result = await invoke<SamplingApprovalDetails>("get_sampling_approval_details", {
          requestId,
        })
        setDetails(result)

        // Resize and show window
        const win = getCurrentWebviewWindow()
        await win.setSize(new LogicalSize(400, 320))
        await win.center()
        await win.show()
        await win.setFocus()
      } catch (err) {
        console.error("Failed to load sampling approval details:", err)
        setError(typeof err === "string" ? err : "Failed to load approval details")
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  // Delay buttons until window is focused (prevent accidental clicks)
  useEffect(() => {
    if (loading || !details || buttonsReady) return
    let timer: ReturnType<typeof setTimeout> | null = null

    const startTimer = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => setButtonsReady(true), 500)
    }

    const win = getCurrentWebviewWindow()
    win.isFocused().then((focused) => {
      if (focused) startTimer()
    })

    const unlistenPromise = win.onFocusChanged(({ payload: focused }) => {
      if (focused) startTimer()
    })

    return () => {
      if (timer) clearTimeout(timer)
      unlistenPromise.then(fn => { try { fn() } catch {} }).catch(() => {})
    }
  }, [loading, details, buttonsReady])

  const handleAction = async (action: "allow" | "deny") => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_sampling_approval", {
        requestId: details.request_id,
        action,
      })
      await getCurrentWebviewWindow().close()
    } catch (err) {
      console.error("Failed to submit sampling approval:", err)
      setError(typeof err === "string" ? err : "Failed to submit response")
      setSubmitting(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen bg-background p-4">
        <div className="text-muted-foreground text-sm">Loading...</div>
      </div>
    )
  }

  if (error || !details) {
    return (
      <div className="flex flex-col h-screen bg-background p-4">
        <p className="text-sm text-destructive text-center">{error || "Request not found"}</p>
      </div>
    )
  }

  const disabled = !buttonsReady || submitting

  return (
    <div className="flex flex-col h-screen bg-background overflow-hidden">
      <div className="flex flex-col flex-1 p-4 overflow-hidden">
        {/* Header */}
        <div className="mb-3 flex-shrink-0">
          <div className="flex items-center gap-2 mb-0.5">
            <ProvidersIcon className="h-5 w-5 text-blue-500" />
            <h1 className="text-sm font-bold">Sampling Request</h1>
          </div>
          <p className="text-xs text-muted-foreground">
            A backend MCP server is requesting an LLM completion
          </p>
        </div>

        {/* Details Grid */}
        <div className="flex-1 overflow-auto">
          <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
            <span className="text-muted-foreground">Server:</span>
            <span className="font-medium truncate">{details.server_id}</span>

            <span className="text-muted-foreground">Messages:</span>
            <span>{details.message_count}</span>

            {details.max_tokens && (
              <>
                <span className="text-muted-foreground">Max tokens:</span>
                <span>{details.max_tokens}</span>
              </>
            )}
          </div>

          {details.system_prompt && (
            <div className="mt-2">
              <span className="text-[10px] font-semibold text-muted-foreground uppercase">System prompt</span>
              <div className="text-xs mt-0.5 bg-muted/50 rounded p-2 max-h-16 overflow-auto font-mono">
                {details.system_prompt.length > 200
                  ? details.system_prompt.slice(0, 200) + "..."
                  : details.system_prompt}
              </div>
            </div>
          )}

          {details.messages_preview.length > 0 && (
            <div className="mt-2">
              <span className="text-[10px] font-semibold text-muted-foreground uppercase">Messages</span>
              <div className="mt-0.5 space-y-1 max-h-28 overflow-auto">
                {details.messages_preview.map((msg, i) => (
                  <div key={i} className="text-xs bg-muted/50 rounded p-1.5">
                    <span className="font-semibold text-muted-foreground">{msg.role}: </span>
                    <span className="font-mono">{msg.content}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Action Buttons */}
        <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
          <Button
            variant="destructive"
            className="flex-1 h-10 font-bold"
            onClick={() => handleAction("deny")}
            disabled={disabled}
          >
            Deny
          </Button>
          <Button
            className="flex-1 h-10 bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
            onClick={() => handleAction("allow")}
            disabled={disabled}
          >
            Allow
          </Button>
        </div>
      </div>
    </div>
  )
}
