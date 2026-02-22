/**
 * Static demo showing a marketplace gated installation approval dialog.
 * Uses the shared FirewallApprovalCard with marketplace request type.
 */
import {
  FirewallApprovalCard,
} from "@app/components/shared/FirewallApprovalCard"

export function MarketplaceInstallDemo() {
  const noop = () => {}

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Claude Code"
        toolName="marketplace_install"
        serverName="marketplace"
        argumentsPreview={JSON.stringify({ package: "@modelcontextprotocol/server-filesystem", version: "2025.1.2" })}
        onAction={noop}
      />
    </div>
  )
}
