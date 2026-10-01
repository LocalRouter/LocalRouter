import fs from 'node:fs'
import path from 'node:path'

/** Resolve a flat public icon URL without exposing sibling files or symlinks. */
export function resolveSharedIcon(iconsDir: string, requestUrl: string): string | null {
  try {
    const filename = decodeURIComponent(requestUrl.split('?')[0]).replace(/^\//, '')
    if (!filename || filename !== path.basename(filename) || filename.includes('\\') || filename.includes('\0')) return null
    if (!/\.(png|svg|gif|jpe?g|ico|webp)$/i.test(filename)) return null
    const root = fs.realpathSync(iconsDir)
    const resolved = fs.realpathSync(path.join(root, filename))
    if (path.dirname(resolved) !== root || !fs.statSync(resolved).isFile()) return null
    return resolved
  } catch {
    // Malformed escapes, missing files, and unreadable paths are not icons.
    return null
  }
}
