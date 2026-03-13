import type { LucideIcon } from "lucide-react"
import { Shield, Wrench, Minimize2, Cpu, BookText, Database, ArrowRight, Monitor, Cloud, Server } from "lucide-react"
import { OPTIMIZE_COLORS } from "./constants"

interface PillProps {
  icon: LucideIcon
  label: string
  colorClass: string
  borderClass: string
}

function Pill({ icon: Icon, label, colorClass, borderClass }: PillProps) {
  return (
    <div className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-card border ${borderClass}`}>
      <Icon className={`h-3.5 w-3.5 shrink-0 ${colorClass}`} />
      <span className="text-xs font-medium whitespace-nowrap">{label}</span>
    </div>
  )
}

/** Pills row above, single long arrow below */
function FlowLine({ direction, children }: { direction: "right" | "left", children: React.ReactNode }) {
  const isRight = direction === "right"
  return (
    <div className="space-y-1">
      {/* Pills row */}
      <div className="flex items-center gap-1.5">
        {children}
      </div>
      {/* Arrow row */}
      <div className="relative h-4 flex items-center">
        <div className="absolute inset-x-0 top-1/2 -translate-y-1/2 h-px bg-muted-foreground/20" />
        <span className={`text-[10px] text-muted-foreground/50 shrink-0 relative bg-card ${isRight ? "pr-1" : "ml-auto pl-1"}`}>
          {isRight ? "request →" : "← response"}
        </span>
      </div>
    </div>
  )
}

export function OptimizeDiagram() {
  return (
    <div className="border rounded-lg bg-card overflow-hidden">
      <div className="grid grid-cols-[auto_auto_1fr_auto_auto] items-stretch">
        {/* ── Column 1: Client ── */}
        <div className="row-span-2 flex flex-col items-center justify-center px-4 py-4 border-r bg-muted/30">
          <Monitor className="h-5 w-5 mb-1.5 text-muted-foreground" />
          <span className="text-sm font-semibold">Client</span>
          <span className="text-[11px] text-muted-foreground">Claude Code,</span>
          <span className="text-[11px] text-muted-foreground">Cursor, etc.</span>
        </div>

        {/* ── Column 2: Left arrows ── */}
        <div className="flex flex-col items-center justify-center px-1.5 border-r border-dashed">
          <span className="text-[10px] text-muted-foreground font-medium mb-0.5">LLM</span>
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/50" />
        </div>
        {/* ── Column 3: LLM Pipeline ── */}
        <div className="px-3 py-3 border-b space-y-3">
          <div className="text-[11px] uppercase tracking-wider text-muted-foreground font-medium">LLM Pipeline</div>
          <FlowLine direction="right">
            <Pill icon={Shield} label="GuardRails" colorClass={OPTIMIZE_COLORS.guardrails} borderClass="border-red-500/30" />
            <Pill icon={Minimize2} label="Compress" colorClass={OPTIMIZE_COLORS.compression} borderClass="border-blue-500/30" />
            <Pill icon={Cpu} label="Strong/Weak" colorClass={OPTIMIZE_COLORS.routing} borderClass="border-purple-500/30" />
          </FlowLine>
          <FlowLine direction="left">
            <Pill icon={Wrench} label="JSON Repair" colorClass={OPTIMIZE_COLORS.jsonRepair} borderClass="border-amber-500/30" />
          </FlowLine>
        </div>
        {/* ── Column 4: Right arrows (LLM) ── */}
        <div className="flex flex-col items-center justify-center px-1.5 border-l border-dashed border-b">
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/50" />
        </div>
        {/* ── Column 5: LLM Provider ── */}
        <div className="row-span-1 flex flex-col items-center justify-center px-4 py-4 border-l bg-muted/30 border-b">
          <Cloud className="h-5 w-5 mb-1.5 text-muted-foreground" />
          <span className="text-sm font-semibold">LLM Provider</span>
          <span className="text-[11px] text-muted-foreground">OpenAI, Anthropic…</span>
        </div>

        {/* ── Row 2: MCP ── */}
        <div className="flex flex-col items-center justify-center px-1.5 border-r border-dashed">
          <span className="text-[10px] text-muted-foreground font-medium mb-0.5">MCP</span>
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/50" />
        </div>
        {/* Column 3: MCP Pipeline */}
        <div className="px-3 py-3 space-y-3">
          <div className="text-[11px] uppercase tracking-wider text-muted-foreground font-medium">MCP Pipeline</div>
          <FlowLine direction="right">
            <Pill icon={BookText} label="Catalog Compress" colorClass={OPTIMIZE_COLORS.catalogCompression} borderClass="border-teal-500/30" />
          </FlowLine>
          <FlowLine direction="left">
            <Pill icon={Database} label="Response RAG" colorClass={OPTIMIZE_COLORS.responseRag} borderClass="border-emerald-500/30" />
          </FlowLine>
        </div>
        {/* Column 4: Right arrows (MCP) */}
        <div className="flex flex-col items-center justify-center px-1.5 border-l border-dashed">
          <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/50" />
        </div>
        {/* Column 5: MCP Servers */}
        <div className="row-span-1 flex flex-col items-center justify-center px-4 py-4 border-l bg-muted/30">
          <Server className="h-5 w-5 mb-1.5 text-muted-foreground" />
          <span className="text-sm font-semibold">MCP Servers</span>
          <span className="text-[11px] text-muted-foreground">External tools</span>
        </div>
      </div>
    </div>
  )
}
