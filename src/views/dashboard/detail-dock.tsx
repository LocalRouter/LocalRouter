import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react'
import { GripVertical, PanelBottom, PanelRight, X } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import { readUiState, usePersistentUiState, writeUiState } from '@/hooks/usePersistentUiState'
import { dockTarget, type DetailDock } from './dock'
import { decodeDetailSize, decodeDock } from './section-state'

export type { DetailDock }


const DOCK_KEY = 'monitor.detailDock'
const SIZE_KEY = 'monitor.detailSize'
const DEFAULT_SIZE = 60

function loadSize(dock: DetailDock): number {
  return readUiState(`${SIZE_KEY}.${dock}`, DEFAULT_SIZE, decodeDetailSize)
}

/**
 * Where the event detail is docked and how large it is, both remembered per
 * dock side. Dragging the detail's grip previews the nearest edge and docks
 * the detail there on release.
 */
export function useDetailDock(bodyRef: RefObject<HTMLElement | null>) {
  const [dock, setDockState] = usePersistentUiState<DetailDock>(DOCK_KEY, 'bottom', decodeDock)
  const [drag, setDrag] = useState<{ target: DetailDock } | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)

  const setDock = useCallback((next: DetailDock) => setDockState(next), [setDockState])

  const saveLayout = useCallback((layout: { [id: string]: number }) => {
    const size = layout.detail
    if (size == null) return
    writeUiState(`${SIZE_KEY}.${dock}`, Math.round(size))
  }, [dock])

  const startDrag = useCallback((event: ReactPointerEvent) => {
    if (event.button !== 0) return
    event.preventDefault()
    cleanupRef.current?.()
    let target = dock
    setDrag({ target })

    const move = (e: PointerEvent) => {
      const rect = bodyRef.current?.getBoundingClientRect()
      if (!rect || rect.width === 0 || rect.height === 0) return
      const next = dockTarget(rect, e.clientX, e.clientY)
      if (next !== target) {
        target = next
        setDrag({ target })
      }
    }
    const finish = (commit: boolean) => {
      cleanup()
      setDrag(null)
      if (commit) setDock(target)
    }
    const up = () => finish(true)
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish(false)
    }
    const cleanup = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('keydown', key)
      document.body.style.removeProperty('cursor')
      cleanupRef.current = null
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('keydown', key)
    document.body.style.cursor = 'grabbing'
    cleanupRef.current = cleanup
  }, [bodyRef, dock, setDock])

  useEffect(() => () => cleanupRef.current?.(), [])

  return { dock, setDock, drag, startDrag, detailSize: loadSize(dock), saveLayout }
}

export function DetailDockControls({
  dock,
  onDockChange,
  onDragStart,
  onClose,
}: {
  dock: DetailDock
  onDockChange: (dock: DetailDock) => void
  onDragStart: (event: ReactPointerEvent) => void
  onClose: () => void
}) {
  const other: DetailDock = dock === 'bottom' ? 'right' : 'bottom'
  const OtherIcon = other === 'right' ? PanelRight : PanelBottom
  return (
    <div className="flex items-center gap-0.5">
      <button
        type="button"
        onPointerDown={onDragStart}
        title="Drag to dock the details below or beside the list"
        aria-label="Drag to move details"
        className="flex h-7 w-6 cursor-grab touch-none items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground active:cursor-grabbing"
      >
        <GripVertical className="h-3.5 w-3.5" />
      </button>
      <Button
        variant="ghost"
        size="icon"
        className="h-7 w-7 text-muted-foreground"
        onClick={() => onDockChange(other)}
        title={other === 'right' ? 'Dock details on the right' : 'Dock details below'}
        aria-label={other === 'right' ? 'Dock details on the right' : 'Dock details below'}
      >
        <OtherIcon className="h-3.5 w-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        className="h-7 w-7 text-muted-foreground"
        onClick={onClose}
        title="Close details"
        aria-label="Close details"
      >
        <X className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}

/** Outline of where the detail will dock when the drag is released. */
export function DockDropPreview({ target }: { target: DetailDock }) {
  return (
    <div className="pointer-events-none absolute inset-0 z-30" aria-hidden>
      <div
        className={cn(
          'absolute flex items-center justify-center rounded-lg border-2 border-dashed border-primary bg-primary/10 text-xs font-medium text-primary transition-all',
          target === 'right' ? 'inset-y-1 right-1 w-[45%]' : 'inset-x-1 bottom-1 h-[55%]',
        )}
      >
        {target === 'right' ? 'Dock on the right' : 'Dock below'}
      </div>
    </div>
  )
}
