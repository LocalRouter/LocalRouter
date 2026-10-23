import { useEffect, useRef, useState } from "react"
import { Eye, EyeOff } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { SecretFindingSummary } from "@/types/tauri-commands"

interface Props {
  finding: SecretFindingSummary
  requestJson?: string | null
  revealed?: string
  onReveal?: () => void
}

export function SecretScanEvent({ finding, requestJson, revealed, onReveal }: Props) {
  const [tab, setTab] = useState("source")
  const sourceRef = useRef<HTMLPreElement>(null)
  const matchRef = useRef<HTMLElement>(null)

  useEffect(() => {
    if (tab !== "source") return
    const frame = requestAnimationFrame(() => {
      const source = sourceRef.current
      const match = matchRef.current
      if (!source || !match) return
      // Scroll only the event text, leaving the popup and approval actions still.
      source.scrollTop += match.getBoundingClientRect().top - source.getBoundingClientRect().top - source.clientHeight / 2 + match.offsetHeight / 2
    })
    return () => cancelAnimationFrame(frame)
  }, [tab])

  return (
    <Tabs value={tab} onValueChange={setTab} className="min-w-0">
      <div className="flex items-center justify-between gap-2">
        <TabsList className="h-7">
          <TabsTrigger className="px-2 py-1 text-[10px]" value="source">Detected source</TabsTrigger>
          <TabsTrigger className="px-2 py-1 text-[10px]" value="request" disabled={!requestJson}>Full request</TabsTrigger>
        </TabsList>
        {onReveal && <button className="shrink-0 p-1 text-muted-foreground hover:text-foreground" onClick={onReveal} title={revealed === undefined ? "Reveal secret" : "Hide secret"} aria-label={revealed === undefined ? "Reveal secret" : "Hide secret"}>{revealed === undefined ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}</button>}
      </div>
      <TabsContent value="source" className="space-y-1.5">
        <p className="text-[10px] text-muted-foreground break-words">{finding.context?.source} · Line {finding.context?.line} · First occurrence</p>
        <pre ref={sourceRef} tabIndex={0} aria-label="Complete detected source" className="h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-background/70 p-2 text-xs font-mono leading-relaxed select-text">
          {finding.context?.before}<mark ref={matchRef} className="rounded bg-orange-500/20 px-0.5 font-semibold text-foreground">{revealed ?? finding.matched_text}</mark>{finding.context?.after}
        </pre>
      </TabsContent>
      <TabsContent value="request">
        <pre tabIndex={0} aria-label="Complete request JSON" className="h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-background/70 p-2 text-xs font-mono leading-relaxed select-text">{requestJson}</pre>
      </TabsContent>
    </Tabs>
  )
}
