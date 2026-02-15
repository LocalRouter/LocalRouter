import { cn } from "@/lib/utils"

export type CategoryActionState = "allow" | "notify" | "ask"

interface CategoryActionButtonProps {
  value: CategoryActionState
  onChange: (state: CategoryActionState) => void
  disabled?: boolean
  size?: "sm" | "md"
  /** Whether this action is inherited from the global default (not explicitly set) */
  inherited?: boolean
  /** States that children have explicitly set (shown as transparent indicators) */
  childRollupStates?: Set<CategoryActionState>
}

const stateConfig: Record<CategoryActionState, { label: string; activeClass: string; rollupClass: string }> = {
  allow: {
    label: "Allow",
    activeClass: "bg-emerald-500 text-white",
    rollupClass: "bg-emerald-500/30 text-emerald-600"
  },
  notify: {
    label: "Notify",
    activeClass: "bg-blue-500 text-white",
    rollupClass: "bg-blue-500/30 text-blue-600"
  },
  ask: {
    label: "Ask",
    activeClass: "bg-amber-500 text-white",
    rollupClass: "bg-amber-500/30 text-amber-600"
  },
}

export function CategoryActionButton({
  value,
  onChange,
  disabled = false,
  size = "md",
  inherited = false,
  childRollupStates,
}: CategoryActionButtonProps) {
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none"
      )}
    >
      {(["allow", "notify", "ask"] as CategoryActionState[]).map((state) => {
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
              state === "allow" && "rounded-l-md",
              state === "ask" && "rounded-r-md"
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}
