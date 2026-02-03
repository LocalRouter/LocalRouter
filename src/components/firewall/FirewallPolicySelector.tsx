import { cn } from "@/lib/utils"

export type FirewallPolicy = "allow" | "ask" | "deny"

interface FirewallPolicySelectorProps {
  value: FirewallPolicy
  onChange: (policy: FirewallPolicy) => void
  disabled?: boolean
  size?: "sm" | "md"
}

const policyConfig: Record<FirewallPolicy, { label: string; activeClass: string }> = {
  allow: { label: "Allow", activeClass: "bg-emerald-500 text-white" },
  ask: { label: "Ask", activeClass: "bg-amber-500 text-white" },
  deny: { label: "Deny", activeClass: "bg-red-500 text-white" },
}

export function FirewallPolicySelector({
  value,
  onChange,
  disabled = false,
  size = "md",
}: FirewallPolicySelectorProps) {
  return (
    <div
      className={cn(
        "inline-flex rounded-md border border-border bg-muted/50",
        disabled && "opacity-50 pointer-events-none"
      )}
    >
      {(["allow", "ask", "deny"] as FirewallPolicy[]).map((policy) => {
        const config = policyConfig[policy]
        const isActive = value === policy
        return (
          <button
            key={policy}
            type="button"
            onClick={() => onChange(policy)}
            disabled={disabled}
            className={cn(
              "transition-colors font-medium",
              size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm",
              isActive
                ? config.activeClass
                : "text-muted-foreground hover:text-foreground hover:bg-muted",
              policy === "allow" && "rounded-l-md",
              policy === "deny" && "rounded-r-md"
            )}
          >
            {config.label}
          </button>
        )
      })}
    </div>
  )
}
