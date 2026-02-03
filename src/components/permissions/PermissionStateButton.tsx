import { cn } from "@/lib/utils"
import type { PermissionState } from "./types"

interface PermissionStateButtonProps {
  value: PermissionState
  onChange: (state: PermissionState) => void
  disabled?: boolean
  size?: "sm" | "md"
  inherited?: boolean
}

const stateConfig: Record<PermissionState, { label: string; activeClass: string }> = {
  allow: { label: "Allow", activeClass: "bg-emerald-500 text-white" },
  ask: { label: "Ask", activeClass: "bg-amber-500 text-white" },
  off: { label: "Off", activeClass: "bg-zinc-500 text-white" },
}

export function PermissionStateButton({
  value,
  onChange,
  disabled = false,
  size = "md",
  inherited = false,
}: PermissionStateButtonProps) {
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none",
        inherited && "opacity-60"
      )}
    >
      {(["allow", "ask", "off"] as PermissionState[]).map((state) => {
        const config = stateConfig[state]
        const isActive = value === state
        return (
          <button
            key={state}
            type="button"
            onClick={() => onChange(state)}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm",
              isActive
                ? config.activeClass
                : "text-muted-foreground hover:text-foreground hover:bg-muted",
              state === "allow" && "rounded-l-md",
              state === "off" && "rounded-r-md"
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}
