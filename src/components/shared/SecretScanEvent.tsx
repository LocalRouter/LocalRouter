import { useEffect, useRef } from "react"
import type { SecretFindingSummary } from "@/types/tauri-commands"

interface Props {
  finding: SecretFindingSummary
  requestJson?: string | null
  revealed?: string
}

export function SecretScanEvent({ finding, requestJson, revealed }: Props) {
  const match = finding.request_match
  const hasRequestMatch = !!requestJson && !!match && match.start >= 0 && match.end > match.start && match.end <= requestJson.length
  const sourceRef = useRef<HTMLPreElement>(null)
  const matchRef = useRef<HTMLElement>(null)

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const source = sourceRef.current
      const match = matchRef.current
      if (!source || !match) return
      // Scroll only the event text, leaving the popup and approval actions still.
      source.scrollTop += match.getBoundingClientRect().top - source.getBoundingClientRect().top - source.clientHeight / 2 + match.offsetHeight / 2
    })
    return () => cancelAnimationFrame(frame)
  }, [requestJson, finding.request_match?.start])

  return (
    <div className="min-w-0 space-y-1.5">
      {finding.context && <p className="text-[10px] text-muted-foreground break-words">{finding.context.source} · Line {finding.context.line} · First occurrence</p>}
      <pre ref={sourceRef} tabIndex={0} aria-label="Complete request JSON" className="h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-background/70 p-2 text-xs font-mono leading-relaxed select-text">
        {hasRequestMatch ? <>{requestJson!.slice(0, match!.start)}<mark ref={matchRef} className="rounded bg-orange-500/20 px-0.5 font-semibold text-foreground">{JSON.stringify(revealed ?? finding.matched_text).slice(1, -1)}</mark>{requestJson!.slice(match!.end)}</> : requestJson}
      </pre>
    </div>
  )
}
