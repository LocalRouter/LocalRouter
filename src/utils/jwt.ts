/** Decode display-only JWT contents locally. This does not verify the signature. */
export function decodeJwt(token: string): { header: Record<string, unknown>; payload: Record<string, unknown> } | null {
  const parts = token.split('.')
  if (parts.length !== 3) return null

  const decodeObject = (part: string): Record<string, unknown> => {
    if (!/^[A-Za-z0-9_-]+$/.test(part)) throw new Error('Invalid base64url')
    const base64 = part.replace(/-/g, '+').replace(/_/g, '/')
    const bytes = Uint8Array.from(atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, '=')), char => char.charCodeAt(0))
    const value: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
    if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new Error('Expected a JSON object')
    return value as Record<string, unknown>
  }

  try {
    return { header: decodeObject(parts[0]), payload: decodeObject(parts[1]) }
  } catch {
    return null
  }
}
