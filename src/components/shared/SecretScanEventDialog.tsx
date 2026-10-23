import { useEffect, useRef, useState } from "react"
import { Eye, EyeOff, LocateFixed } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { SecretFindingSummary } from "@/types/tauri-commands"

interface Props {
  finding: SecretFindingSummary
  requestJson?: string | null
  clientName: string
  model: string
  dark?: boolean
  revealError?: string
  revealed?: string
  onReveal?: () => void
  onClose: () => void
}

export function SecretScanEventDialog({ finding, requestJson, clientName, model, dark, revealError, revealed, onReveal, onClose }: Props) {
  const [tab, setTab] = useState("source")
  const matchRef = useRef<HTMLElement>(null)
  const jumpToSecret = () => matchRef.current?.scrollIntoView({ block: "center" })

  useEffect(() => {
    if (tab !== "source") return
    const frame = requestAnimationFrame(jumpToSecret)
    return () => cancelAnimationFrame(frame)
  }, [tab])

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose() }}>
      <DialogContent className={`${dark ? "dark" : ""} flex h-[calc(100dvh-2rem)] max-h-[760px] w-[calc(100vw-2rem)] max-w-4xl flex-col gap-3 p-4 text-foreground`}>
        <DialogHeader className="shrink-0 pr-6 text-left">
          <DialogTitle>Request event</DialogTitle>
          <DialogDescription className="break-words text-xs">{clientName} · {model}</DialogDescription>
        </DialogHeader>
        <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
          <TabsList className="self-start shrink-0">
            <TabsTrigger value="source">Detected source</TabsTrigger>
            <TabsTrigger value="request" disabled={!requestJson}>Full request</TabsTrigger>
          </TabsList>
          <TabsContent value="source" className="min-h-0 flex-1 data-[state=active]:flex data-[state=active]:flex-col gap-2">
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
              <p className="text-xs text-muted-foreground">{finding.context?.source} · Line {finding.context?.line} · First occurrence</p>
              <Button size="sm" variant="ghost" onClick={jumpToSecret}><LocateFixed className="mr-1 h-3.5 w-3.5" />Jump to secret</Button>
            </div>
            <pre tabIndex={0} aria-label="Complete detected source" className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border bg-muted/50 p-3 text-xs font-mono leading-relaxed select-text">
              {finding.context?.before}<mark ref={matchRef} className="rounded bg-orange-500/20 px-0.5 font-semibold text-foreground">{revealed ?? finding.matched_text}</mark>{finding.context?.after}
            </pre>
          </TabsContent>
          <TabsContent value="request" className="min-h-0 flex-1 data-[state=active]:flex data-[state=active]:flex-col">
            <pre tabIndex={0} aria-label="Complete request JSON" className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border bg-muted/50 p-3 text-xs font-mono leading-relaxed select-text">{requestJson}</pre>
          </TabsContent>
        </Tabs>
        {revealError && <p role="alert" className="text-xs text-destructive">{revealError}</p>}
        <div className="flex shrink-0 items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">Detected secrets are masked in the full request.</span>
          {tab === "source" && onReveal && <Button size="sm" variant="outline" onClick={onReveal}>{revealed === undefined ? <Eye className="mr-1 h-3.5 w-3.5" /> : <EyeOff className="mr-1 h-3.5 w-3.5" />}{revealed === undefined ? "Reveal secret" : "Hide secret"}</Button>}
        </div>
      </DialogContent>
    </Dialog>
  )
}
