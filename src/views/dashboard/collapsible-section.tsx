import { useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, type LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

/** Open/closed state of a dashboard section, remembered across reloads. */
export function useCollapsed(storageKey: string) {
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(storageKey) === '1')
  const toggle = () => {
    setCollapsed((c) => {
      localStorage.setItem(storageKey, c ? '0' : '1')
      return !c
    })
  }
  return [collapsed, toggle] as const
}

interface CollapsibleSectionProps {
  /** Region name for assistive technology. */
  ariaLabel: string
  title: string
  icon: LucideIcon
  collapsed: boolean
  onToggle: () => void
  /** Header content after the title (summary, inline actions). */
  summary?: ReactNode
  /** Controls at the right end of the header. */
  actions?: ReactNode
  className?: string
  children?: ReactNode
}

/** A dashboard card whose body folds away behind its header. */
export function CollapsibleSection({
  ariaLabel,
  title,
  icon: Icon,
  collapsed,
  onToggle,
  summary,
  actions,
  className,
  children,
}: CollapsibleSectionProps) {
  return (
    <section
      className={cn('overflow-hidden rounded-xl border bg-card', className)}
      aria-label={ariaLabel}
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-2">
        <button
          type="button"
          onClick={onToggle}
          className="flex items-center gap-1.5 rounded text-sm font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-expanded={!collapsed}
        >
          {collapsed ? (
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          ) : (
            <ChevronDown className="h-4 w-4 text-muted-foreground" />
          )}
          <Icon className="h-4 w-4 text-muted-foreground" />
          {title}
        </button>
        {summary}
        {actions && <div className="ml-auto flex items-center gap-1">{actions}</div>}
      </div>
      {!collapsed && children}
    </section>
  )
}
