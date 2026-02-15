import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { LogicalSize } from "@tauri-apps/api/dpi"
import { Button } from "@/components/ui/Button"
import {
  FirewallApprovalCard,
  FirewallApprovalHeader,
  getRequestType,
  type ApprovalAction,
} from "@/components/shared/FirewallApprovalCard"
import type { ModelInfo, ClientInfo, ModelPermissions, SafetyVerdict, CategoryActionRequired } from "@/types/tauri-commands"

interface ApprovalDetails {
  request_id: string
  client_id: string
  client_name: string
  tool_name: string
  server_name: string
  arguments_preview: string
  timeout_seconds: number
  created_at_secs_ago: number
  is_model_request?: boolean
  is_guardrail_request?: boolean
  guardrail_details?: {
    verdicts: SafetyVerdict[]
    actions_required: CategoryActionRequired[]
    total_duration_ms: number
    scan_direction: "request" | "response"
  }
}

// Model param field definitions
interface ModelParamField {
  key: string
  label: string
  type: "text" | "number"
  min?: number
  max?: number
  step?: number
}

// Resolve effective permission for a model given client's model permissions
function resolveModelPermission(
  perms: ModelPermissions,
  provider: string,
  modelId: string
): string {
  const modelKey = `${provider}__${modelId}`
  if (perms.models[modelKey]) return perms.models[modelKey]
  if (perms.providers[provider]) return perms.providers[provider]
  return perms.global
}

const MODEL_PARAM_FIELDS: ModelParamField[] = [
  { key: "temperature", label: "Temperature", type: "number", min: 0, max: 2, step: 0.1 },
  { key: "max_tokens", label: "Max Tokens", type: "number", min: 1, step: 1 },
  { key: "max_completion_tokens", label: "Max Completion Tokens", type: "number", min: 1, step: 1 },
  { key: "top_p", label: "Top P", type: "number", min: 0, max: 1, step: 0.1 },
  { key: "frequency_penalty", label: "Frequency Penalty", type: "number", min: -2, max: 2, step: 0.1 },
  { key: "presence_penalty", label: "Presence Penalty", type: "number", min: -2, max: 2, step: 0.1 },
  { key: "seed", label: "Seed", type: "number", step: 1 },
]

