import { Badge } from "@/components/ui/Badge"
import { cn } from "@/lib/utils"

interface ExperimentalBadgeProps {
  className?: string
}

export function ExperimentalBadge({ className }: ExperimentalBadgeProps) {
  return (
    <Badge
      variant="outline"
      className={cn(
        "bg-purple-500/10 text-purple-900 dark:text-purple-400",
        className
      )}
    >
      EXPERIMENTAL
    </Badge>
  )
}
