/**
 * Website demo wrapper for the Push Relay approval popup.
 * Renders the shared FirewallApprovalCard for a held push event in a
 * dark-themed container.
 */
import { FirewallApprovalCard } from "@app/components/shared/FirewallApprovalCard"
import type { PushEventApprovalDetails } from "@app/types/tauri-commands"

const DEMO_EVENT: PushEventApprovalDetails = {
  kind: "event",
  hold_id: "demo-hold",
  server_id: "mcp-vocalcord",
  server_name: "vocalcord",
  event_name: "message.received",
  qualified_name: "vocalcord__message.received",
  event_id: "vocalcord__msg-551",
  ingress_mode: "push",
  data: {
    channel: "telegram",
    from: "Dana",
    text: "Can you look at the failing deploy?",
  },
  rendered_content: "Dana (telegram): Can you look at the failing deploy?",
  pending_for_client: 3,
  pending_of_type: 2,
}

export function PushEventApprovalDemo() {
  const noop = () => {}
  const receivedAt = new Date(Date.now() - 2 * 60_000).toISOString()

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl w-[420px] max-w-full bg-background text-foreground">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Claude Code"
        toolName={DEMO_EVENT.qualified_name}
        serverName={DEMO_EVENT.server_name}
        isPushEventRequest
        pushEvent={DEMO_EVENT}
        pushReceivedAt={receivedAt}
        onAction={noop}
        onAllowAllPending={noop}
        onEdit={noop}
      />
    </div>
  )
}
