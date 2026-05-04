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
import { Coins, Bot } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { categoryActionLabel } from "@/components/permissions/CategoryActionButton"
import type { SafetyVerdict, CategoryActionRequired, SecretFindingSummary } from "@/types/tauri-commands"

export type ApprovalAction = "deny" | "deny_session" | "deny_always" | "block_categories" | "allow_once" | "allow_session" | "allow_1_minute" | "allow_1_hour" | "allow_permanent" | "allow_categories" | "deny_1_hour" | "disable_client"

export type RequestType = "marketplace" | "skill" | "model" | "tool" | "guardrail" | "free_tier_fallback" | "auto_router" | "secret_scan"

/** Determine request type from server/tool names */
export function getRequestType(details: {
  server_name: string
  tool_name: string
  is_model_request?: boolean
  is_guardrail_request?: boolean
  is_free_tier_fallback?: boolean
  is_auto_router_request?: boolean
  is_secret_scan_request?: boolean
}): RequestType {
  if (details.is_secret_scan_request) {
    return "secret_scan"
  }
  if (details.is_auto_router_request) {
    return "auto_router"
  }
  if (details.is_free_tier_fallback) {
    return "free_tier_fallback"
  }
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
    details.tool_name.startsWith("Skill") ||
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
        description: "Review this package before installing",
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
    case "free_tier_fallback":
      return {
        icon: <Coins className="h-5 w-5 text-amber-500" />,
        title: "Free Tier Exhausted",
        description: "All free-tier models are at capacity. Proceed with paid models?",
      }
    case "guardrail":
      return {
        icon: <FEATURES.guardrails.icon className={`h-5 w-5 ${FEATURES.guardrails.color}`} />,
        title: "GuardRail Alert",
        description: "Content flagged by guardrail rules",
      }
    case "auto_router":
      return {
        icon: <Bot className="h-5 w-5 text-emerald-500" />,
        title: "Auto Router",
        description: "Auto-routing will select a model for this request",
      }
    case "secret_scan":
      return {
        icon: <FEATURES.secretScanning.icon className={`h-5 w-5 ${FEATURES.secretScanning.color}`} />,
        title: "Secrets Detected",
        description: "Potential secrets found in outbound request",
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

/** Marketplace listing metadata for display in the approval popup */
export interface MarketplaceListingInfo {
  name: string
  description?: string | null
  homepage?: string | null
  vendor?: string | null
  author?: string | null
  source_label?: string | null
  source_repo?: string | null
  install_type?: "mcp_server" | "skill"
}

export interface FirewallApprovalCardProps {
  clientName: string
  toolName: string
  serverName: string
  argumentsPreview?: string
  isModelRequest?: boolean
  isGuardrailRequest?: boolean
  isFreeTierFallback?: boolean
  isAutoRouterRequest?: boolean
  isSecretScanRequest?: boolean
  secretScanFindings?: SecretFindingSummary[]
  secretScanDurationMs?: number
  guardrailVerdicts?: SafetyVerdict[]
  guardrailDirection?: "request" | "response"
  guardrailActions?: CategoryActionRequired[]
  guardrailFlaggedText?: string
  /** Marketplace listing details for install popups */
  marketplaceListing?: MarketplaceListingInfo | null
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
  isFreeTierFallback,
  isAutoRouterRequest,
  isSecretScanRequest,
  secretScanFindings,
  secretScanDurationMs,
  guardrailVerdicts,
  guardrailDirection,
  guardrailActions,
  guardrailFlaggedText,
  marketplaceListing,
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
    is_free_tier_fallback: isFreeTierFallback,
    is_auto_router_request: isAutoRouterRequest,
    is_secret_scan_request: isSecretScanRequest,
  })
  const parsedArgs = parseArguments(argumentsPreview || "")
  const canEdit = requestType !== "marketplace" && requestType !== "guardrail" && requestType !== "free_tier_fallback" && requestType !== "secret_scan"
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
              {marketplaceListing ? (
                <>
                  <span className="text-muted-foreground">Name:</span>
                  <span className="font-medium">{marketplaceListing.name}</span>
                  {marketplaceListing.description && (
                    <>
                      <span className="text-muted-foreground">Description:</span>
                      <span className="truncate" title={marketplaceListing.description}>{marketplaceListing.description}</span>
                    </>
                  )}
                  <span className="text-muted-foreground">Type:</span>
                  <span>{marketplaceListing.install_type === "mcp_server" ? "MCP Server" : "Skill"}</span>
                  {(marketplaceListing.vendor || marketplaceListing.author) && (
                    <>
                      <span className="text-muted-foreground">{marketplaceListing.install_type === "mcp_server" ? "Vendor:" : "Author:"}</span>
                      <span>{marketplaceListing.vendor || marketplaceListing.author}</span>
                    </>
                  )}
                  {marketplaceListing.source_label && (
                    <>
                      <span className="text-muted-foreground">Source:</span>
                      <span>{marketplaceListing.source_label}</span>
                    </>
                  )}
                </>
              ) : (
                <>
                  <span className="text-muted-foreground">Tool:</span>
                  <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">
                    {toolName}
                  </code>
                </>
              )}
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
          ) : requestType === "auto_router" ? (
            <>
              <span className="text-muted-foreground">Mode:</span>
              <span className="font-medium">Auto Model Selection</span>
              {argumentsPreview && (
                <>
                  <span className="text-muted-foreground">Models:</span>
                  <span className="font-mono truncate text-[11px]" title={argumentsPreview}>{argumentsPreview}</span>
                </>
              )}
            </>
          ) : requestType === "secret_scan" ? (
            <>
              <span className="text-muted-foreground">Model:</span>
              <span className="truncate">{toolName}</span>
              {secretScanDurationMs !== undefined && (
                <>
                  <span className="text-muted-foreground">Scan time:</span>
                  <span>{secretScanDurationMs}ms</span>
                </>
              )}
              <span className="text-muted-foreground">Findings:</span>
              <span className="font-medium text-orange-500">{secretScanFindings?.length ?? 0} secret(s) detected</span>
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

          {/* Marketplace source link */}
          {requestType === "marketplace" && marketplaceListing && (marketplaceListing.homepage || marketplaceListing.source_repo) && (
            <>
              <span className="text-muted-foreground">Source:</span>
              <button
                className="text-blue-500 hover:text-blue-400 hover:underline text-left truncate font-mono text-[11px]"
                title={marketplaceListing.homepage || marketplaceListing.source_repo || ""}
                onClick={() => window.open(marketplaceListing.homepage || marketplaceListing.source_repo || "", "_blank")}
              >
                {(marketplaceListing.homepage || marketplaceListing.source_repo || "").replace(/^https?:\/\//, "")}
              </button>
            </>
          )}

          {/* Arguments inline (non-guardrail, non-marketplace with listing) */}
          {requestType !== "guardrail" && requestType !== "secret_scan" && !(requestType === "marketplace" && marketplaceListing) && parsedArgs.map(({ key, value }) => (
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
                  <span className="font-semibold">{verdict.model_label || verdict.model_id}</span>
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
                      <span className={`px-1 py-0.5 rounded font-bold ${act.action === "allow" ? "bg-emerald-500/20 text-emerald-600" : act.action === "block" ? "bg-red-600/20 text-red-600" : act.action === "ask" ? "bg-amber-500/20 text-amber-600" : "bg-blue-500/20 text-blue-600"}`}>
                        {categoryActionLabel(act.action).toUpperCase()}
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

        {/* Secret scan findings */}
        {requestType === "secret_scan" && secretScanFindings && secretScanFindings.length > 0 && (
          <div className="mt-2 space-y-1.5 max-h-64 overflow-auto">
            <span className="text-[10px] font-semibold text-muted-foreground">Detected Secrets</span>
            {secretScanFindings.map((finding, i) => (
              <div key={i} className="bg-muted/50 rounded px-2 py-1.5 text-xs space-y-0.5">
                <div className="flex items-center justify-between">
                  <span className="font-semibold">{finding.rule_description}</span>
                  <span className="px-1.5 py-0.5 rounded text-[10px] font-bold bg-orange-500/20 text-orange-600">
                    {finding.category.replace(/_/g, " ")}
                  </span>
                </div>
                <div className="font-mono text-[10px] bg-background/50 rounded px-1 py-0.5 truncate" title={finding.matched_text}>
                  {finding.matched_text}
                </div>
                <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
                  <span>Entropy: <span className="font-mono font-medium text-foreground">{finding.entropy.toFixed(2)}</span></span>
                  <span className="ml-auto font-mono">{finding.rule_id}</span>
                </div>
              </div>
            ))}
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
            {requestType === "secret_scan" ? "Block" : "Deny Once"}
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
              {!isModelRequest && !isGuardrailRequest && !isFreeTierFallback && !isSecretScanRequest && !isAutoRouterRequest && (
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
              {isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("disable_client")}>
                  Disable Client
                </DropdownMenuItem>
              )}
              {isSecretScanRequest && (
                <DropdownMenuItem onClick={() => onAction?.("disable_client")}>
                  Disable Client
                </DropdownMenuItem>
              )}
              {isSecretScanRequest && (
                <DropdownMenuItem onClick={() => onAction?.("deny_always")}>
                  Disable Scan for Client
                </DropdownMenuItem>
              )}
              {!isGuardrailRequest && !isSecretScanRequest && (
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
            {requestType === "secret_scan" ? "Allow" : "Allow Once"}
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
              {!isModelRequest && !isGuardrailRequest && !isFreeTierFallback && !isSecretScanRequest && !isAutoRouterRequest && (
                <DropdownMenuItem onClick={() => onAction?.("allow_session")}>
                  Allow for Session
                </DropdownMenuItem>
              )}
              {(isModelRequest || isGuardrailRequest || isFreeTierFallback || isAutoRouterRequest) && (
                <DropdownMenuItem onClick={() => onAction?.("allow_1_minute")}>
                  Allow for 1 Minute
                </DropdownMenuItem>
              )}
              {(isModelRequest || isGuardrailRequest || isFreeTierFallback || isAutoRouterRequest || isSecretScanRequest) && (
                <DropdownMenuItem onClick={() => onAction?.("allow_1_hour")}>
                  Allow for 1 Hour
                </DropdownMenuItem>
              )}
              {isGuardrailRequest && (
                <DropdownMenuItem onClick={() => onAction?.("allow_categories")}>
                  Allow Always for Categories
                </DropdownMenuItem>
              )}
              {!isSecretScanRequest && (
                <DropdownMenuItem onClick={() => onAction?.("allow_permanent")}>
                  {isGuardrailRequest ? "Allow All Always for Client" : "Allow Always"}
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  )
}
