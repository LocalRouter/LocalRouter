/**
 * ClientModeSelector - Shared component for selecting client access mode.
 *
 * Used in both the client settings tab and the new client wizard.
 * Displays four modes with custom arrow icons:
 *   - Both: two parallel arrows (LLM + MCP independent paths)
 *   - Both via LLM: arrow with sub-arrow branching from it (MCP through LLM)
 *   - LLM Only: single arrow
 *   - MCP Only: single arrow with server node
 */

import type { ClientMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"

// ── Custom arrow icons ──────────────────────────────────────────────────

function LlmOnlyIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 12h14" />
      <path d="M14 7l5 5-5 5" />
    </svg>
  )
}

function McpOnlyIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 12h9" />
      <path d="M10 7l5 5-5 5" />
      <rect x="17" y="8" width="4" height="8" rx="1" />
    </svg>
  )
}

function BothIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {/* Top arrow */}
      <path d="M5 8h12" />
      <path d="M13 5l4 3-4 3" />
      {/* Bottom arrow */}
      <path d="M5 16h12" />
      <path d="M13 13l4 3-4 3" />
    </svg>
  )
}

function BothViaLlmIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {/* Main arrow */}
      <path d="M5 8h13" />
      <path d="M14 5l4 3-4 3" />
      {/* Sub-arrow branching from main arrow */}
      <path d="M11 8v8h7" />
      <path d="M15 13l3 3-3 3" />
    </svg>
  )
}

// ── Mode definitions ────────────────────────────────────────────────────

const MODE_OPTIONS: {
  value: ClientMode
  label: string
  description: string
  Icon: React.ComponentType<{ className?: string }>
  experimental?: boolean
}[] = [
  {
    value: "both",
    label: "Both",
    description: "Full access to LLM routing and MCP servers",
    Icon: BothIcon,
  },
  {
    value: "mcp_via_llm",
    label: "Both via LLM",
    description: "MCP tools injected into LLM requests and executed server-side",
    Icon: BothViaLlmIcon,
    experimental: true,
  },
  {
    value: "llm_only",
    label: "LLM Only",
    description: "Only LLM routing (hides MCP/Skills tabs)",
    Icon: LlmOnlyIcon,
  },
  {
    value: "mcp_only",
    label: "MCP Only",
    description: "Only MCP proxy (hides Models tab)",
    Icon: McpOnlyIcon,
  },
]

// ── Component ───────────────────────────────────────────────────────────

interface ClientModeSelectorProps {
  mode: ClientMode
  onModeChange: (mode: ClientMode) => void
  template?: ClientTemplate | null
}

/** Check if a mode is allowed by the current template */
function isModeAllowed(mode: ClientMode, template: ClientTemplate | null | undefined): boolean {
  if (!template) return true
  if (mode === "both") return template.supportsLlm && template.supportsMcp
  if (mode === "llm_only") return template.supportsLlm
  if (mode === "mcp_only") return template.supportsMcp
  // MCP via LLM only requires LLM support — MCP tools are server-side
  if (mode === "mcp_via_llm") return template.supportsLlm
  return true
}

export function ClientModeSelector({ mode, onModeChange, template }: ClientModeSelectorProps) {
  return (
    <div className="grid gap-2">
      {MODE_OPTIONS.map((option) => {
        const allowed = isModeAllowed(option.value, template)
        const selected = mode === option.value
        return (
          <button
            key={option.value}
            type="button"
            onClick={() => allowed && onModeChange(option.value)}
            disabled={!allowed}
            className={`flex items-center gap-3 p-3 rounded-lg border text-left transition-colors
              ${selected ? "border-primary bg-accent" : allowed ? "border-muted hover:border-primary/50" : "border-muted"}
              ${!allowed ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}
              focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2`}
          >
            <div className={`shrink-0 ${selected ? "text-primary" : "text-muted-foreground"}`}>
              <option.Icon className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="text-sm font-medium flex items-center gap-2">
                {option.label}
                {option.experimental && (
                  <span className="text-[10px] font-medium px-1.5 py-0.5 rounded-full bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400">
                    Experimental
                  </span>
                )}
              </p>
              <p className="text-xs text-muted-foreground mt-0.5">
                {option.description}
                {!allowed && template && ` (not supported by ${template.name})`}
              </p>
            </div>
          </button>
        )
      })}
    </div>
  )
}
