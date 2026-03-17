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
  type MarketplaceListingInfo,
} from "@/components/shared/FirewallApprovalCard"
import type { ModelInfo, ClientInfo, ModelPermissions, SafetyVerdict, CategoryActionRequired, McpServerListing, SkillListing, SecretFindingSummary } from "@/types/tauri-commands"

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
  is_free_tier_fallback?: boolean
  is_auto_router_request?: boolean
  is_mcp_via_llm_request?: boolean
  is_secret_scan_request?: boolean
  guardrail_details?: {
    verdicts: SafetyVerdict[]
    actions_required: CategoryActionRequired[]
    total_duration_ms: number
    scan_direction: "request" | "response"
    flagged_text: string
  }
  secret_scan_details?: {
    findings: SecretFindingSummary[]
    scan_duration_ms: number
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

type EditorTab = "params" | "messages" | "raw"

export function FirewallApproval() {
  const [details, setDetails] = useState<ApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [buttonsReady, setButtonsReady] = useState(false)

  // Edit mode state
  const [editMode, setEditMode] = useState(false)
  const [fullArguments, setFullArguments] = useState<string | null>(null)
  const [editedJson, setEditedJson] = useState<string>("")
  const [jsonValid, setJsonValid] = useState(true)
  const [editorTab, setEditorTab] = useState<EditorTab>("params")
  // Key-value pairs for kv editor mode (tool/skill)
  const [kvPairs, setKvPairs] = useState<{ key: string; value: string }[]>([])
  // Model params for model/auto-router editor
  const [modelParams, setModelParams] = useState<Record<string, string>>({})
  // MCP via LLM tools (preserved through edit mode, read-only)
  const [mcpTools, setMcpTools] = useState<unknown[] | null>(null)
  // Available models for dropdown (filtered by client permissions)
  const [allowedModels, setAllowedModels] = useState<ModelInfo[]>([])
  // Candidate models for auto-router
  const [candidateModels, setCandidateModels] = useState<string[]>([])
  // Messages editor
  const [editedMessages, setEditedMessages] = useState<{ role: string; content: string }[]>([])
  // Marketplace listing details
  const [marketplaceListing, setMarketplaceListing] = useState<MarketplaceListingInfo | null>(null)

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

        // Fetch marketplace listing details for install tools
        if (result.tool_name.includes("marketplace__install")) {
          try {
            const args = JSON.parse(result.arguments_preview || "{}")
            const name = args.name as string | undefined
            if (name) {
              if (result.tool_name.includes("install_mcp_server")) {
                const listings = await invoke<McpServerListing[]>("marketplace_search_mcp_servers", { query: name })
                const listing = listings.find((l) => l.name === name)
                if (listing) {
                  setMarketplaceListing({
                    name: listing.name,
                    description: listing.description,
                    homepage: listing.homepage,
                    vendor: listing.vendor,
                    install_type: "mcp_server",
                  })
                }
              } else if (result.tool_name.includes("install_skill")) {
                const listings = await invoke<SkillListing[]>("marketplace_search_skills", { query: name })
                const listing = listings.find((l) => l.name === name)
                if (listing) {
                  setMarketplaceListing({
                    name: listing.name,
                    description: listing.description,
                    author: listing.author,
                    source_label: listing.source_label,
                    source_repo: listing.source_repo,
                    install_type: "skill",
                  })
                }
              }
            }
          } catch (err) {
            console.error("Failed to fetch marketplace listing:", err)
          }
        }

        // Resize window based on content type, then show
        const win = getCurrentWebviewWindow()
        if (result.tool_name.includes("marketplace__install")) {
          await win.setSize(new LogicalSize(440, 380))
          await win.center()
        } else if (result.is_free_tier_fallback) {
          await win.setSize(new LogicalSize(400, 280))
          await win.center()
        } else if (result.is_guardrail_request) {
          const verdictCount = result.guardrail_details?.verdicts?.length || 0
          const hasFlaggedText = !!result.guardrail_details?.flagged_text
          const height = Math.min(580, 320 + verdictCount * 60 + (hasFlaggedText ? 80 : 0))
          await win.setSize(new LogicalSize(440, height))
          await win.center()
        }
        await win.show()
        await win.setFocus()
      } catch (err) {
        console.error("Failed to load approval details:", err)
        // Request expired or not found — just close the popup silently
        try {
          await getCurrentWebviewWindow().close()
        } catch {
          // Fallback: show error if we can't close
          setError(typeof err === "string" ? err : "Failed to load approval details")
        }
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [])

  // Start button delay when the window gains focus (not on mount)
  // This prevents accidental clicks when popups appear suddenly.
  // Also handles the case where the window spawns already focused.
  useEffect(() => {
    if (loading || !details || buttonsReady) return
    let timer: ReturnType<typeof setTimeout> | null = null

    const startTimer = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => setButtonsReady(true), 500)
    }

    // Check if already focused (window spawned with focus)
    const win = getCurrentWebviewWindow()
    win.isFocused().then((focused) => {
      if (focused) startTimer()
    })

    // Also listen for future focus changes
    const unlistenPromise = win.onFocusChanged(({ payload: focused }) => {
      if (focused) startTimer()
    })

    return () => {
      if (timer) clearTimeout(timer)
      unlistenPromise.then(fn => { try { fn() } catch {} }).catch(() => {})
    }
  }, [loading, details, buttonsReady])

  /** Resize the window based on the active editor tab */
  const resizeForTab = async (tab: EditorTab) => {
    const win = getCurrentWebviewWindow()
    if (tab === "params") {
      await win.setSize(new LogicalSize(500, 520))
    } else {
      // Messages and Raw JSON need more space
      await win.setSize(new LogicalSize(560, 600))
    }
    await win.center()
  }

  /** Extract the request object from full arguments (handles auto-router nesting) */
  const getRequestFromArgs = (obj: Record<string, unknown>, isAutoRouter: boolean): Record<string, unknown> => {
    if (isAutoRouter && typeof obj.request === "object" && obj.request !== null) {
      return obj.request as Record<string, unknown>
    }
    return obj
  }

  const enterEditMode = async () => {
    if (!details) return
    try {
      const args = await invoke<string | null>("get_firewall_full_arguments", {
        requestId: details.request_id,
      })
      const jsonStr = args || "{}"
      setFullArguments(jsonStr)
      setJsonValid(true)
      setEditMode(true)

      const isModelLike = details.is_model_request || details.is_auto_router_request

      try {
        const obj = JSON.parse(jsonStr)

        if (isModelLike) {
          // For model/auto-router: extract request object and initialize params + messages
          const reqObj = getRequestFromArgs(obj, !!details.is_auto_router_request)

          // Initialize model params
          const params: Record<string, string> = {}
          if (reqObj.model != null) params.model = String(reqObj.model)
          for (const field of MODEL_PARAM_FIELDS) {
            const val = reqObj[field.key]
            params[field.key] = val != null ? String(val) : ""
          }
          setModelParams(params)

          // Initialize messages
          const msgs = Array.isArray(reqObj.messages) ? reqObj.messages : []
          setEditedMessages(
            msgs.map((m: Record<string, unknown>) => ({
              role: typeof m.role === "string" ? m.role : "user",
              content: typeof m.content === "string"
                ? m.content
                : JSON.stringify(m.content),
            }))
          )

          // Capture MCP via LLM tools (preserved through edit, not editable via params)
          if (Array.isArray(reqObj.tools)) {
            setMcpTools(reqObj.tools as unknown[])
          }

          // For auto-router: extract candidate models
          if (details.is_auto_router_request && Array.isArray(obj.candidate_models)) {
            setCandidateModels(obj.candidate_models as string[])
          }

          // Set raw JSON to the request portion only
          setEditedJson(JSON.stringify(reqObj, null, 2))
        } else {
          // For tools/skills: use kv pairs
          if (typeof obj === "object" && obj !== null) {
            setKvPairs(
              Object.entries(obj).map(([key, value]) => ({
                key,
                value: typeof value === "string" ? value : JSON.stringify(value),
              }))
            )
          }
          setEditedJson(jsonStr)
        }
      } catch {
        setKvPairs([])
        setEditedJson(jsonStr)
      }

      // Fetch allowed models for model/auto-router
      if (isModelLike) {
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

      // Reset tab to params and resize
      setEditorTab("params")
      await resizeForTab("params")
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
    setMcpTools(null)
    setAllowedModels([])
    setCandidateModels([])
    setEditedMessages([])
    setEditorTab("params")

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

  /** Build the full edited request JSON for model/auto-router */
  const buildEditedRequestJson = (): Record<string, unknown> => {
    const obj: Record<string, unknown> = {}
    // Model
    if (modelParams.model) obj.model = modelParams.model
    // Params
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
    // Messages
    obj.messages = editedMessages.map((m) => {
      // Try to parse content as JSON (for multipart content)
      let content: unknown = m.content
      try {
        const parsed = JSON.parse(m.content)
        if (Array.isArray(parsed)) content = parsed
      } catch {
        // keep as string
      }
      return { role: m.role, content }
    })
    // MCP via LLM tools (read-only, preserved from original request)
    if (mcpTools && mcpTools.length > 0) {
      obj.tools = mcpTools
    }
    return obj
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
        const isModelLike = details.is_model_request || details.is_auto_router_request
        let editedData: string

        if (isModelLike) {
          if (editorTab === "raw") {
            // Raw JSON mode — use editedJson directly, wrapping for auto-router
            if (details.is_auto_router_request) {
              editedData = JSON.stringify({ request: JSON.parse(editedJson) })
            } else {
              editedData = editedJson
            }
          } else {
            // Params/Messages mode — build from state
            const reqObj = buildEditedRequestJson()
            if (details.is_auto_router_request) {
              editedData = JSON.stringify({ request: reqObj })
            } else {
              editedData = JSON.stringify(reqObj)
            }
          }
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
    is_guardrail_request: details.is_guardrail_request,
    is_free_tier_fallback: details.is_free_tier_fallback,
    is_auto_router_request: details.is_auto_router_request,
    is_secret_scan_request: details.is_secret_scan_request,
  })
  const isModelLike = requestType === "model" || requestType === "auto_router"

  // Tab style helper
  const tabClass = (tab: EditorTab) =>
    `text-xs px-2 py-1 rounded ${editorTab === tab ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`

  // Render the model + params editor (used by model and auto-router)
  const renderParamsEditor = () => (
    <div className="flex flex-col gap-2 overflow-auto flex-1">
      {requestType === "model" && (
        <div className="text-xs text-muted-foreground mb-1">
          Provider: <span className="font-medium text-foreground">{details.server_name}</span>
        </div>
      )}
      {/* Model dropdown */}
      <div className="grid grid-cols-[120px_1fr] gap-2 items-center">
        <label className="text-xs text-muted-foreground">Model:</label>
        <select
          className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-ring"
          value={modelParams.model ?? ""}
          onChange={(e) => setModelParams((prev) => ({ ...prev, model: e.target.value }))}
        >
          {/* For auto-router: option to keep auto */}
          {requestType === "auto_router" && (
            <option value="localrouter/auto">Auto (let router decide)</option>
          )}
          {/* Candidate models for auto-router */}
          {candidateModels.length > 0 && (
            <optgroup label="Router Candidates">
              {candidateModels.map((m) => (
                <option key={`candidate-${m}`} value={m.split("/").slice(1).join("/") || m}>
                  {m}
                </option>
              ))}
            </optgroup>
          )}
          {/* All allowed models */}
          <optgroup label={candidateModels.length > 0 ? "All Available" : "Models"}>
            {/* Include current value even if not in the list */}
            {modelParams.model &&
              modelParams.model !== "localrouter/auto" &&
              !allowedModels.some((m) => m.id === modelParams.model) &&
              !candidateModels.some((c) => c.endsWith(`/${modelParams.model}`)) && (
                <option value={modelParams.model}>{modelParams.model}</option>
              )}
            {allowedModels.map((m) => (
              <option key={`${m.provider}__${m.id}`} value={m.id}>
                {m.id}
              </option>
            ))}
          </optgroup>
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

  // Render messages editor
  const renderMessagesEditor = () => (
    <div className="flex flex-col flex-1 overflow-hidden">
      <div className="flex items-center justify-between mb-2 flex-shrink-0">
        <span className="text-xs text-muted-foreground">
          {editedMessages.length} message{editedMessages.length !== 1 ? "s" : ""}
        </span>
        <button
          className="text-xs px-2 py-1 rounded bg-muted text-muted-foreground hover:bg-muted/80"
          onClick={() => setEditedMessages((prev) => [...prev, { role: "user", content: "" }])}
        >
          + Add
        </button>
      </div>
      <div className="flex flex-col gap-2 overflow-auto flex-1">
        {editedMessages.map((msg, idx) => (
          <div key={idx} className="flex flex-col gap-1 bg-muted/30 rounded p-2">
            <div className="flex items-center gap-2">
              <select
                className="h-6 px-1 text-xs rounded border border-border bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                value={msg.role}
                onChange={(e) => {
                  const updated = [...editedMessages]
                  updated[idx] = { ...msg, role: e.target.value }
                  setEditedMessages(updated)
                }}
              >
                <option value="system">system</option>
                <option value="user">user</option>
                <option value="assistant">assistant</option>
                <option value="tool">tool</option>
              </select>
              <button
                className="ml-auto text-[10px] text-destructive hover:text-destructive/80"
                onClick={() => setEditedMessages((prev) => prev.filter((_, i) => i !== idx))}
              >
                Remove
              </button>
            </div>
            <textarea
              className="min-h-[48px] px-2 py-1 text-xs rounded border border-border bg-background text-foreground font-mono resize-y focus:outline-none focus:ring-1 focus:ring-ring"
              value={msg.content}
              rows={Math.min(6, Math.max(2, msg.content.split("\n").length))}
              onChange={(e) => {
                const updated = [...editedMessages]
                updated[idx] = { ...msg, content: e.target.value }
                setEditedMessages(updated)
              }}
              spellCheck={false}
            />
          </div>
        ))}
        {editedMessages.length === 0 && (
          <div className="text-xs text-muted-foreground italic">No messages</div>
        )}
      </div>
    </div>
  )

  // Render raw JSON editor
  const renderRawJsonEditor = () => (
    <div className="flex flex-col flex-1 overflow-hidden">
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
    </div>
  )

  // Render edit mode view for tool/skill (kv + raw toggle — unchanged from original)
  const renderToolEditor = () => (
    <div className="flex flex-col flex-1 overflow-hidden">
      {/* Editor mode tabs */}
      <div className="flex gap-1 mb-2 flex-shrink-0">
        <button
          className={`text-xs px-2 py-1 rounded ${editorTab === "params" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
          onClick={() => {
            if (editorTab === "raw") syncJsonToKv(editedJson)
            setEditorTab("params")
          }}
        >
          Fields
        </button>
        <button
          className={`text-xs px-2 py-1 rounded ${editorTab === "raw" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
          onClick={() => {
            if (editorTab === "params") syncKvToJson(kvPairs)
            setEditorTab("raw")
          }}
        >
          JSON
        </button>
      </div>

      {editorTab === "params" ? (
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
        renderRawJsonEditor()
      )}
    </div>
  )

  /** Sync model params + messages into the raw JSON editor when switching to raw tab */
  const syncModelStateToJson = () => {
    const obj = buildEditedRequestJson()
    const json = JSON.stringify(obj, null, 2)
    setEditedJson(json)
    setJsonValid(true)
  }

  /** Sync raw JSON back to model params + messages when switching away from raw tab */
  const syncJsonToModelState = () => {
    try {
      const obj = JSON.parse(editedJson)
      if (typeof obj !== "object" || obj === null) return
      // Params
      const params: Record<string, string> = {}
      if (obj.model != null) params.model = String(obj.model)
      for (const field of MODEL_PARAM_FIELDS) {
        const val = obj[field.key]
        params[field.key] = val != null ? String(val) : ""
      }
      setModelParams(params)
      // Messages
      if (Array.isArray(obj.messages)) {
        setEditedMessages(
          obj.messages.map((m: Record<string, unknown>) => ({
            role: typeof m.role === "string" ? m.role : "user",
            content: typeof m.content === "string" ? m.content : JSON.stringify(m.content),
          }))
        )
      }
      // Preserve tools from raw JSON edits
      if (Array.isArray(obj.tools)) {
        setMcpTools(obj.tools as unknown[])
      }
      setJsonValid(true)
    } catch {
      setJsonValid(false)
    }
  }

  // Render the three-tab editor for model/auto-router
  const renderModelLikeEditor = () => (
    <div className="flex flex-col flex-1 overflow-hidden">
      {details.is_mcp_via_llm_request && (
        <div className="text-[11px] text-blue-400 bg-blue-500/10 px-2 py-1 rounded mb-2 flex-shrink-0">
          MCP via LLM — request includes server-injected tools
        </div>
      )}
      {/* Tab bar */}
      <div className="flex gap-1 mb-2 flex-shrink-0">
        <button
          className={tabClass("params")}
          onClick={async () => {
            if (editorTab === "raw") syncJsonToModelState()
            setEditorTab("params")
            await resizeForTab("params")
          }}
        >
          Model + Params
        </button>
        <button
          className={tabClass("messages")}
          onClick={async () => {
            if (editorTab === "raw") syncJsonToModelState()
            setEditorTab("messages")
            await resizeForTab("messages")
          }}
        >
          Messages ({editedMessages.length})
        </button>
        <button
          className={tabClass("raw")}
          onClick={async () => {
            if (editorTab !== "raw") syncModelStateToJson()
            setEditorTab("raw")
            await resizeForTab("raw")
          }}
        >
          JSON
        </button>
      </div>

      {editorTab === "params" && renderParamsEditor()}
      {editorTab === "messages" && renderMessagesEditor()}
      {editorTab === "raw" && renderRawJsonEditor()}
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
          isFreeTierFallback={details.is_free_tier_fallback}
          isAutoRouterRequest={details.is_auto_router_request}
          isSecretScanRequest={details.is_secret_scan_request}
          guardrailVerdicts={details.guardrail_details?.verdicts}
          guardrailDirection={details.guardrail_details?.scan_direction}
          guardrailActions={details.guardrail_details?.actions_required}
          guardrailFlaggedText={details.guardrail_details?.flagged_text}
          secretScanFindings={details.secret_scan_details?.findings}
          secretScanDurationMs={details.secret_scan_details?.scan_duration_ms}
          marketplaceListing={marketplaceListing}
          onAction={buttonsReady ? handleAction : undefined}
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
            ) : requestType === "auto_router" ? (
              <>
                <span className="text-muted-foreground">Mode:</span>
                <span className="font-medium">Auto Model Selection</span>
              </>
            ) : (
              <>
                <span className="text-muted-foreground">{requestType === "skill" ? "Skill" : "Tool"}:</span>
                <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">{details.tool_name}</code>
              </>
            )}
          </div>

          {/* Editor */}
          {isModelLike ? renderModelLikeEditor() : renderToolEditor()}
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
            disabled={submitting || (!jsonValid && editorTab === "raw")}
          >
            Allow with Edits
          </Button>
        </div>
      </div>
    </div>
  )
}
