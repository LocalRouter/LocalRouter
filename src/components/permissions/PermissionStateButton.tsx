import { cn } from "@/lib/utils"
import type { PermissionState } from "./types"

interface PermissionStateButtonProps {
  value: PermissionState
  onChange: (state: PermissionState) => void
  disabled?: boolean
  size?: "sm" | "md"
  /** Whether this permission is inherited from parent (not explicitly set) */
  inherited?: boolean
  /** States that children have explicitly set (shown as transparent indicators) */
  childRollupStates?: Set<PermissionState>
}

const stateConfig: Record<PermissionState, { label: string; activeClass: string; rollupClass: string }> = {
  allow: {
    label: "Allow",
    activeClass: "bg-emerald-500 text-white",
    rollupClass: "bg-emerald-500/30 text-emerald-600"
  },
  ask: {
    label: "Ask",
    activeClass: "bg-amber-500 text-white",
    rollupClass: "bg-amber-500/30 text-amber-600"
  },
  off: {
    label: "Off",
    activeClass: "bg-zinc-500 text-white",
    rollupClass: "bg-zinc-500/30 text-zinc-600"
  },
}

export function PermissionStateButton({
  value,
  onChange,
  disabled = false,
  size = "md",
  inherited = false,
  childRollupStates,
}: PermissionStateButtonProps) {
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none"
      )}
    >
      {(["allow", "ask", "off"] as PermissionState[]).map((state) => {
        const config = stateConfig[state]
        const isActive = value === state
        const isChildRollup = childRollupStates?.has(state) && !isActive

        // Determine button appearance:
        // - Active and explicit (not inherited): full color
        // - Active and inherited: full color but with reduced opacity
        // - Child rollup (not active): transparent background with color tint
        // - Inactive: default muted state

        const getButtonClass = () => {
          if (isActive) {
            // Active state - use full color, but reduce opacity if inherited
            return cn(config.activeClass, inherited && "opacity-60")
          }
          if (isChildRollup) {
            // Child rollup indicator - transparent background with color tint
            return config.rollupClass
          }
          // Default inactive state
          return "text-muted-foreground hover:text-foreground hover:bg-muted"
        }

        return (
          <button
            key={state}
            type="button"
            onClick={() => onChange(state)}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm",
              getButtonClass(),
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
