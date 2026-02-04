// Stub for @tauri-apps/api/event in demo mode
// Implements a proper in-memory event bus for demo functionality

type EventCallback<T = unknown> = (event: { payload: T }) => void

interface ListenerEntry {
  id: number
  callback: EventCallback
}

// In-memory event bus for demo mode
const eventListeners = new Map<string, Set<ListenerEntry>>()
let listenerIdCounter = 0

/**
 * Listen for events from the Tauri backend (mock version)
 */
export const listen = async <T = unknown>(
  eventName: string,
  callback: EventCallback<T>
): Promise<() => void> => {
  const id = ++listenerIdCounter

  if (!eventListeners.has(eventName)) {
    eventListeners.set(eventName, new Set())
  }

  const entry: ListenerEntry = { id, callback: callback as EventCallback }
  eventListeners.get(eventName)!.add(entry)

  // Return unlisten function
  return () => {
    const listeners = eventListeners.get(eventName)
    if (listeners) {
      listeners.delete(entry)
    }
  }
}

/**
 * Emit an event to all registered listeners (mock version)
 */
export const emit = async <T = unknown>(eventName: string, payload?: T): Promise<void> => {
  const listeners = eventListeners.get(eventName)
  if (listeners) {
    listeners.forEach(({ callback }) => {
      // Use setTimeout to simulate async event delivery like real Tauri
      setTimeout(() => {
        try {
          callback({ payload })
        } catch (error) {
          console.error(`[Mock Event] Error in listener for "${eventName}":`, error)
        }
      }, 0)
    })
  }
}

/**
 * Once listener - listens for an event only once
 */
export const once = async <T = unknown>(
  eventName: string,
  callback: EventCallback<T>
): Promise<() => void> => {
  const unlisten = await listen<T>(eventName, (event) => {
    unlisten()
    callback(event)
  })
  return unlisten
}
