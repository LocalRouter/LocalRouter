import * as React from "react"
import { CircleHelp } from "lucide-react"
import { TooltipContent, TooltipProvider } from "./tooltip"
import * as TooltipPrimitive from "@radix-ui/react-tooltip"
import { cn } from "@/lib/utils"

interface InfoTooltipProps {
  /** Tooltip content — supports ReactNode for rich content */
  content: React.ReactNode
  /** Tooltip placement side */
  side?: "top" | "right" | "bottom" | "left"
  /** Delay before tooltip appears when hovering children (default 500ms) */
  controlDelay?: number
  /** Additional className for the icon */
  className?: string
  /** The control element — hovering it opens the same tooltip with a delay */
  children?: React.ReactNode
}

/**
 * Info icon with tooltip. Hovering the icon shows the tooltip instantly.
 * If children are provided, hovering them shows the same tooltip after a delay,
 * always anchored to the info icon.
 */
export function InfoTooltip({
  content,
  side = "top",
  controlDelay = 500,
  className,
  children,
}: InfoTooltipProps) {
  const [open, setOpen] = React.useState(false)
  const delayRef = React.useRef<ReturnType<typeof setTimeout> | null>(null)

  const clearDelay = () => {
    if (delayRef.current) {
      clearTimeout(delayRef.current)
      delayRef.current = null
    }
  }

  const showImmediate = () => {
    clearDelay()
    setOpen(true)
  }

  const showDelayed = () => {
    clearDelay()
    delayRef.current = setTimeout(() => setOpen(true), controlDelay)
  }

  const hide = () => {
    clearDelay()
    setOpen(false)
  }

  React.useEffect(() => clearDelay, [])

  const tooltipContent = typeof content === "string" ? <p>{content}</p> : content

  return (
    <TooltipProvider delayDuration={0}>
      <TooltipPrimitive.Root open={open} onOpenChange={setOpen}>
        <div
          className="flex items-center gap-2"
          onMouseLeave={hide}
        >
          <TooltipPrimitive.Trigger asChild>
            <span
              className={cn(
                "inline-flex cursor-help text-muted-foreground/70 hover:text-muted-foreground transition-colors ml-1",
                className,
              )}
              tabIndex={0}
              onMouseEnter={showImmediate}
            >
              <CircleHelp className="h-3.5 w-3.5" />
            </span>
          </TooltipPrimitive.Trigger>
          {children && (
            <div onMouseEnter={showDelayed} onMouseLeave={clearDelay}>
              {children}
            </div>
          )}
        </div>
        <TooltipContent side={side} className="max-w-xs text-xs">
          {tooltipContent}
        </TooltipContent>
      </TooltipPrimitive.Root>
    </TooltipProvider>
  )
}
