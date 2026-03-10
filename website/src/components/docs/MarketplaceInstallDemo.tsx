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
        toolName="marketplace__install"
        serverName="Marketplace"
        marketplaceListing={{
          name: "@modelcontextprotocol/server-filesystem",
          description: "Node.js server implementing Model Context Protocol for filesystem operations",
          vendor: "Anthropic",
          homepage: "https://github.com/modelcontextprotocol/servers",
          install_type: "mcp_server",
        }}
        onAction={noop}
      />
    </div>
  )
}
