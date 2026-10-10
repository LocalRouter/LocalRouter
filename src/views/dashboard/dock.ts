export type DetailDock = 'bottom' | 'right'

/** The dock edge nearest to a pointer inside the monitor body. */
export function dockTarget(rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>, x: number, y: number): DetailDock {
  const fromRight = 1 - (x - rect.left) / rect.width
  const fromBottom = 1 - (y - rect.top) / rect.height
  return fromRight < fromBottom ? 'right' : 'bottom'
}
