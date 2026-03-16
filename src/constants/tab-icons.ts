import type { LucideIcon } from "lucide-react"
import {
  Info, PlayCircle, Settings, LayoutDashboard, Plug, MessageSquare,
  Cable, Blocks, Sparkles, Palette, HeartPulse,
  ScrollText, Download, FileCheck, Server, Cpu, GitCompare,
  Search, Bot, History, Wand2, Store, Users, Coins, Brain,
} from "lucide-react"
import { FEATURES } from "./features"

export const TAB_ICON_CLASS = "h-3.5 w-3.5 mr-1"

export const TAB_ICONS = {
  info: Info,
  tryItOut: PlayCircle,
  settings: Settings,
  overview: LayoutDashboard,
  connect: Plug,
  llm: MessageSquare,
  mcp: Cable,
  mcpAndSkill: Blocks,
  strongWeak: FEATURES.routing.icon,
  guardrails: FEATURES.guardrails.icon,
  optimize: Sparkles,
  appearance: Palette,
  healthChecks: HeartPulse,
  logs: ScrollText,
  updates: Download,
  licenses: FileCheck,
  providers: Server,
  allModels: Cpu,
  compatibility: GitCompare,
  browse: Search,
  viaMcp: Cable,
  agents: Bot,
  sessions: History,
  skills: Wand2,
  codingAgents: Bot,
  marketplace: Store,
  models: Cpu,
  mcpServers: Server,
  model: Cpu,
  client: Users,
  freeTier: Coins,
  memory: Brain,
} as const satisfies Record<string, LucideIcon>
