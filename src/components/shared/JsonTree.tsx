import { useState } from 'react'
import { Check, ChevronRight, Copy } from 'lucide-react'
import { cn } from '@/lib/utils'

/** Containers deeper than this start collapsed, so large payloads open readable. */
const AUTO_EXPAND_DEPTH = 2
/** Long strings are clipped until expanded. */
const STRING_PREVIEW = 400

function isContainer(value: unknown): value is Record<string, unknown> | unknown[] {
  return value !== null && typeof value === 'object'
}

function entriesOf(value: Record<string, unknown> | unknown[]): [string, unknown][] {
  return Array.isArray(value) ? value.map((item, index) => [String(index), item]) : Object.entries(value)
}

function summary(value: Record<string, unknown> | unknown[]): string {
  const count = Array.isArray(value) ? value.length : Object.keys(value).length
  return Array.isArray(value) ? `${count} item${count === 1 ? '' : 's'}` : `${count} key${count === 1 ? '' : 's'}`
}

function Scalar({ value }: { value: unknown }) {
  const [full, setFull] = useState(false)
  if (value === null || value === undefined) return <span className="text-muted-foreground italic">null</span>
  if (typeof value === 'boolean') return <span className="text-purple-600 dark:text-purple-400">{String(value)}</span>
  if (typeof value === 'number') return <span className="text-blue-600 dark:text-blue-400">{value}</span>
  const text = String(value)
  const clipped = !full && text.length > STRING_PREVIEW
  return (
    <span className="text-emerald-700 dark:text-emerald-400 whitespace-pre-wrap break-words">
      "{clipped ? text.slice(0, STRING_PREVIEW) : text}"
      {text.length > STRING_PREVIEW && (
        <button type="button" onClick={() => setFull(f => !f)} className="ml-1 rounded border border-border/60 px-1 text-[10px] text-muted-foreground hover:text-foreground">
          {full ? 'less' : `+${(text.length - STRING_PREVIEW).toLocaleString()} chars`}
        </button>
      )}
    </span>
  )
}

function Node({ name, value, depth }: { name: string | null; value: unknown; depth: number }) {
  const [open, setOpen] = useState(depth < AUTO_EXPAND_DEPTH)
  const label = name !== null && <span className="text-foreground/80">{name}<span className="text-muted-foreground">: </span></span>

  if (!isContainer(value)) {
    return <div className="pl-4 leading-5">{label}<Scalar value={value} /></div>
  }

  const entries = entriesOf(value)
  const brackets = Array.isArray(value) ? ['[', ']'] : ['{', '}']
  if (entries.length === 0) {
    return <div className="pl-4 leading-5">{label}<span className="text-muted-foreground">{brackets.join('')}</span></div>
  }

  return (
    <div className={cn(depth > 0 && 'pl-4')}>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(o => !o)}
        className="-ml-4 flex items-center gap-0 rounded leading-5 hover:bg-muted/60 text-left"
      >
        <ChevronRight className={cn('h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform', open && 'rotate-90')} />
        <span className="pl-0.5">{label}</span>
        <span className="text-muted-foreground">{brackets[0]}</span>
        {!open && <span className="mx-1 text-[10px] text-muted-foreground">{summary(value)}</span>}
        {!open && <span className="text-muted-foreground">{brackets[1]}</span>}
      </button>
      {open && (
        <>
          <div className="border-l border-border/50 ml-[3px]">
            {entries.map(([key, child]) => <Node key={key} name={key} value={child} depth={depth + 1} />)}
          </div>
          <div className="leading-5 text-muted-foreground">{brackets[1]}</div>
        </>
      )}
    </div>
  )
}

/** Collapsible, syntax-coloured JSON view with a copy button. */
export function JsonTree({ data, label, className }: { data: unknown; label?: string; className?: string }) {
  const [copied, setCopied] = useState(false)
  if (data === undefined) return null
  return (
    <div className={cn('relative rounded-md border bg-muted/30 p-2 pl-5 font-mono text-[11px] overflow-auto', className)}>
      <button
        type="button"
        onClick={() => {
          navigator.clipboard.writeText(typeof data === 'string' ? data : JSON.stringify(data, null, 2)).then(() => {
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
          })
        }}
        title={label ? `Copy ${label}` : 'Copy JSON'}
        className="absolute right-1.5 top-1.5 z-10 flex items-center gap-1 rounded border border-border/60 bg-background/80 px-1.5 py-0.5 font-sans text-[10px] text-muted-foreground hover:text-foreground"
      >
        {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
        {copied ? 'Copied' : label ? `Copy ${label}` : 'Copy'}
      </button>
      <Node name={null} value={data} depth={0} />
    </div>
  )
}
