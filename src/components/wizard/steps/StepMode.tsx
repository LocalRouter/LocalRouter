/**
 * StepMode - Client mode selection step for the client creation wizard.
 *
 * Two checkboxes: LLM Routing, MCP Proxy.
 * Derives ClientMode from the combination: both checked → "both",
 * only LLM → "llm_only", only MCP → "mcp_only".
 * Pre-selects based on the template's defaultMode and disables unsupported options.
 */

import type { ClientMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import { Cpu, Terminal, Check } from "lucide-react"

interface StepModeProps {
  mode: ClientMode
  onChange: (mode: ClientMode) => void
  template: ClientTemplate | null
}

const CHECKBOX_OPTIONS: {
  key: "llm" | "mcp"
  label: string
  description: string
  icon: React.ReactNode
}[] = [
  {
    key: "llm",
    label: "LLM Routing",
    description: "Route LLM requests through LocalRouter.",
    icon: <Cpu className="h-5 w-5" />,
  },
  {
    key: "mcp",
    label: "MCP Proxy",
    description: "Use LocalRouter's MCP servers and skills.",
    icon: <Terminal className="h-5 w-5" />,
  },
]

function deriveMode(llm: boolean, mcp: boolean): ClientMode {
  if (llm && mcp) return "both"
  if (llm) return "llm_only"
  return "mcp_only"
}

export function StepMode({ mode, onChange, template }: StepModeProps) {
  const llmChecked = mode === "both" || mode === "llm_only"
  const mcpChecked = mode === "both" || mode === "mcp_only"

  const isDisabled = (key: "llm" | "mcp") => {
    if (!template) return false
    if (key === "llm" && !template.supportsLlm) return true
    if (key === "mcp" && !template.supportsMcp) return true
    return false
  }

  const handleToggle = (key: "llm" | "mcp") => {
    let newLlm = llmChecked
    let newMcp = mcpChecked
    if (key === "llm") newLlm = !newLlm
    else newMcp = !newMcp
    // At least one must be checked
    if (!newLlm && !newMcp) return
    onChange(deriveMode(newLlm, newMcp))
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Choose what this client can access through LocalRouter.
      </p>
      <div className="grid gap-3">
        {CHECKBOX_OPTIONS.map((option) => {
          const checked = option.key === "llm" ? llmChecked : mcpChecked
          const disabled = isDisabled(option.key)
          return (
            <button
              key={option.key}
              onClick={() => !disabled && handleToggle(option.key)}
              disabled={disabled}
              className={`flex items-start gap-4 p-4 rounded-lg border-2 text-left transition-colors
                ${checked ? "border-primary bg-accent" : "border-muted hover:border-primary/50"}
                ${disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}
                focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2`}
            >
              <div className={`mt-1 flex h-5 w-5 shrink-0 items-center justify-center rounded border-2 transition-colors
                ${checked ? "border-primary bg-primary text-primary-foreground" : "border-muted-foreground/40"}`}>
                {checked && <Check className="h-3.5 w-3.5" strokeWidth={3} />}
              </div>
              <div className={`mt-0.5 ${checked ? "text-primary" : "text-muted-foreground"}`}>
                {option.icon}
              </div>
              <div>
                <p className="font-medium text-sm">{option.label}</p>
                <p className="text-xs text-muted-foreground mt-0.5">{option.description}</p>
                {disabled && (
                  <p className="text-xs text-destructive mt-1">Not supported by this application</p>
                )}
              </div>
            </button>
          )
        })}
      </div>
    </div>
  )
}
