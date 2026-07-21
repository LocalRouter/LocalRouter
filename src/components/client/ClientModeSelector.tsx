/**
 * ClientModeSelector - Shared component for selecting a client's access modes.
 *
 * The former single 4-way mode is split into two independent axes:
 *   - LLM: Off / Gateway / Inspect Proxy (passive) / Rewrite Proxy (active, WIP)
 *   - MCP: Off / Gateway / Via LLM
 *
 * Not every combination is legal, so options gray out contextually:
 *   - "MCP via LLM" needs the native LLM gateway (it can't inject tools into
 *     proxied traffic), so it's disabled whenever the LLM proxy is selected.
 *   - The LLM proxy modes are disabled while "MCP via LLM" is selected.
 *   - The active "Rewrite Proxy" is not implemented yet (permanently disabled).
 *   - At least one axis must stay enabled (the last "Off" is disabled).
 *
 * Used in both the client settings tab and the new-client wizard.
 */

import type { LlmMode, McpMode } from "@/types/tauri-commands"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import { EXPERIMENTAL } from "@/constants/features"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"

// ── Icons ───────────────────────────────────────────────────────────────

function OffIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="8" />
      <path d="M12 4v8" />
    </svg>
  )
}

function ArrowIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 12h14" />
      <path d="M14 7l5 5-5 5" />
    </svg>
  )
}

function ServerIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 12h9" />
      <path d="M10 7l5 5-5 5" />
      <rect x="17" y="8" width="4" height="8" rx="1" />
    </svg>
  )
}

function ViaLlmIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 8h13" />
      <path d="M14 5l4 3-4 3" />
      <path d="M11 8v8h7" />
      <path d="M15 13l3 3-3 3" />
    </svg>
  )
}

function InspectIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {/* magnifier over a path — passive inspection */}
      <path d="M3 12h6" />
      <circle cx="14" cy="12" r="4" />
      <path d="M17 15l4 4" />
    </svg>
  )
}

function RewriteIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {/* pencil over a path — active rewrite */}
      <path d="M3 12h7" />
      <path d="M13 15l6-6a2 2 0 0 0-3-3l-6 6v3z" />
    </svg>
  )
}

// ── Option definitions ──────────────────────────────────────────────────

type ModeOption<T> = {
  value: T
  label: string
  description: string
  Icon: React.ComponentType<{ className?: string }>
  experimentalKey?: keyof typeof EXPERIMENTAL
}

const LLM_OPTIONS: ModeOption<LlmMode>[] = [
  {
    value: "off",
    label: "Off",
    description: "No LLM access",
    Icon: OffIcon,
  },
  {
    value: "gateway",
    label: "Gateway",
    description: "LocalRouter routes LLM requests through its native API",
    Icon: ArrowIcon,
  },
  {
    value: "proxy_inspect",
    label: "Inspect Proxy",
    description: "Passive HTTPS proxy — inspect traffic in the monitor, no changes",
    Icon: InspectIcon,
  },
  {
    value: "proxy_rewrite",
    label: "Rewrite Proxy",
    description: "Active HTTPS proxy — inspect and rewrite requests (coming soon)",
    Icon: RewriteIcon,
  },
]

const MCP_OPTIONS: ModeOption<McpMode>[] = [
  {
    value: "off",
    label: "Off",
    description: "No MCP access",
    Icon: OffIcon,
  },
  {
    value: "gateway",
    label: "Gateway",
    description: "Direct MCP proxy — the client speaks MCP to LocalRouter",
    Icon: ServerIcon,
  },
  {
    value: "via_llm",
    label: "Via LLM",
    description: "MCP tools injected into LLM requests, executed server-side",
    Icon: ViaLlmIcon,
    experimentalKey: "mcpViaLlm",
  },
]

function isLlmProxy(mode: LlmMode): boolean {
  return mode === "proxy_inspect" || mode === "proxy_rewrite"
}

