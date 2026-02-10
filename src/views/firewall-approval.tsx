import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { LogicalSize } from "@tauri-apps/api/dpi"
import { ChevronDown, Pencil } from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { ProvidersIcon, McpIcon, SkillsIcon, StoreIcon } from "@/components/icons/category-icons"
import type { ModelInfo, ClientInfo, ModelPermissions } from "@/types/tauri-commands"

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
}

type ApprovalAction = "deny" | "deny_session" | "deny_always" | "allow_once" | "allow_session" | "allow_1_hour" | "allow_permanent"

// Parse JSON arguments into key-value pairs for display
function parseArguments(jsonStr: string): { key: string; value: string }[] {
  if (!jsonStr || jsonStr === "{}") return []
  try {
    const obj = JSON.parse(jsonStr)
    if (typeof obj !== "object" || obj === null) return []
    return Object.entries(obj).map(([key, value]) => ({
      key,
      value: typeof value === "string" ? value : JSON.stringify(value),
    }))
  } catch {
    return []
  }
}

// Determine request type from details
function getRequestType(details: ApprovalDetails): "marketplace" | "skill" | "model" | "tool" {
  if (
    details.server_name.toLowerCase().includes("marketplace") ||
    details.tool_name.toLowerCase().includes("marketplace")
  ) {
    return "marketplace"
  }
  if (
    details.tool_name.startsWith("skill_") ||
    details.server_name.toLowerCase().includes("skill")
  ) {
    return "skill"
  }
  if (details.is_model_request) {
    return "model"
  }
  return "tool"
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

  const requestType = getRequestType(details)
  const parsedArgs = parseArguments(details.arguments_preview)
  // Get header content based on request type
  const getHeaderContent = () => {
    switch (requestType) {
      case "marketplace":
        return {
          icon: <StoreIcon className="h-5 w-5 text-pink-500" />,
          title: "Marketplace Installation",
          description: "A skill from the marketplace wants to be installed",
        }
      case "skill":
        return {
          icon: <SkillsIcon className="h-5 w-5 text-purple-500" />,
          title: "Skill Execution",
          description: "A skill is requesting permission to run",
        }
      case "model":
        return {
          icon: <ProvidersIcon className="h-5 w-5 text-amber-500" />,
          title: "Model Access",
          description: "Access to an AI model is being requested",
        }
      default:
        return {
          icon: <McpIcon className="h-5 w-5 text-blue-500" />,
          title: "Tool Approval",
          description: "A tool is requesting permission to execute",
        }
    }
  }

  const header = getHeaderContent()
  const canEdit = requestType !== "marketplace"

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


  return (
    <div className="flex flex-col h-screen bg-background overflow-hidden">
      <div className="flex flex-col flex-1 p-4 overflow-hidden">
        {/* Header */}
        <div className="mb-3 flex-shrink-0">
          <div className="flex items-center gap-2 mb-0.5">
            {header.icon}
            <h1 className="text-sm font-bold">{header.title}</h1>
          </div>
          <p className="text-xs text-muted-foreground">{header.description}</p>
        </div>

        {editMode ? (
          /* Edit Mode View */
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
        ) : (
          /* Normal View */
          <div className="flex-1 overflow-auto">
            <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
              <span className="text-muted-foreground">Client:</span>
              <span className="font-medium truncate">{details.client_name}</span>

              {requestType === "marketplace" ? (
                <>
                  <span className="text-muted-foreground">Skill:</span>
                  <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                    {details.tool_name}
                  </code>
                </>
              ) : requestType === "skill" ? (
                <>
                  <span className="text-muted-foreground">Skill:</span>
                  <span className="truncate">{details.server_name}</span>
                  <span className="text-muted-foreground">Action:</span>
                  <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                    {details.tool_name.replace(/^skill_/, "").replace(/_/g, " ")}
                  </code>
                </>
              ) : requestType === "model" ? (
                <>
                  <span className="text-muted-foreground">Model:</span>
                  <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                    {details.tool_name}
                  </code>
                  <span className="text-muted-foreground">Provider:</span>
                  <span className="truncate">{details.server_name}</span>
                </>
              ) : (
                <>
                  <span className="text-muted-foreground">Tool:</span>
                  <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                    {details.tool_name}
                  </code>
                  <span className="text-muted-foreground">Server:</span>
                  <span className="truncate">{details.server_name}</span>
                </>
              )}

              {/* Arguments inline */}
              {parsedArgs.map(({ key, value }) => (
                <span key={key} className="contents">
                  <span className="text-muted-foreground">{key}:</span>
                  <span className="font-mono truncate" title={value}>
                    {value.length > 60 ? `${value.slice(0, 60)}...` : value}
                  </span>
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Action Buttons */}
        <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
          {editMode ? (
            /* Edit mode actions */
            <>
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
            </>
          ) : (
            /* Normal mode actions */
            <>
              {/* Split button: Deny Once (main) + dropdown for other options */}
              <div className="flex flex-1">
                <Button
                  variant="destructive"
                  className="flex-1 h-10 rounded-r-none font-bold"
                  onClick={() => handleAction("deny")}
                  disabled={submitting}
                >
                  Deny
                </Button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="destructive"
                      className="h-10 px-2 rounded-l-none border-l border-red-700"
                      disabled={submitting}
                    >
                      <ChevronDown className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start">
                    {!details.is_model_request && (
                      <DropdownMenuItem onClick={() => handleAction("deny_session")}>
                        Deny for Session
                      </DropdownMenuItem>
                    )}
                    <DropdownMenuItem onClick={() => handleAction("deny_always")}>
                      Deny Always
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>

              {/* Edit button - hidden for marketplace */}
              {canEdit && (
                <Button
                  className="h-10 px-3 bg-amber-500 hover:bg-amber-600 text-white font-bold"
                  onClick={enterEditMode}
                  disabled={submitting}
                >
                  <Pencil className="h-3.5 w-3.5 mr-1" />
                  Modify
                </Button>
              )}

              {/* Split button: Allow Once (main) + dropdown for other options */}
              <div className="flex flex-1">
                <Button
                  className="flex-1 h-10 rounded-r-none bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
                  onClick={() => handleAction("allow_once")}
                  disabled={submitting}
                >
                  Allow Once
                </Button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      className="h-10 px-2 rounded-l-none border-l border-emerald-700 bg-emerald-600 hover:bg-emerald-700 text-white"
                      disabled={submitting}
                    >
                      <ChevronDown className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    {!details.is_model_request && (
                      <DropdownMenuItem onClick={() => handleAction("allow_session")}>
                        Allow for Session
                      </DropdownMenuItem>
                    )}
                    {details.is_model_request && (
                      <DropdownMenuItem onClick={() => handleAction("allow_1_hour")}>
                        Allow for 1 Hour
                      </DropdownMenuItem>
                    )}
                    <DropdownMenuItem onClick={() => handleAction("allow_permanent")}>
                      Allow Always
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
