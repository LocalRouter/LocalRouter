/**
 * Website demo wrapper for the Free-Tier Fallback approval popup.
 * Renders the shared FirewallApprovalCard with isFreeTierFallback=true
 * in a dark-themed container.
 */
import {
  FirewallApprovalCard,
} from "@app/components/shared/FirewallApprovalCard"

export function FreeTierFallbackDemo() {
  const noop = () => {}

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Claude Code"
        toolName="openai/gpt-4o"
        serverName="openai"
        isFreeTierFallback
        onAction={noop}
      />
    </div>
  )
}
