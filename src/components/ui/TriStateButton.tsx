import { cn } from "@/lib/utils"

type TriStateValue = "default" | "on" | "off"

interface TriStateButtonProps {
  value: boolean | null
  onChange: (value: boolean | null) => void
  disabled?: boolean
  size?: "sm" | "md"
  /** Label shown for the default/inherit option */
  defaultLabel?: string
  onLabel?: string
  offLabel?: string
}

const stateConfig: Record<TriStateValue, { activeClass: string }> = {
  default: {
    activeClass: "bg-zinc-500 text-white",
  },
  on: {
    activeClass: "bg-emerald-500 text-white",
  },
  off: {
    activeClass: "bg-red-500 text-white",
  },
}

function toTriState(value: boolean | null): TriStateValue {
  if (value === null) return "default"
  return value ? "on" : "off"
}

function fromTriState(state: TriStateValue): boolean | null {
  if (state === "default") return null
  return state === "on"
}

export function TriStateButton({
  value,
  onChange,
  disabled = false,
  size = "sm",
  defaultLabel = "Default",
  onLabel = "On",
  offLabel = "Off",
}: TriStateButtonProps) {
  const current = toTriState(value)
  const buttons: { key: TriStateValue; label: string; position: "first" | "middle" | "last" }[] = [
    { key: "default", label: defaultLabel, position: "first" },
    { key: "on", label: onLabel, position: "middle" },
    { key: "off", label: offLabel, position: "last" },
  ]

  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none"
      )}
    >
      {buttons.map(({ key, label, position }) => {
        const isActive = current === key
        return (
          <button
            key={key}
            type="button"
            onClick={() => onChange(fromTriState(key))}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm",
              isActive
                ? stateConfig[key].activeClass
                : "text-muted-foreground hover:text-foreground hover:bg-muted",
              position === "first" && "rounded-l-md",
              position === "last" && "rounded-r-md"
            )}
          >
            {label}
          </button>
        )
      })}
    </div>
  )
}
