/**
 * Global HTTPS-proxy firewall approval popup.
 *
 * Mounted once at the app root. When the proxy firewall hits an "ask" rule it
 * pauses the request and emits `proxy-firewall-ask`; this shows the request and
 * routes the user's Allow/Deny back to the backend, which resumes or blocks it.
 */

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import type { FirewallApprovalRequest, RespondProxyFirewallParams } from "@/types/tauri-commands"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/Button"
import { ShieldQuestion } from "lucide-react"

export function ProxyFirewallApproval() {
  const [queue, setQueue] = useState<FirewallApprovalRequest[]>([])
  const current = queue[0] ?? null

  useEffect(() => {
    const l = listenSafe<FirewallApprovalRequest>("proxy-firewall-ask", (e) => {
      setQueue((q) => [...q, e.payload])
    })
    return () => l.cleanup()
  }, [])

  const respond = async (allow: boolean) => {
    if (!current) return
    const requestId = current.request_id
    setQueue((q) => q.slice(1))
    try {
      await invoke("respond_proxy_firewall", { requestId, allow } satisfies RespondProxyFirewallParams)
    } catch (e) {
      console.error("Failed to respond to firewall approval:", e)
    }
  }

  return (
    <AlertDialog open={!!current}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            <ShieldQuestion className="h-5 w-5 text-amber-500" />
            Approve LLM request?
          </AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="space-y-2 text-sm">
              <p>
                <span className="font-medium">{current?.client_name}</span> is sending a request
                through the HTTPS proxy.
              </p>
              <div className="grid grid-cols-2 gap-1 text-xs text-muted-foreground">
                <span>Model</span>
                <span className="font-mono text-foreground">{current?.model ?? "—"}</span>
                <span>Messages</span>
                <span className="text-foreground">{current?.message_count}</span>
                <span>Tools</span>
                <span className="text-foreground">{current?.has_tools ? "yes" : "no"}</span>
              </div>
              {current?.preview && (
                <pre className="max-h-40 overflow-auto rounded bg-muted p-2 text-[11px] whitespace-pre-wrap break-all">
                  {current.preview}
                </pre>
              )}
              {queue.length > 1 && (
                <p className="text-xs text-muted-foreground">{queue.length - 1} more waiting…</p>
              )}
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" onClick={() => respond(false)}>
            Deny
          </Button>
          <Button onClick={() => respond(true)}>Allow</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
