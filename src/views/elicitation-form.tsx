import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { LogicalSize } from "@tauri-apps/api/dpi"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/Toggle"
import { MessageSquare } from "lucide-react"

interface ElicitationDetails {
  request_id: string
  server_id: string
  message: string
  schema: Record<string, unknown>
  timeout_seconds: number
  created_at_secs_ago: number
}

function SchemaField({ name, schema, value, onChange }: {
  name: string
  schema: Record<string, unknown>
  value: unknown
  onChange: (val: unknown) => void
}) {
  const title = (schema.title as string) || name
  const description = schema.description as string | undefined

  if (schema.enum) {
    return (
      <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
        <label className="text-xs text-muted-foreground">{title}:</label>
        <select
          className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-ring"
          value={(value as string) ?? ""}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">Select...</option>
          {(schema.enum as string[]).map((opt: string) => (
            <option key={opt} value={opt}>{opt}</option>
          ))}
        </select>
      </div>
    )
  }

  if (schema.type === "boolean") {
    return (
      <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
        <label className="text-xs text-muted-foreground">{title}:</label>
        <Switch checked={!!value} onCheckedChange={onChange} />
      </div>
    )
  }

  if (schema.type === "number" || schema.type === "integer") {
    return (
      <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
        <label className="text-xs text-muted-foreground">{title}:</label>
        <Input
          type="number"
          className="h-7 text-xs"
          value={(value as string) ?? ""}
          onChange={(e) => onChange(e.target.value ? Number(e.target.value) : null)}
          placeholder={description || name}
        />
      </div>
    )
  }

  // Default: string
  return (
    <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
      <label className="text-xs text-muted-foreground">{title}:</label>
      <Input
        className="h-7 text-xs"
        value={(value as string) ?? ""}
        onChange={(e) => onChange(e.target.value)}
        placeholder={description || name}
      />
    </div>
  )
}

export function ElicitationForm() {
  const [details, setDetails] = useState<ElicitationDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [formData, setFormData] = useState<Record<string, unknown>>({})
  const [buttonsReady, setButtonsReady] = useState(false)

  useEffect(() => {
    const loadDetails = async () => {
      try {
        const window = getCurrentWebviewWindow()
        const label = window.label
        const requestId = label.replace("elicitation-form-", "")

        const result = await invoke<ElicitationDetails>("get_elicitation_details", { requestId })
        setDetails(result)

        // Initialize form data with defaults from schema
        if (result.schema?.properties) {
          const defaults: Record<string, unknown> = {}
          for (const [key, prop] of Object.entries(result.schema.properties as Record<string, Record<string, unknown>>)) {
            if (prop.default !== undefined) {
              defaults[key] = prop.default
            }
          }
          setFormData(defaults)
        }

        // Calculate height based on number of fields
        const fieldCount = Object.keys((result.schema?.properties || {}) as object).length
        const height = Math.min(500, 220 + fieldCount * 40)

        // Resize and show window
        const win = getCurrentWebviewWindow()
        await win.setSize(new LogicalSize(400, height))
        await win.center()
        await win.show()
        await win.setFocus()
      } catch (err) {
        console.error("Failed to load elicitation details:", err)
        setError(typeof err === "string" ? err : "Failed to load elicitation details")
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  // Delay buttons until window is focused
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

  const handleSubmit = async () => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_elicitation_response", {
        requestId: details.request_id,
        data: formData,
      })
      await getCurrentWebviewWindow().close()
    } catch (err) {
      console.error("Failed to submit elicitation:", err)
      setError(typeof err === "string" ? err : "Failed to submit response")
      setSubmitting(false)
    }
  }

  const handleCancel = async () => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("cancel_elicitation", { requestId: details.request_id })
      await getCurrentWebviewWindow().close()
    } catch (err) {
      console.error("Failed to cancel elicitation:", err)
      setError(typeof err === "string" ? err : "Failed to cancel")
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

  const properties = (details.schema?.properties || {}) as Record<string, Record<string, unknown>>
  const propertyEntries = Object.entries(properties)
  const disabled = !buttonsReady || submitting

  return (
    <div className="flex flex-col h-screen bg-background overflow-hidden">
      <div className="flex flex-col flex-1 p-4 overflow-hidden">
        {/* Header */}
        <div className="mb-3 flex-shrink-0">
          <div className="flex items-center gap-2 mb-0.5">
            <MessageSquare className="h-5 w-5 text-purple-500" />
            <h1 className="text-sm font-bold">Input Required</h1>
          </div>
          <p className="text-xs text-muted-foreground">
            Server <span className="font-medium text-foreground">{details.server_id}</span> is requesting user input
          </p>
        </div>

        {/* Message */}
        {details.message && (
          <p className="text-xs mb-3 flex-shrink-0">{details.message}</p>
        )}

        {/* Form Fields */}
        <div className="flex-1 overflow-auto">
          <div className="space-y-2">
            {propertyEntries.map(([key, propSchema]) => (
              <SchemaField
                key={key}
                name={key}
                schema={propSchema}
                value={formData[key]}
                onChange={(val) => setFormData(prev => ({ ...prev, [key]: val }))}
              />
            ))}
            {propertyEntries.length === 0 && details.schema?.type === "string" && (
              <SchemaField
                name="value"
                schema={details.schema as Record<string, unknown>}
                value={formData.value}
                onChange={(val) => setFormData(prev => ({ ...prev, value: val }))}
              />
            )}
          </div>
        </div>

        {/* Action Buttons - matching firewall pattern */}
        <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
          <Button
            variant="destructive"
            className="flex-1 h-10 font-bold"
            onClick={handleCancel}
            disabled={disabled}
          >
            Deny
          </Button>
          <Button
            className="flex-1 h-10 bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
            onClick={handleSubmit}
            disabled={disabled}
          >
            Submit
          </Button>
        </div>
      </div>
    </div>
  )
}