// ── Component ───────────────────────────────────────────────────────────

interface ClientModeSelectorProps {
  llmMode: LlmMode
  mcpMode: McpMode
  onLlmModeChange: (mode: LlmMode) => void
  onMcpModeChange: (mode: McpMode) => void
  template?: ClientTemplate | null
}

/** Availability (enabled + reason) for a single LLM option given the current MCP state. */
function llmOptionState(
  value: LlmMode,
  mcpMode: McpMode,
  template: ClientTemplate | null | undefined,
): { allowed: boolean; reason?: string } {
  // The active rewrite proxy is not implemented yet.
  if (value === "proxy_rewrite") return { allowed: false, reason: "Coming soon" }

  // Template must support LLM for any non-off LLM mode.
  if (value !== "off" && template && !template.supportsLlm) {
    return { allowed: false, reason: `Not supported by ${template.name}` }
  }

  // Proxy modes are incompatible with MCP via LLM.
  if (isLlmProxy(value) && mcpMode === "via_llm") {
    return { allowed: false, reason: "Incompatible with MCP via LLM" }
  }

  // Don't allow turning LLM off when MCP is already off (nothing left enabled).
  if (value === "off" && mcpMode === "off") {
    return { allowed: false, reason: "Enable at least one of LLM or MCP" }
  }

  return { allowed: true }
}

/** Availability (enabled + reason) for a single MCP option given the current LLM state. */
function mcpOptionState(
  value: McpMode,
  llmMode: LlmMode,
  template: ClientTemplate | null | undefined,
): { allowed: boolean; reason?: string } {
  // Template must support MCP for any non-off MCP mode.
  if (value !== "off" && template && !template.supportsMcp) {
    return { allowed: false, reason: `Not supported by ${template.name}` }
  }

  // MCP via LLM needs the native LLM gateway.
  if (value === "via_llm" && llmMode !== "gateway") {
    return { allowed: false, reason: "Requires LLM gateway mode" }
  }

  // Don't allow turning MCP off when LLM is already off.
  if (value === "off" && llmMode === "off") {
    return { allowed: false, reason: "Enable at least one of LLM or MCP" }
  }

  return { allowed: true }
}

function ModeOptionButton<T extends string>({
  option,
  selected,
  allowed,
  reason,
  onSelect,
}: {
  option: ModeOption<T>
  selected: boolean
  allowed: boolean
  reason?: string
  onSelect: (value: T) => void
}) {
  return (
    <button
      type="button"
      onClick={() => allowed && onSelect(option.value)}
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
          {option.experimentalKey && EXPERIMENTAL[option.experimentalKey] && <ExperimentalBadge />}
        </p>
        <p className="text-xs text-muted-foreground mt-0.5">
          {option.description}
          {!allowed && reason && ` (${reason})`}
        </p>
      </div>
    </button>
  )
}

export function ClientModeSelector({
  llmMode,
  mcpMode,
  onLlmModeChange,
  onMcpModeChange,
  template,
}: ClientModeSelectorProps) {
  return (
    <div className="grid gap-5">
      <section className="grid gap-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">LLM</p>
        {LLM_OPTIONS.map((option) => {
          const { allowed, reason } = llmOptionState(option.value, mcpMode, template)
          return (
            <ModeOptionButton
              key={option.value}
              option={option}
              selected={llmMode === option.value}
              allowed={allowed}
              reason={reason}
              onSelect={onLlmModeChange}
            />
          )
        })}
      </section>

      <section className="grid gap-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">MCP</p>
        {MCP_OPTIONS.map((option) => {
          const { allowed, reason } = mcpOptionState(option.value, llmMode, template)
          return (
            <ModeOptionButton
              key={option.value}
              option={option}
              selected={mcpMode === option.value}
              allowed={allowed}
              reason={reason}
              onSelect={onMcpModeChange}
            />
          )
        })}
      </section>
    </div>
  )
}