export function FirewallApproval() {
  const [details, setDetails] = useState<ApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Edit mode state
  const [editMode, setEditMode] = useState(false)
  const [fullArguments, setFullArguments] = useState<string | null>(null)
  const [editedJson, setEditedJson] = useState<string>("")
  const [jsonValid, setJsonValid] = useState(true)
  const [editorMode, setEditorMode] = useState<"kv" | "raw">("kv")
  // Key-value pairs for kv editor mode
  const [kvPairs, setKvPairs] = useState<{ key: string; value: string }[]>([])
  // Model params for model editor
  const [modelParams, setModelParams] = useState<Record<string, string>>({})
  // Available models for dropdown (filtered by client permissions)
  const [allowedModels, setAllowedModels] = useState<ModelInfo[]>([])

  useEffect(() => {
    const loadDetails = async () => {
      try {
        const window = getCurrentWebviewWindow()
        const label = window.label
        const requestId = label.replace("firewall-approval-", "")

        const result = await invoke<ApprovalDetails>("get_firewall_approval_details", {
          requestId,
        })
        setDetails(result)

        // Resize window for guardrail popups (more content to display)
        if (result.is_guardrail_request) {
          const win = getCurrentWebviewWindow()
          const verdictCount = result.guardrail_details?.verdicts?.length || 0
          const height = Math.min(500, 320 + verdictCount * 60)
          await win.setSize(new LogicalSize(440, height))
          await win.center()
        }
      } catch (err) {
        console.error("Failed to load approval details:", err)
        setError(typeof err === "string" ? err : "Failed to load approval details")
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  const enterEditMode = async () => {
    if (!details) return
    try {
      const args = await invoke<string | null>("get_firewall_full_arguments", {
        requestId: details.request_id,
      })
      const jsonStr = args || "{}"
      setFullArguments(jsonStr)
      setEditedJson(jsonStr)
      setJsonValid(true)
      setEditMode(true)

      // Initialize kv pairs from JSON
      try {
        const obj = JSON.parse(jsonStr)
        if (typeof obj === "object" && obj !== null) {
          setKvPairs(
            Object.entries(obj).map(([key, value]) => ({
              key,
              value: typeof value === "string" ? value : JSON.stringify(value),
            }))
          )
        }
      } catch {
        setKvPairs([])
      }

      // For model requests, initialize model params and fetch allowed models
      if (details.is_model_request) {
        try {
          const obj = JSON.parse(jsonStr)
          const params: Record<string, string> = {}
          for (const field of MODEL_PARAM_FIELDS) {
            const val = obj[field.key]
            params[field.key] = val != null ? String(val) : ""
          }
          setModelParams(params)
        } catch {
          setModelParams({})
        }

        // Fetch all models and client permissions, then filter
        try {
          const [allModels, clientInfo] = await Promise.all([
            invoke<ModelInfo[]>("list_all_models"),
            invoke<ClientInfo>("get_client", { clientId: details.client_id }),
          ])
          const filtered = allModels.filter((m) => {
            const perm = resolveModelPermission(clientInfo.model_permissions, m.provider, m.id)
            return perm === "allow" || perm === "ask"
          })
          setAllowedModels(filtered)
        } catch (err) {
          console.error("Failed to fetch models for dropdown:", err)
          setAllowedModels([])
        }
      }

      // Resize window for edit mode
      const win = getCurrentWebviewWindow()
      await win.setSize(new LogicalSize(500, 520))
      await win.center()
    } catch (err) {
      console.error("Failed to enter edit mode:", err)
    }
  }

  const exitEditMode = async () => {
    setEditMode(false)
    setFullArguments(null)
    setEditedJson("")
    setKvPairs([])
    setModelParams({})
    setAllowedModels([])
    setEditorMode("kv")

    const win = getCurrentWebviewWindow()
    await win.setSize(new LogicalSize(400, 320))
    await win.center()
  }

  // Sync kv pairs to JSON string
  const syncKvToJson = (pairs: { key: string; value: string }[]) => {
    try {
      const obj: Record<string, unknown> = {}
      for (const { key, value } of pairs) {
        try {
          obj[key] = JSON.parse(value)
        } catch {
          obj[key] = value
        }
      }
      const json = JSON.stringify(obj, null, 2)
      setEditedJson(json)
      setJsonValid(true)
    } catch {
      setJsonValid(false)
    }
  }

  // Sync JSON string to kv pairs
  const syncJsonToKv = (json: string) => {
    try {
      const obj = JSON.parse(json)
      if (typeof obj === "object" && obj !== null) {
        setKvPairs(
          Object.entries(obj).map(([key, value]) => ({
            key,
            value: typeof value === "string" ? value : JSON.stringify(value),
          }))
        )
      }
      setJsonValid(true)
    } catch {
      setJsonValid(false)
    }
  }

  // Build model params JSON from form fields
  const buildModelParamsJson = (): string => {
    const obj: Record<string, unknown> = {}
    for (const field of MODEL_PARAM_FIELDS) {
      const val = modelParams[field.key]
      if (val === "" || val === undefined) {
        obj[field.key] = null
      } else if (field.type === "number") {
        const num = Number(val)
        obj[field.key] = isNaN(num) ? null : num
      } else {
        obj[field.key] = val
      }
    }
    return JSON.stringify(obj, null, 2)
  }

  const handleAction = async (action: ApprovalAction) => {
    if (!details) return
    setSubmitting(true)
    try {
      const params: Record<string, unknown> = {
        requestId: details.request_id,
        action,
      }

      // In edit mode, send edited data
      if (editMode) {
        let editedData: string
        if (details.is_model_request) {
          editedData = buildModelParamsJson()
        } else {
          editedData = editedJson
        }
        // Only send if actually different from original
        if (editedData !== fullArguments) {
          params.editedArguments = editedData
        }
      }

      await invoke("submit_firewall_approval", params)
      await getCurrentWebviewWindow().close()
    } catch (err) {
      console.error("Failed to submit approval:", err)
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

  const requestType = getRequestType({
    server_name: details.server_name,
    tool_name: details.tool_name,
    is_model_request: details.is_model_request,
  })

  // Render edit mode view for model requests
  const renderModelEditor = () => (
    <div className="flex flex-col gap-2 overflow-auto flex-1">
      <div className="text-xs text-muted-foreground mb-1">
        Provider: <span className="font-medium text-foreground">{details.server_name}</span>
      </div>
      {/* Model dropdown */}
      <div className="grid grid-cols-[120px_1fr] gap-2 items-center">
        <label className="text-xs text-muted-foreground">Model:</label>
        <select
          className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-ring"
          value={modelParams.model ?? ""}
          onChange={(e) => setModelParams((prev) => ({ ...prev, model: e.target.value }))}
        >
          {/* Include current value even if not in the list */}
          {modelParams.model && !allowedModels.some((m) => m.id === modelParams.model) && (
            <option value={modelParams.model}>{modelParams.model}</option>
          )}
          {allowedModels.map((m) => (
            <option key={`${m.provider}__${m.id}`} value={m.id}>
              {m.id}
            </option>
          ))}
          {allowedModels.length === 0 && !modelParams.model && (
            <option value="" disabled>Loading models...</option>
          )}
        </select>
      </div>
      {/* Other param fields */}
      {MODEL_PARAM_FIELDS.filter((f) => f.key !== "model").map((field) => (
        <div key={field.key} className="grid grid-cols-[120px_1fr] gap-2 items-center">
          <label className="text-xs text-muted-foreground">{field.label}:</label>
          <input
            type="number"
            className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground placeholder:text-muted-foreground font-mono focus:outline-none focus:ring-1 focus:ring-ring"
            value={modelParams[field.key] ?? ""}
            onChange={(e) => setModelParams((prev) => ({ ...prev, [field.key]: e.target.value }))}
            placeholder="not set"
            min={field.min}
            max={field.max}
            step={field.step}
          />
        </div>
      ))}
    </div>
  )

  // Render edit mode view for tool/skill/prompt requests (kv + raw toggle)
  const renderJsonEditor = () => (
    <div className="flex flex-col flex-1 overflow-hidden">
      {/* Editor mode tabs */}
      <div className="flex gap-1 mb-2 flex-shrink-0">
        <button
          className={`text-xs px-2 py-1 rounded ${editorMode === "kv" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
          onClick={() => {
            if (editorMode === "raw") {
              syncJsonToKv(editedJson)
            }
            setEditorMode("kv")
          }}
        >
          Fields
        </button>
        <button
          className={`text-xs px-2 py-1 rounded ${editorMode === "raw" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
          onClick={() => {
            if (editorMode === "kv") {
              syncKvToJson(kvPairs)
            }
            setEditorMode("raw")
          }}
        >
          JSON
        </button>
      </div>

      {editorMode === "kv" ? (
        <div className="flex flex-col gap-1.5 overflow-auto flex-1">
          {kvPairs.map(({ key, value }, idx) => (
            <div key={key} className="grid grid-cols-[100px_1fr] gap-2 items-start">
              <label className="text-xs text-muted-foreground pt-1 truncate" title={key}>
                {key}:
              </label>
              <textarea
                className="min-h-[28px] px-2 py-1 text-xs rounded border border-border bg-background text-foreground font-mono resize-y focus:outline-none focus:ring-1 focus:ring-ring"
                value={value}
                rows={value.length > 60 ? 3 : 1}
                onChange={(e) => {
                  const newPairs = [...kvPairs]
                  newPairs[idx] = { key, value: e.target.value }
                  setKvPairs(newPairs)
                  syncKvToJson(newPairs)
                }}
              />
            </div>
          ))}
          {kvPairs.length === 0 && (
            <div className="text-xs text-muted-foreground italic">No arguments</div>
          )}
        </div>
      ) : (
        <textarea
          className={`flex-1 px-2 py-1 text-xs rounded border font-mono resize-none bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring ${
            jsonValid ? "border-border" : "border-destructive"
          }`}
          value={editedJson}
          onChange={(e) => {
            setEditedJson(e.target.value)
            try {
              JSON.parse(e.target.value)
              setJsonValid(true)
            } catch {
              setJsonValid(false)
            }
          }}
          spellCheck={false}
        />
      )}
    </div>
  )

  // Normal mode: use the shared card component
  if (!editMode) {
    return (
      <div className="flex flex-col h-screen bg-background overflow-hidden">
        <FirewallApprovalCard
          className="flex flex-col flex-1 p-4 overflow-hidden"
          clientName={details.client_name}
          toolName={details.tool_name}
          serverName={details.server_name}
          argumentsPreview={details.arguments_preview}
          isModelRequest={details.is_model_request}
          isGuardrailRequest={details.is_guardrail_request}
          guardrailVerdicts={details.guardrail_details?.verdicts}
          guardrailDirection={details.guardrail_details?.scan_direction}
          guardrailActions={details.guardrail_details?.actions_required}
          onAction={handleAction}
          onEdit={enterEditMode}
          submitting={submitting}
        />
      </div>
    )
  }

  // Edit mode: custom layout with editors
  return (
    <div className="flex flex-col h-screen bg-background overflow-hidden">
      <div className="flex flex-col flex-1 p-4 overflow-hidden">
        {/* Header */}
        <FirewallApprovalHeader requestType={requestType} />

        {/* Edit Mode Content */}
        <div className="flex-1 overflow-hidden flex flex-col">
          {/* Context info */}
          <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs mb-3 flex-shrink-0">
            <span className="text-muted-foreground">Client:</span>
            <span className="font-medium truncate">{details.client_name}</span>
            {requestType === "model" ? (
              <>
                <span className="text-muted-foreground">Provider:</span>
                <span className="truncate">{details.server_name}</span>
              </>
            ) : (
              <>
                <span className="text-muted-foreground">{requestType === "skill" ? "Skill" : "Tool"}:</span>
                <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">{details.tool_name}</code>
              </>
            )}
          </div>

          {/* Editor */}
          {requestType === "model" ? renderModelEditor() : renderJsonEditor()}
        </div>

        {/* Edit mode actions */}
        <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
          <Button
            variant="ghost"
            className="h-10"
            onClick={exitEditMode}
            disabled={submitting}
          >
            Back
          </Button>
          <div className="flex-1" />
          <Button
            className="h-10 bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
            onClick={() => handleAction("allow_once")}
            disabled={submitting || (!jsonValid && !details.is_model_request)}
          >
            Allow with Edits
          </Button>
        </div>
      </div>
    </div>
  )
}
