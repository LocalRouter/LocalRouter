import { useEffect, useRef, useState } from "react"
import type { SecretFindingSummary } from "@/types/tauri-commands"

interface Props {
  finding: SecretFindingSummary
  requestJson?: string | null
  revealed?: string
}

// Bound wrapped-text layout even after navigating through a very large request.
const SECTION_SIZE = 16_000

export function SecretScanEvent({ finding, requestJson, revealed }: Props) {
  const match = finding.request_match
  const hasRequestMatch = !!requestJson && !!match && match.start >= 0 && match.end > match.start && match.end <= requestJson.length
  const text = requestJson ?? ""
  const initialStart = hasRequestMatch && text.length > SECTION_SIZE ? Math.max(0, match!.start - SECTION_SIZE / 2) : 0
  const [section, setSection] = useState<{ text: string; matchStart?: number; start: number } | null>(null)
  const start = section?.text === text && section.matchStart === match?.start ? section.start : initialStart
  const end = Math.min(text.length, start + SECTION_SIZE)
  const visibleMatch = hasRequestMatch && match!.start >= start && match!.start < end
  const sourceRef = useRef<HTMLPreElement>(null)
  const matchRef = useRef<HTMLElement>(null)
  const navigate = (next: number) => setSection({ text, matchStart: match?.start, start: next })

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const source = sourceRef.current
      const mark = matchRef.current
      if (!source) return
      source.scrollTop = 0
      if (!mark) return
      // Scroll only the event text, leaving the popup and approval actions still.
      source.scrollTop += mark.getBoundingClientRect().top - source.getBoundingClientRect().top - source.clientHeight / 2 + mark.offsetHeight / 2
    })
    return () => cancelAnimationFrame(frame)
  }, [text, match?.start, start])

  // Replace only the selected match. Other secrets stay masked by the backend.
  // A pathological match/reveal must not bypass the layout bound either.
  const selectedText = visibleMatch ? JSON.stringify(revealed ?? finding.matched_text).slice(1, -1) : ""
  const clippedMatch = selectedText.length > SECTION_SIZE

  return (
    <div className="min-w-0 space-y-1.5">
      {finding.context && <p className="text-[10px] text-muted-foreground break-words">{finding.context.source} · Line {finding.context.line} · First occurrence</p>}
      {text.length > SECTION_SIZE && (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground" aria-label="Request sections">
          <span>Characters {(start + 1).toLocaleString()}–{end.toLocaleString()} of {text.length.toLocaleString()}</span>
          <button type="button" className="underline" disabled={start === 0} onClick={() => navigate(Math.max(0, start - SECTION_SIZE))}>Earlier section</button>
          <button type="button" className="underline" disabled={end === text.length} onClick={() => navigate(end)}>Later section</button>
          {hasRequestMatch && <button type="button" className="underline" onClick={() => navigate(initialStart)}>Jump to secret</button>}
        </div>
      )}
      <pre ref={sourceRef} tabIndex={0} aria-label="Complete request JSON" className="h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-background/70 p-2 text-xs font-mono leading-relaxed select-text">
        {visibleMatch ? <>{text.slice(start, match!.start)}<mark ref={matchRef} className="rounded bg-orange-500/20 px-0.5 font-semibold text-foreground">{selectedText.slice(0, SECTION_SIZE)}{clippedMatch && '…'}</mark>{text.slice(Math.max(match!.end, start), end)}</> : text.slice(start, end)}
      </pre>
      {clippedMatch && <p className="text-[10px] text-muted-foreground">Long secret preview truncated.</p>}
    </div>
  )
}
