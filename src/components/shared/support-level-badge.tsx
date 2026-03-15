import { CheckCircle2, Circle, MinusCircle, ArrowLeftRight, CircleDashed } from "lucide-react"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { SupportLevel } from "@/types/tauri-commands"

const SUPPORT_CONFIG: Record<SupportLevel, {
  icon: typeof CheckCircle2
  label: string
  className: string
  shortLabel: string
}> = {
  supported: {
    icon: CheckCircle2,
    label: "Supported",
    className: "text-green-600 dark:text-green-400",
    shortLabel: "\u2713",
  },
  partial: {
    icon: Circle,
    label: "Partial",
    className: "text-yellow-600 dark:text-yellow-400",
    shortLabel: "P",
  },
  translated: {
    icon: ArrowLeftRight,
    label: "Via Translation",
    className: "text-blue-600 dark:text-blue-400",
    shortLabel: "\u2713*",
  },
  not_supported: {
    icon: MinusCircle,
    label: "Not Supported",
    className: "text-muted-foreground/50",
    shortLabel: "\u2014",
  },
  not_implemented: {
    icon: CircleDashed,
    label: "Not Yet Implemented",
    className: "text-muted-foreground/40",
    shortLabel: "\u2014",
  },
}

interface SupportLevelBadgeProps {
  level: SupportLevel
  notes?: string | null
  compact?: boolean
}

export function SupportLevelBadge({ level, notes, compact = false }: SupportLevelBadgeProps) {
  const config = SUPPORT_CONFIG[level]
  const Icon = config.icon

  const badge = compact ? (
    <span className={`text-xs font-medium ${config.className}`}>
      {config.shortLabel}
    </span>
  ) : (
    <span className={`inline-flex items-center gap-1 text-xs ${config.className}`}>
      <Icon className="h-3 w-3" />
      <span>{config.label}</span>
    </span>
  )

  if (notes) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="cursor-help">{badge}</span>
        </TooltipTrigger>
        <TooltipContent side="top" className="max-w-xs">
          <p className="text-xs">{notes}</p>
        </TooltipContent>
      </Tooltip>
    )
  }

  return badge
}
