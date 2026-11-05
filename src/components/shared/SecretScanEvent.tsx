import { useEffect, useRef, useState } from "react"
import type { SecretFindingSummary } from "@/types/tauri-commands"
import { Button } from "@/components/ui/Button"

interface Props { finding: SecretFindingSummary; requestJson?: string | null; revealed?: string }

/** A readable excerpt first; the complete event has its own native text viewer. */
export function SecretScanEvent({ finding, requestJson, revealed }: Props) {
  const [full, setFull] = useState(false)
  const editor = useRef<HTMLTextAreaElement>(null)
  const match = finding.request_match
  const located = !!requestJson && !!match && match.start >= 0 && match.end > match.start && match.end <= requestJson.length
  const selected = revealed ?? finding.matched_text
  const encoded = JSON.stringify(selected).slice(1, -1)
  const fullText = located ? requestJson!.slice(0, match!.start) + encoded + requestJson!.slice(match!.end) : requestJson ?? ""
  const before = finding.context?.before ?? (located ? requestJson!.slice(0, match!.start) : "")
  const after = finding.context?.after ?? (located ? requestJson!.slice(match!.end) : "")
  const nearBefore = before.slice(-300).split('\n').slice(-3).join('\n')
  const nearAfter = after.slice(0, 300).split('\n').slice(0, 4).join('\n')

  useEffect(() => {
    const element = editor.current
    if (!full || !element || !located) return
    element.focus()
    element.setSelectionRange(match!.start, match!.start + encoded.length)
    const prefix = fullText.slice(0, match!.start)
    const line = prefix.split('\n').length - 1
    const column = prefix.length - prefix.lastIndexOf('\n') - 1
    element.scrollTop = Math.max(0, line - 2) * 20
    element.scrollLeft = Math.max(0, column - 12) * 7.2
  }, [full, located, match?.start, encoded.length, fullText])

  return <section className="flex h-full min-h-0 flex-col gap-2" aria-label="Event">
    <div className="flex shrink-0 items-center justify-between gap-2">
      <p className="text-[10px] text-muted-foreground min-w-0 break-words">{finding.context ? `${finding.context.source} · Line ${finding.context.line}` : 'Outbound request'}</p>
      {requestJson && <Button variant="ghost" size="sm" className="shrink-0 h-7 px-2 text-xs" onClick={() => setFull(value => !value)}>{full ? 'Show context' : 'View full event'}</Button>}
    </div>
    {full ? <textarea ref={editor} aria-label="Full event JSON" readOnly wrap="off" spellCheck={false} value={fullText} className="min-h-0 flex-1 w-full resize-none rounded-md border bg-background p-3 font-mono text-xs leading-5 focus:outline-none focus:ring-1 focus:ring-ring" /> :
      <pre tabIndex={0} aria-label="Event context" className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-background p-3 font-mono text-xs leading-relaxed select-text">
        {before.length > nearBefore.length && '…'}{nearBefore}<mark className="rounded bg-orange-500/20 px-0.5 font-semibold text-foreground">{selected.slice(0, 300)}{selected.length > 300 && '…'}</mark>{nearAfter}{after.length > nearAfter.length && '…'}
      </pre>}
  </section>
}
