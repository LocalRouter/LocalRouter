import { cn } from "@/lib/utils"
import type { IndexingState } from "@/types/tauri-commands"

interface IndexingStateButtonProps {
  value: IndexingState
  onChange: (state: IndexingState) => void
  disabled?: boolean
  size?: "sm" | "md"
  /** Whether this state is inherited from parent (not explicitly set) */
  inherited?: boolean
  /** States that children have explicitly set (shown as transparent indicators) */
  childRollupStates?: Set<IndexingState>
}

const stateConfig: Record<IndexingState, { label: string; activeClass: string; rollupClass: string }> = {
  enable: {
    label: "On",
    activeClass: "bg-emerald-500 text-white",
    rollupClass: "bg-emerald-500/30 text-emerald-600",
  },
  disable: {
    label: "Off",
    activeClass: "bg-zinc-500 text-white",
    rollupClass: "bg-zinc-500/30 text-zinc-600",
  },
}

const ALL_STATES: IndexingState[] = ["enable", "disable"]

export function IndexingStateButton({
  value,
  onChange,
  disabled = false,
  size = "md",
  inherited = false,
  childRollupStates,
}: IndexingStateButtonProps) {
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none"
      )}
    >
      {ALL_STATES.map((state) => {
        const config = stateConfig[state]
        const isActive = value === state
        const isChildRollup = childRollupStates?.has(state) && !isActive

        const getButtonClass = () => {
          if (isActive) {
            return cn(config.activeClass, inherited && "opacity-60")
          }
          if (isChildRollup) {
            return config.rollupClass
          }
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
              state === ALL_STATES[0] && "rounded-l-md",
              state === ALL_STATES[ALL_STATES.length - 1] && "rounded-r-md"
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}
