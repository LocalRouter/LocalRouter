import { cn } from "@/lib/utils"
import type { PushPermission } from "@/types/tauri-commands"
import { permissionStateConfig } from "./PermissionStateButton"

/** A tree value: a real permission, or "inherit" to clear the override. */
export type PushPermissionChoice = PushPermission | "inherit"

// Push permissions use the MCP permission vocabulary and colours
// (PermissionStateButton): Allow, Ask, Off.
const STATES: PushPermission[] = ["allow", "ask", "off"]

/** Active style of the Inherit / Default segment (distinct from Off's grey). */
const INHERIT_ACTIVE = "bg-foreground/80 text-background"

export function pushPermissionLabel(p: PushPermission): string {
  return permissionStateConfig[p].label
}

interface PushPermissionButtonProps {
  /** The effective permission (explicit or inherited) */
  value: PushPermission
  onChange: (state: PushPermissionChoice) => void
  disabled?: boolean
  size?: "sm" | "md"
  /** The value is inherited from the level above */
  inherited?: boolean
  /** Explicit values set somewhere below this node */
  childRollupStates?: Set<PushPermissionChoice>
  /** Show an "Inherit" segment (every level below the root) */
  showInherit?: boolean
}

/**
 * Allow / Ask / Off selector for Push Relay, with an optional Inherit segment.
 * As in the MCP permission tree, an inherited value is shown dimmed; Inherit is
 * then active too.
 */
export function PushPermissionButton({
  value,
  onChange,
  disabled = false,
  size = "md",
  inherited = false,
  childRollupStates,
  showInherit = true,
}: PushPermissionButtonProps) {
  const segment = size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm"
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none",
      )}
    >
      {showInherit && (
        <button
          type="button"
          onClick={() => onChange("inherit")}
          disabled={disabled}
          title="Use the setting from the level above"
          className={cn(
            "transition-colors font-medium rounded-l-md",
            segment,
            inherited ? INHERIT_ACTIVE : "text-muted-foreground hover:text-foreground hover:bg-muted",
          )}
        >
          Inherit
        </button>
      )}
      {STATES.map((state, idx) => {
        const config = permissionStateConfig[state]
        const isActive = value === state
        const isChildRollup = childRollupStates?.has(state) && !isActive
        return (
          <button
            key={state}
            type="button"
            onClick={() => onChange(state)}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              segment,
              isActive
                ? cn(config.activeClass, inherited && "opacity-60")
                : isChildRollup
                  ? config.rollupClass
                  : "text-muted-foreground hover:text-foreground hover:bg-muted",
              !showInherit && idx === 0 && "rounded-l-md",
              idx === STATES.length - 1 && "rounded-r-md",
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}

interface PushPermissionWithDefaultButtonProps {
  /** Explicit value, or null to use the global default */
  value: PushPermission | null
  globalDefault: PushPermission
  onChange: (state: PushPermission | null) => void
  disabled?: boolean
  size?: "sm" | "md"
  childRollupStates?: Set<PushPermissionChoice>
}

/** Root-level selector: "Default (Allow)" | Allow | Ask | Off. */
export function PushPermissionWithDefaultButton({
  value,
  globalDefault,
  onChange,
  disabled = false,
  size = "sm",
  childRollupStates,
}: PushPermissionWithDefaultButtonProps) {
  const segment = size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm"
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none",
      )}
    >
      <button
        type="button"
        onClick={() => onChange(null)}
        disabled={disabled}
        className={cn(
          "transition-colors font-medium rounded-l-md",
          segment,
          value === null ? INHERIT_ACTIVE : "text-muted-foreground hover:text-foreground hover:bg-muted",
        )}
      >
        Default ({pushPermissionLabel(globalDefault)})
      </button>
      {STATES.map((state, idx) => {
        const config = permissionStateConfig[state]
        const isActive = value === state
        const isChildRollup = childRollupStates?.has(state) && !isActive
        return (
          <button
            key={state}
            type="button"
            onClick={() => onChange(state)}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              segment,
              isActive
                ? config.activeClass
                : isChildRollup
                  ? config.rollupClass
                  : "text-muted-foreground hover:text-foreground hover:bg-muted",
              idx === STATES.length - 1 && "rounded-r-md",
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}
