import { useState } from 'react'
import { cn } from '@/lib/utils'

export const TEXT_SECTION_SIZE = 16_000

/** Keep wrapped-text layout bounded while retaining access to every character. */
export function SectionedText({ text, className, label = 'Payload', onChange }: { text: string; className?: string; label?: string; onChange?: (text: string) => void }) {
  const [section, setSection] = useState({ text, start: 0 })
  const start = section.text === text ? section.start : 0
  const end = Math.min(text.length, start + TEXT_SECTION_SIZE)
  return <div className="min-w-0 space-y-1" aria-label={label}>
    {text.length > TEXT_SECTION_SIZE && <div className="flex flex-wrap gap-2 text-[10px] text-muted-foreground">
      <span>Characters {(start + 1).toLocaleString()}–{end.toLocaleString()} of {text.length.toLocaleString()}</span>
      <button type="button" className="underline disabled:opacity-40" disabled={start === 0} onClick={() => setSection({ text, start: Math.max(0, start - TEXT_SECTION_SIZE) })}>Earlier section</button>
      <button type="button" className="underline disabled:opacity-40" disabled={end === text.length} onClick={() => setSection({ text, start: end })}>Later section</button>
    </div>}
    {onChange ? <textarea aria-label={label} className={cn('w-full min-h-24 rounded border bg-background p-2 font-mono text-xs', className)} spellCheck={false} value={text.slice(start, end)} onChange={event => {
      const updated = text.slice(0, start) + event.target.value + text.slice(end)
      setSection({ text: updated, start })
      onChange(updated)
    }} /> : <pre key={start} className={cn('whitespace-pre-wrap break-words font-mono text-xs max-h-56 overflow-auto', className)}>{text.slice(start, end)}</pre>}
  </div>
}
