import { useState, type ReactNode } from 'react'

/** Render one bounded page, including when the user navigates a long collection. */
export function PagedItems<T>({ items, children, pageSize = 20, label = 'Items' }: {
  items: readonly T[]
  children: (item: T, index: number) => ReactNode
  pageSize?: number
  label?: string
}) {
  const [page, setPage] = useState(0)
  const lastPage = Math.max(0, Math.ceil(items.length / pageSize) - 1)
  const current = Math.min(page, lastPage)
  const start = current * pageSize
  return <>
    {items.slice(start, start + pageSize).map((item, index) => children(item, start + index))}
    {items.length > pageSize && <div className="flex items-center gap-2 text-[10px] text-muted-foreground" aria-label={`${label} pages`}>
      <button type="button" className="underline disabled:opacity-40" disabled={current === 0} onClick={() => setPage(current - 1)}>Previous {label.toLowerCase()}</button>
      <span>{start + 1}–{Math.min(items.length, start + pageSize)} of {items.length}</span>
      <button type="button" className="underline disabled:opacity-40" disabled={current === lastPage} onClick={() => setPage(current + 1)}>Next {label.toLowerCase()}</button>
    </div>}
  </>
}
