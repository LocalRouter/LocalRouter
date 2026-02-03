import * as React from "react"
import { AlertCircle } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { cn } from "@/lib/utils"

interface DisabledOverlayProps {
  title?: string
  description?: string
  actionLabel?: string
  onAction?: () => void
  className?: string
  children?: React.ReactNode
}

export function DisabledOverlay({
  title = "Feature Disabled",
  description = "This feature is currently disabled.",
  actionLabel = "Enable",
  onAction,
  className,
  children,
}: DisabledOverlayProps) {
  return (
    <div className={cn("relative", className)}>
      {children}
      <div className="absolute inset-0 bg-background/80 backdrop-blur-sm flex flex-col items-center justify-center gap-4 rounded-lg">
        <AlertCircle className="h-10 w-10 text-muted-foreground" />
        <div className="text-center px-4">
          <h3 className="font-semibold text-base">{title}</h3>
          <p className="text-sm text-muted-foreground mt-1">{description}</p>
        </div>
        {onAction && (
          <Button onClick={onAction} size="sm">
            {actionLabel}
          </Button>
        )}
      </div>
    </div>
  )
}
