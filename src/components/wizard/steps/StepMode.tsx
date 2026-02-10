/**
 * StepMode - Client mode selection step for the client creation wizard.
 *
 * Three radio cards: LLM Routing, MCP Proxy, Both.
 * Pre-selects the template's defaultMode and disables unsupported options.
 */

import type { ClientMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import { Cpu, Terminal, Layers } from "lucide-react"

interface StepModeProps {
  mode: ClientMode
  onChange: (mode: ClientMode) => void
  template: ClientTemplate | null
}

const MODE_OPTIONS: {
  value: ClientMode
  label: string
  description: string
  icon: React.ReactNode
  requiresLlm: boolean
  requiresMcp: boolean
}[] = [
  {
    value: "both",
    label: "Both",
    description: "Full access to LLM routing and MCP servers.",
    icon: <Layers className="h-5 w-5" />,
    requiresLlm: true,
    requiresMcp: true,
  },
  {
    value: "llm_only",
    label: "LLM Routing",
    description: "Route LLM requests through LocalRouter.",
    icon: <Cpu className="h-5 w-5" />,
    requiresLlm: true,
    requiresMcp: false,
  },
  {
    value: "mcp_only",
    label: "MCP Proxy",
    description: "Use LocalRouter's MCP servers and skills.",
    icon: <Terminal className="h-5 w-5" />,
    requiresLlm: false,
    requiresMcp: true,
  },
]

export function StepMode({ mode, onChange, template }: StepModeProps) {
  const isDisabled = (option: typeof MODE_OPTIONS[number]) => {
    if (!template) return false
    if (option.requiresLlm && !template.supportsLlm) return true
    if (option.requiresMcp && !template.supportsMcp) return true
    return false
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Choose what this client can access through LocalRouter.
      </p>
      <div className="grid gap-3">
        {MODE_OPTIONS.map((option) => {
          const disabled = isDisabled(option)
          const selected = mode === option.value
          return (
            <button
              key={option.value}
              onClick={() => !disabled && onChange(option.value)}
              disabled={disabled}
              className={`flex items-start gap-4 p-4 rounded-lg border-2 text-left transition-colors
                ${selected ? "border-primary bg-accent" : "border-muted hover:border-primary/50"}
                ${disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}
                focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2`}
            >
              <div className={`mt-0.5 ${selected ? "text-primary" : "text-muted-foreground"}`}>
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
