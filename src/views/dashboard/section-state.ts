/**
 * Decoders for the Dashboard's remembered UI state. Values saved before the
 * durable store existed came from localStorage as strings ('1'/'0', 'right',
 * '60'), so each decoder also accepts those.
 */
import type { DetailDock } from './dock'

export function decodeCollapsed(value: unknown): boolean | undefined {
  if (value === true || value === 1 || value === '1') return true
  if (value === false || value === 0 || value === '0') return false
  return undefined
}

export function decodeDock(value: unknown): DetailDock | undefined {
  return value === 'right' || value === 'bottom' ? value : undefined
}

/** A detail panel size in percent, within the range the layout allows. */
export function decodeDetailSize(value: unknown): number | undefined {
  const size = typeof value === 'string' && value.trim() !== '' ? Number(value) : value
  return typeof size === 'number' && Number.isFinite(size) && size >= 20 && size <= 85 ? size : undefined
}
