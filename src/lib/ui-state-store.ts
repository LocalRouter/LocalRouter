/**
 * Durable UI state: small JSON values the UI remembers between runs (folded
 * dashboard sections, dock side, filters).
 *
 * The backend file (`get_ui_state` / `set_ui_state`) is the source of truth,
 * because the WebView's localStorage buffers writes and loses the recent ones
 * when the app process ends abruptly. Every value is also mirrored to
 * localStorage under the same key, as JSON, which is what the website demo and
 * a failed backend fall back to. A key the backend has never stored is adopted
 * from localStorage on first read and written through, so state saved before
 * the backend store existed carries over.
 *
 * Kept free of React and Tauri imports so it can be unit tested.
 */

export interface UiStateBackend {
  load(): Promise<Record<string, unknown>>
  save(key: string, value: unknown): Promise<void>
}

/** The subset of `Storage` the store uses. */
export interface UiStateMirror {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

type Listener = () => void

/** Reads a localStorage value written as JSON, or as a bare legacy string. */
function parseMirrored(raw: string): unknown {
  try {
    return JSON.parse(raw)
  } catch {
    return raw
  }
}

export class UiStateStore {
  private values = new Map<string, unknown>()
  /** Mirror reads made before the backend map loaded, cached so repeated reads are stable. */
  private earlyReads = new Map<string, { raw: string; value: unknown }>()
  private backendKeys = new Set<string>()
  private loaded = false
  private backendAvailable = false
  private initPromise: Promise<void> | null = null
  private listeners = new Map<string, Set<Listener>>()
  private pendingWrites = new Map<string, unknown>()
  private flushing: Promise<void> | null = null

  constructor(
    private readonly backend: UiStateBackend,
    private readonly mirror: UiStateMirror | null,
    private readonly loadTimeoutMs = 3000,
  ) {}

  /** Loads the backend map once. Never rejects; a failure leaves localStorage as the source. */
  init(): Promise<void> {
    if (!this.initPromise) {
      this.initPromise = this.load()
    }
    return this.initPromise
  }

  private async load() {
    let timer: ReturnType<typeof setTimeout> | undefined
    try {
      const map = await Promise.race([
        this.backend.load(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('timed out loading UI state')), this.loadTimeoutMs)
        }),
      ])
      if (map && typeof map === 'object') {
        for (const [key, value] of Object.entries(map)) {
          if (value === null || value === undefined) continue
          this.backendKeys.add(key)
          // A value set while the map was loading is newer; its write is queued.
          if (this.values.has(key)) continue
          this.values.set(key, value)
          this.writeMirror(key, value)
        }
      }
      this.backendAvailable = true
    } catch (error) {
      console.warn('UI state backend unavailable; using localStorage only:', error)
    } finally {
      clearTimeout(timer)
      this.loaded = true
      this.earlyReads.clear()
      this.notifyAll()
    }
  }

  /** The stored value for `key`, or `undefined` if nothing is stored. */
  get(key: string): unknown {
    if (this.values.has(key)) return this.values.get(key)
    const raw = this.readMirror(key)
    if (raw === null) return undefined
    const value = parseMirrored(raw)
    // Before the backend map has loaded, report the mirror without adopting
    // it, so the backend's value wins once it arrives.
    if (!this.loaded) {
      const early = this.earlyReads.get(key)
      if (early && early.raw === raw) return early.value
      this.earlyReads.set(key, { raw, value })
      return value
    }
    this.values.set(key, value)
    if (this.backendAvailable && !this.backendKeys.has(key)) {
      // Adopt a value saved before the backend store existed.
      this.enqueueWrite(key, value)
    }
    return value
  }

  set(key: string, value: unknown) {
    if (value === undefined) return
    const previous = this.values.get(key)
    if (this.values.has(key) && JSON.stringify(previous) === JSON.stringify(value)) return
    this.values.set(key, value)
    this.writeMirror(key, value)
    this.enqueueWrite(key, value)
    this.notify(key)
  }

  subscribe(key: string, listener: Listener): () => void {
    let set = this.listeners.get(key)
    if (!set) {
      set = new Set()
      this.listeners.set(key, set)
    }
    set.add(listener)
    return () => {
      set.delete(listener)
      if (set.size === 0) this.listeners.delete(key)
    }
  }

  /** Resolves once every queued backend write has been attempted. */
  async whenIdle(): Promise<void> {
    while (this.flushing) await this.flushing
  }

  private enqueueWrite(key: string, value: unknown) {
    // Only the latest value per key is sent, and one write is in flight at a
    // time, so writes reach the backend in the order they were made.
    this.pendingWrites.set(key, value)
    if (!this.flushing) {
      this.flushing = this.flush().finally(() => {
        this.flushing = null
      })
    }
  }

  private async flush() {
    await this.init()
    while (this.pendingWrites.size > 0) {
      const [key, value] = this.pendingWrites.entries().next().value as [string, unknown]
      this.pendingWrites.delete(key)
      if (!this.backendAvailable) continue
      try {
        await this.backend.save(key, value)
        this.backendKeys.add(key)
      } catch (error) {
        console.warn(`Failed to save UI state "${key}":`, error)
      }
    }
  }

  private readMirror(key: string): string | null {
    try {
      return this.mirror?.getItem(key) ?? null
    } catch {
      return null
    }
  }

  private writeMirror(key: string, value: unknown) {
    try {
      this.mirror?.setItem(key, JSON.stringify(value))
    } catch {
      // The mirror is optional.
    }
  }

  private notify(key: string) {
    this.listeners.get(key)?.forEach((listener) => listener())
  }

  private notifyAll() {
    this.listeners.forEach((set) => set.forEach((listener) => listener()))
  }
}
