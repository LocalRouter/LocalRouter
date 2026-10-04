import { useSyncExternalStore } from 'react'
import { Clock } from 'lucide-react'
import type { MonitorEvent } from '@/types/tauri-commands'
import { eventDurationMs } from './monitor-events'

// One clock for all visible pending durations; only the duration labels rerender.
const listeners = new Set<() => void>()
let now = Date.now()
let timer: ReturnType<typeof setInterval> | undefined
const getSnapshot = () => now
const noop = () => {}
const subscribeInactive = () => noop

function subscribe(listener: () => void) {
  listeners.add(listener)
  if (timer === undefined) {
    now = Date.now()
    timer = setInterval(() => {
      now = Date.now()
      listeners.forEach(notify => notify())
    }, 100)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) {
      clearInterval(timer)
      timer = undefined
    }
  }
}

export function EventDuration({ event, showClock = false }: {
  event: Pick<MonitorEvent, 'timestamp' | 'status' | 'duration_ms'>
  showClock?: boolean
}) {
  const currentTime = useSyncExternalStore(event.status === 'pending' ? subscribe : subscribeInactive, getSnapshot)
  const duration = eventDurationMs(event, currentTime)
  if (duration == null) return showClock ? null : <>—</>
  return showClock ? (
    <span className="text-xs text-muted-foreground flex items-center gap-1 tabular-nums" data-testid="event-detail-duration">
      <Clock className="h-3 w-3" />{duration}ms
    </span>
  ) : <>{duration}ms</>
}
