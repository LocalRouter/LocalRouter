/**
 * Normalize whatever `invoke()` or other async boundaries throw into a
 * user-visible string.
 *
 * Tauri commands that return `Result<T, String>` reject the JS promise
 * with the raw string (not an `Error` instance), so the common pattern
 * `err instanceof Error ? err.message : '<generic fallback>'` silently
 * swallows the actual backend message. This helper covers the three
 * shapes we see in practice: `Error`, string, and arbitrary JSON.
 */
export function errorMessage(err: unknown, fallback = 'Unknown error'): string {
  if (err instanceof Error) {
    return err.message || fallback
  }
  if (typeof err === 'string') {
    return err || fallback
  }
  if (err && typeof err === 'object') {
    // Tauri sometimes wraps the error in `{ message }` / `{ error }`;
    // fall back to JSON so the user at least sees the raw payload.
    const asRecord = err as Record<string, unknown>
    if (typeof asRecord.message === 'string') return asRecord.message
    if (typeof asRecord.error === 'string') return asRecord.error
    try {
      return JSON.stringify(err)
    } catch {
      return fallback
    }
  }
  return fallback
}
