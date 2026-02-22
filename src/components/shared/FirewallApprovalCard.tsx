/**
 * Shared presentational component for the firewall approval dialog.
 * Used by both the Tauri app (src/views/firewall-approval.tsx)
 * and the website demo (website/src/components/FirewallApprovalDemo.tsx).
 *
 * Keep this component free of Tauri-specific imports so the website can use it.
 */
import { ChevronDown, Pencil } from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { ProvidersIcon, McpIcon, SkillsIcon, StoreIcon } from "@/components/icons/category-icons"
import { Shield } from "lucide-react"
import type { SafetyVerdict, CategoryActionRequired } from "@/types/tauri-commands"

export type ApprovalAction = "deny" | "deny_session" | "deny_always" | "block_categories" | "allow_once" | "allow_session" | "allow_1_hour" | "allow_permanent" | "allow_categories" | "deny_1_hour"

export type RequestType = "marketplace" | "skill" | "model" | "tool" | "guardrail"

/** Determine request type from server/tool names */
export function getRequestType(details: {
  server_name: string
  tool_name: string
  is_model_request?: boolean
  is_guardrail_request?: boolean
}): RequestType {
  if (details.is_guardrail_request) {
    return "guardrail"
  }
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

/** Parse JSON arguments into key-value pairs for display */
export function parseArguments(jsonStr: string): { key: string; value: string }[] {
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

/** Get header icon, title, and description for a request type */
export function getHeaderContent(requestType: RequestType) {
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
    case "guardrail":
      return {
        icon: <Shield className="h-5 w-5 text-red-500" />,
        title: "GuardRail Alert",
        description: "Content flagged by guardrail rules",
      }
    default:
      return {
        icon: <McpIcon className="h-5 w-5 text-blue-500" />,
        title: "Tool Approval",
        description: "A tool is requesting permission to execute",
      }
  }
}

/** Header section - exported for reuse in edit mode */
export function FirewallApprovalHeader({ requestType }: { requestType: RequestType }) {
  const header = getHeaderContent(requestType)
  return (
    <div className="mb-3 flex-shrink-0">
      <div className="flex items-center gap-2 mb-0.5">
        {header.icon}
        <h1 className="text-sm font-bold">{header.title}</h1>
      </div>
      <p className="text-xs text-muted-foreground">{header.description}</p>
    </div>
  )
}

export interface FirewallApprovalCardProps {
  clientName: string
  toolName: string
  serverName: string
  argumentsPreview?: string
  isModelRequest?: boolean
  isGuardrailRequest?: boolean
  guardrailVerdicts?: SafetyVerdict[]
  guardrailDirection?: "request" | "response"
  guardrailActions?: CategoryActionRequired[]
  guardrailFlaggedText?: string
  /** If not provided, all buttons are disabled (demo mode) */
  onAction?: (action: ApprovalAction) => void
  onEdit?: () => void
  submitting?: boolean
  className?: string
}

/** Format a confidence value (0-1) as a percentage string */
/** Format a SafetyCategory for display (handles both string and Custom object variants) */
function formatCategory(category: string | Record<string, string>): string {
  if (typeof category === "string") return category.replace(/_/g, " ")
  if (typeof category === "object" && category !== null) {
    const value = Object.values(category)[0]
    return typeof value === "string" ? value.replace(/_/g, " ") : String(value)
  }
  return String(category)
}

function formatConfidence(confidence: number | null): string {
  if (confidence === null) return "N/A"
  return `${Math.round(confidence * 100)}%`
}

export function FirewallApprovalCard({
  clientName,
  toolName,
  serverName,
  argumentsPreview,
  isModelRequest,
  isGuardrailRequest,
  guardrailVerdicts,
  guardrailDirection,
  guardrailActions,
  guardrailFlaggedText,
  onAction,
  onEdit,
  submitting = false,
  className,
}: FirewallApprovalCardProps) {
  const requestType = getRequestType({
    server_name: serverName,
    tool_name: toolName,
    is_model_request: isModelRequest,
    is_guardrail_request: isGuardrailRequest,
  })
  const parsedArgs = parseArguments(argumentsPreview || "")
  const canEdit = requestType !== "marketplace" && requestType !== "guardrail"
  const disabled = !onAction || submitting

  return (
    <div className={className}>
      {/* Header */}
      <FirewallApprovalHeader requestType={requestType} />

      {/* Details Grid */}
      <div className="flex-1 overflow-auto">
        <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
          <span className="text-muted-foreground">Client:</span>
          <span className="font-medium truncate">{clientName}</span>

          {requestType === "guardrail" ? (
            <>
              <span className="text-muted-foreground">Direction:</span>
              <span className="font-medium capitalize">{guardrailDirection || "request"}</span>
              <span className="text-muted-foreground">Model:</span>
              <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">{toolName}</code>
            </>
          ) : requestType === "marketplace" ? (
            <>
              <span className="text-muted-foreground">Skill:</span>
              <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                {toolName}
              </code>
            </>
          ) : requestType === "skill" ? (
            <>
              <span className="text-muted-foreground">Skill:</span>
              <span className="truncate">{serverName}</span>
              <span className="text-muted-foreground">Action:</span>
              <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                {toolName.replace(/^skill_/, "").replace(/_/g, " ")}
              </code>
            </>
          ) : requestType === "model" ? (
            <>
              <span className="text-muted-foreground">Model:</span>
              <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                {toolName}
              </code>
              <span className="text-muted-foreground">Provider:</span>
              <span className="truncate">{serverName}</span>
            </>
          ) : (
            <>
              <span className="text-muted-foreground">Tool:</span>
              <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                {toolName}
              </code>
              <span className="text-muted-foreground">Server:</span>
              <span className="truncate">{serverName}</span>
            </>
          )}

          {/* Arguments inline (non-guardrail) */}
          {requestType !== "guardrail" && parsedArgs.map(({ key, value }) => (
            <span key={key} className="contents">
              <span className="text-muted-foreground">{key}:</span>
              <span className="font-mono truncate" title={value}>
                {value.length > 60 ? `${value.slice(0, 60)}...` : value}
              </span>
            </span>
          ))}
        </div>

        {/* Flagged text context */}
        {requestType === "guardrail" && guardrailFlaggedText && (
          <div className="mt-2 bg-muted/50 rounded px-2 py-1.5">
            <span className="text-[10px] font-semibold text-muted-foreground">Flagged Content</span>
            <p className="text-xs mt-0.5 whitespace-pre-wrap break-words max-h-24 overflow-auto font-mono leading-relaxed">
              {guardrailFlaggedText}
            </p>
          </div>
        )}

        {/* Safety verdicts - grouped by model */}
        {requestType === "guardrail" && guardrailVerdicts && guardrailVerdicts.length > 0 && (
          <div className="mt-2 space-y-2 max-h-48 overflow-auto">
            {guardrailVerdicts.map((verdict) => (
              <div key={verdict.model_id} className="bg-muted/50 rounded px-2 py-1.5 text-xs space-y-1">
                <div className="flex items-center justify-between">
                  <span className="font-semibold">{verdict.model_id}</span>
                  <div className="flex items-center gap-1.5">
                    <span className="text-muted-foreground text-[10px]">{verdict.check_duration_ms}ms</span>
                    <span className={`px-1.5 py-0.5 rounded text-[10px] font-bold ${verdict.is_safe ? "bg-emerald-500 text-white" : "bg-red-500 text-white"}`}>
                      {verdict.is_safe ? "SAFE" : "UNSAFE"}
                    </span>
                  </div>
                </div>
                {verdict.flagged_categories.length > 0 && (
                  <div className="space-y-0.5 pl-1">
                    {verdict.flagged_categories.map((cat, i) => (
                      <div key={i} className="flex items-center gap-1.5 text-[10px]">
                        <span className="text-red-500 font-medium">{formatCategory(cat.category)}</span>
                        <span className="text-muted-foreground">({cat.native_label})</span>
                        <span className="ml-auto font-mono">{formatConfidence(cat.confidence)}</span>
                      </div>
                    ))}
                  </div>
                )}
                {verdict.confidence !== null && (
                  <div className="text-[10px] text-muted-foreground">
                    Overall confidence: {formatConfidence(verdict.confidence)}
                  </div>
                )}
              </div>
            ))}

            {/* Actions required summary */}
            {guardrailActions && guardrailActions.length > 0 && (
              <div className="border-t border-border pt-1.5">
                <span className="text-[10px] font-semibold text-muted-foreground">Actions Required</span>
                <div className="space-y-0.5 mt-0.5">
                  {guardrailActions.map((act, i) => (
                    <div key={i} className="flex items-center gap-1.5 text-[10px]">
                      <span className={`px-1 py-0.5 rounded font-bold ${act.action === "allow" ? "bg-emerald-500/20 text-emerald-600" : act.action === "ask" ? "bg-amber-500/20 text-amber-600" : "bg-blue-500/20 text-blue-600"}`}>
                        {act.action.toUpperCase()}
                      </span>
                      <span className="font-medium">{formatCategory(act.category)}</span>
                      <span className="text-muted-foreground ml-auto">{act.model_id}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Action Buttons */}
      <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
        {/* Split button: Deny (main) + dropdown */}
        <div className="flex flex-1">
          <Button
            variant="destructive"
            className="flex-1 h-10 rounded-r-none font-bold"
            onClick={() => onAction?.("deny")}
            disabled={disabled}
          >
            Deny
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="destructive"
                className="h-10 px-2 rounded-l-none border-l border-red-700"
                disabled={disabled}
              >
                <ChevronDown className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {!isModelRequest && !isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("deny_session")}>
                  Deny for Session
                </DropdownMenuItem>
              )}
              {isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("deny_1_hour")}>
                  Deny for 1 Hour
                </DropdownMenuItem>
              )}
              {isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("block_categories")}>
                  Deny Categories Always
                </DropdownMenuItem>
              )}
              {!isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("deny_always")}>
                  Deny Always
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {/* Edit button - hidden for marketplace */}
        {canEdit && (
          <Button
            className="h-10 px-3 bg-amber-500 hover:bg-amber-600 text-white font-bold"
            onClick={onEdit}
            disabled={disabled}
          >
            <Pencil className="h-3.5 w-3.5 mr-1" />
            Modify
          </Button>
        )}

        {/* Split button: Allow Once (main) + dropdown */}
        <div className="flex flex-1">
          <Button
            className="flex-1 h-10 rounded-r-none bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
            onClick={() => onAction?.("allow_once")}
            disabled={disabled}
          >
            Allow Once
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                className="h-10 px-2 rounded-l-none border-l border-emerald-700 bg-emerald-600 hover:bg-emerald-700 text-white"
                disabled={disabled}
              >
                <ChevronDown className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {!isModelRequest && !isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("allow_session")}>
                  Allow for Session
                </DropdownMenuItem>
              )}
              {(isModelRequest || isGuardrailRequest) && (
                <DropdownMenuItem onClick={() => onAction?.("allow_1_hour")}>
                  Allow for 1 Hour
                </DropdownMenuItem>
              )}
              {isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("allow_categories")}>
                  Allow Always for Categories
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => onAction?.("allow_permanent")}>
                {isGuardrailRequest ? "Allow All Always for Client" : "Allow Always"}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  )
}
