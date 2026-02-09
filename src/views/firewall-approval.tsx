import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { ChevronDown } from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { ProvidersIcon, McpIcon, SkillsIcon, StoreIcon } from "@/components/icons/category-icons"

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

export function FirewallApproval() {
  const [details, setDetails] = useState<ApprovalDetails | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [remainingSeconds, setRemainingSeconds] = useState<number>(0)
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null)

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
        const remaining = Math.max(0, result.timeout_seconds - result.created_at_secs_ago)
        setRemainingSeconds(remaining)
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
    if (!details || remainingSeconds <= 0) return

    timerRef.current = setInterval(() => {
      setRemainingSeconds((prev) => {
        if (prev <= 1) {
          if (timerRef.current) clearInterval(timerRef.current)
          getCurrentWebviewWindow().close()
          return 0
        }
        return prev - 1
      })
    }, 1000)

    return () => {
      if (timerRef.current) clearInterval(timerRef.current)
    }
  }, [details])

  const handleAction = async (action: ApprovalAction) => {
    if (!details) return
    setSubmitting(true)
    try {
      await invoke("submit_firewall_approval", {
        requestId: details.request_id,
        action,
      })
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
  const progressPercent = details.timeout_seconds > 0
    ? (remainingSeconds / details.timeout_seconds) * 100
    : 0

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

  return (
    <div className="flex flex-col h-screen bg-background overflow-hidden">
      {/* Timeout Progress Bar */}
      <div className="h-1 bg-muted w-full flex-shrink-0">
        <div
          className="h-full bg-amber-500 transition-all duration-1000 ease-linear"
          style={{ width: `${progressPercent}%` }}
        />
      </div>

      <div className="flex flex-col flex-1 p-4">
        {/* Header */}
        <div className="mb-3 flex-shrink-0">
          <div className="flex items-center gap-2 mb-0.5">
            {header.icon}
            <h1 className="text-sm font-bold">{header.title}</h1>
          </div>
          <p className="text-xs text-muted-foreground">{header.description}</p>
        </div>

        {/* Details - all inline */}
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
              <>
                <span key={`${key}-label`} className="text-muted-foreground">{key}:</span>
                <span key={`${key}-value`} className="font-mono truncate" title={value}>
                  {value.length > 60 ? `${value.slice(0, 60)}...` : value}
                </span>
              </>
            ))}
          </div>
        </div>

        {/* Action Buttons */}
        <div className="flex gap-2 pt-3 mt-auto flex-shrink-0">
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
        </div>
      </div>
    </div>
  )
}
