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
  description: string
  className: string
  shortLabel: string
}> = {
  supported: {
    icon: CheckCircle2,
    label: "Supported",
    description: "Full native support",
    className: "text-green-600 dark:text-green-400",
    shortLabel: "\u2713",
  },
  partial: {
    icon: Circle,
    label: "Partial",
    description: "Only some models or configurations",
    className: "text-yellow-600 dark:text-yellow-400",
    shortLabel: "P",
  },
  translated: {
    icon: ArrowLeftRight,
    label: "Via Translation",
    description: "Supported via LocalRouter translation layer",
    className: "text-blue-600 dark:text-blue-400",
    shortLabel: "\u2713*",
  },
  not_supported: {
    icon: MinusCircle,
    label: "Not Supported",
    description: "Not available for this provider",
    className: "text-muted-foreground/50",
    shortLabel: "\u2014",
  },
  not_implemented: {
    icon: CircleDashed,
    label: "Not Yet Implemented",
    description: "Planned but not yet built",
    className: "text-muted-foreground/40",
    shortLabel: "\u2014",
  },
}

interface SupportLevelBadgeProps {
  level: SupportLevel
  notes?: string | null
  compact?: boolean
  /** Feature name for richer tooltips */
  featureName?: string
}

export function SupportLevelBadge({ level, notes, compact = false, featureName }: SupportLevelBadgeProps) {
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

  // Always show a tooltip: specific notes if available, otherwise the level description
  const tooltipTitle = featureName
    ? `${featureName}: ${config.label}`
    : config.label
  const tooltipBody = notes || config.description

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="cursor-help">{badge}</span>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-xs">
        {notes ? (
          <p className="text-xs">
            <span className="font-medium">{config.label}:</span> {notes}
          </p>
        ) : (
          <p className="text-xs">
            <span className="font-medium">{tooltipTitle}</span>
            {" \u2014 "}
            {tooltipBody}
          </p>
        )}
      </TooltipContent>
    </Tooltip>
  )
}
