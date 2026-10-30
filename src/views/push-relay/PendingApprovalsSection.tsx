import { useMemo, useState } from "react"
import { Loader2 } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import type { PendingPushApproval, PushApprovalAction } from "@/types/tauri-commands"
import { formatRelativeTime, payloadPreview } from "./format"

export interface PendingApprovalsSectionProps {
  /** Held events; `null` until the first load (section hidden). */
  approvals: PendingPushApproval[] | null
  /**
   * Answer one held occurrence (`submit_push_approval`). `editedData` is set
   * for `allow_edited`. Omit to render the queue read-only.
   */
  onDecide?: (requestId: string, action: PushApprovalAction, editedData?: unknown) => Promise<void>
  /** Open a client's detail page. */
  onOpenClient?: (clientId: string) => void
}

interface ApprovalGroup {
  key: string
  client_id: string
  client_name: string
  server_name: string
  event_name: string
  items: PendingPushApproval[]
}

/**
 * The Ask queue: one window per client and event type with a count, rather
 * than a popup per occurrence (plan: "Holding semantics (Ask)").
 */
export function PendingApprovalsSection({ approvals, onDecide, onOpenClient }: PendingApprovalsSectionProps) {
  const [busy, setBusy] = useState<string | null>(null)

  const groups = useMemo(() => {
    const map = new Map<string, ApprovalGroup>()
    for (const a of approvals ?? []) {
      const key = `${a.client_id}|${a.qualified_name}`
      const group = map.get(key) ?? {
        key,
        client_id: a.client_id,
        client_name: a.client_name,
        server_name: a.server_name,
        event_name: a.event_name,
        items: [],
      }
      group.items.push(a)
      map.set(key, group)
    }
    return [...map.values()]
  }, [approvals])

  if (approvals === null) return null

  const decide = async (requestId: string, action: PushApprovalAction) => {
    if (!onDecide) return
    setBusy(requestId + action)
    try {
      await onDecide(requestId, action)
    } finally {
      setBusy(null)
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <CardTitle className="text-base">Pending approvals</CardTitle>
          {approvals.length > 0 && <Badge variant="warning">{approvals.length}</Badge>}
        </div>
        <CardDescription>
          Events whose type is set to Ask are held here before they reach the client. Unanswered
          events are denied when the approval timeout passes.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {groups.length === 0 ? (
          <p className="text-sm text-muted-foreground">Nothing is waiting for approval.</p>
        ) : (
          groups.map((group) => {
            // The head of the queue is the one being asked about; already
            // decided events only wait for the ones ahead of them.
            const first = group.items.find((i) => !i.decided) ?? group.items[0]
            const undecided = group.items.filter((i) => !i.decided).length
            return (
              <div key={group.key} className="rounded-md border p-3 space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="text-sm font-medium truncate">
                      <code className="text-xs">{group.event_name}</code>
                      <span className="text-muted-foreground"> from {group.server_name} for </span>
                      {onOpenClient ? (
                        <button className="hover:underline" onClick={() => onOpenClient(group.client_id)}>
                          {group.client_name}
                        </button>
                      ) : (
                        group.client_name
                      )}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {group.items.length} held · oldest {formatRelativeTime(group.items[0].received_at)} · denied{" "}
                      {formatRelativeTime(first.expires_at)}
                    </p>
                  </div>
                  {onDecide && undecided > 1 && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy !== null}
                      onClick={() => decide(first.request_id, "allow_all_pending_of_type")}
                    >
                      Allow all {undecided}
                    </Button>
                  )}
                </div>
                <pre className="text-xs bg-muted rounded p-2 whitespace-pre-wrap break-words max-h-24 overflow-y-auto">
                  {payloadPreview(first.data)}
                </pre>
                {onDecide && !first.decided && (
                  <div className="flex flex-wrap gap-1.5">
                    {(
                      [
                        ["allow", "Allow"],
                        ["deny", "Deny"],
                        ["allow_type_session", "Allow type this session"],
                        ["allow_type_always", "Always allow this type"],
                        ["deny_type_always", "Always block this type"],
                      ] as [PushApprovalAction, string][]
                    ).map(([action, label]) => (
                      <Button
                        key={action}
                        size="sm"
                        variant={action === "allow" ? "default" : "ghost"}
                        disabled={busy !== null}
                        onClick={() => decide(first.request_id, action)}
                      >
                        {busy === first.request_id + action && <Loader2 className="h-3 w-3 mr-1 animate-spin" />}
                        {label}
                      </Button>
                    ))}
                  </div>
                )}
              </div>
            )
          })
        )}
      </CardContent>
    </Card>
  )
}
