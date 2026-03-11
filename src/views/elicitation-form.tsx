import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/Toggle"

interface ElicitationDetails {
  request_id: string
  server_id: string
  message: string
  schema: any
  timeout_seconds: number
  created_at_secs_ago: number
}

function SchemaField({ name, schema, value, onChange }: {
  name: string
  schema: any
  value: any
  onChange: (val: any) => void
}) {
  if (schema.enum) {
    return (
      <div>
        <label className="text-xs text-muted-foreground">{schema.title || name}</label>
        <select
          className="w-full mt-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm"
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">Select...</option>
          {schema.enum.map((opt: string) => (
            <option key={opt} value={opt}>{opt}</option>
          ))}
        </select>
      </div>
    )
  }

  if (schema.type === "boolean") {
    return (
      <div className="flex items-center justify-between">
        <label className="text-xs text-muted-foreground">{schema.title || name}</label>
        <Switch checked={!!value} onCheckedChange={onChange} />
      </div>
    )
  }

  if (schema.type === "number" || schema.type === "integer") {
    return (
      <div>
        <label className="text-xs text-muted-foreground">{schema.title || name}</label>
        <Input
          type="number"
          className="mt-1"
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value ? Number(e.target.value) : null)}
          placeholder={schema.description || name}
        />
      </div>
    )
  }

  // Default: string
  return (
    <div>
      <label className="text-xs text-muted-foreground">{schema.title || name}</label>
      <Input
        className="mt-1"
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value)}
        placeholder={schema.description || name}
      />
    </div>
  )
}

export function ElicitationForm() {
  const [details, setDetails] = useState<ElicitationDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [formData, setFormData] = useState<Record<string, any>>({})

  useEffect(() => {
    const label = (window as any).__TAURI_INTERNALS__?.metadata?.currentWebview?.label || ""
    const requestId = label.replace("elicitation-form-", "")

    if (!requestId) {
      setError("Missing request ID")
      setLoading(false)
      return
    }

    invoke<ElicitationDetails>("get_elicitation_details", { requestId })
      .then((d) => {
        setDetails(d)
        // Initialize form data with defaults from schema
        if (d.schema?.properties) {
          const defaults: Record<string, any> = {}
          for (const [key, prop] of Object.entries(d.schema.properties)) {
            if ((prop as any).default !== undefined) {
              defaults[key] = (prop as any).default
            }
          }
          setFormData(defaults)
        }
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false))
  }, [])

  const handleSubmit = async () => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_elicitation_response", {
        requestId: details.request_id,
        data: formData,
      })
      const window = getCurrentWebviewWindow()
      await window.close()
    } catch (e) {
      setError(String(e))
      setSubmitting(false)
    }
  }

  const handleCancel = async () => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("cancel_elicitation", { requestId: details.request_id })
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

  const properties = details.schema?.properties || {}
  const propertyEntries = Object.entries(properties)

  return (
    <div className="h-screen bg-background text-foreground p-4 flex flex-col">
      <div className="flex-1 space-y-3 overflow-auto">
        <div>
          <h2 className="text-base font-semibold">Input Required</h2>
          <p className="text-xs text-muted-foreground mt-1">
            Server <span className="font-medium text-foreground">{details.server_id}</span>
          </p>
        </div>

        {details.message && (
          <p className="text-sm">{details.message}</p>
        )}

        <div className="space-y-3">
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
              schema={details.schema}
              value={formData.value}
              onChange={(val) => setFormData(prev => ({ ...prev, value: val }))}
            />
          )}
        </div>
      </div>

      <div className="flex gap-2 pt-3 border-t">
        <Button
          variant="outline"
          className="flex-1"
          onClick={handleCancel}
          disabled={submitting}
        >
          Cancel
        </Button>
        <Button
          className="flex-1"
          onClick={handleSubmit}
          disabled={submitting}
        >
          Submit
        </Button>
      </div>
    </div>
  )
}
