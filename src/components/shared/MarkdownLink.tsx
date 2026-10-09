import type { AnchorHTMLAttributes, MouseEvent } from 'react'
import type { Components } from 'react-markdown'
import { open } from '@tauri-apps/plugin-shell'
import { isValidHttpUrl } from '@/utils/url'

/**
 * Anchor renderer for `react-markdown` content that comes from untrusted
 * sources (model output, MCP tool results, release notes).
 *
 * Without this, a plain `<a href>` click performs an in-webview navigation:
 * the main LocalRouter window is replaced by the linked page, which can then
 * impersonate the app's own UI. Links are instead handed to the OS browser
 * via the shell plugin, and only `http(s)` targets are honoured.
 */
export function MarkdownLink({ href, children, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const safe = typeof href === 'string' && isValidHttpUrl(href)

  const handleClick = (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault()
    if (safe && href) {
      void open(href).catch(() => {})
    }
  }

  if (!safe) {
    // Render the text only; never expose a non-http(s) target.
    return <span {...(rest as Record<string, unknown>)}>{children}</span>
  }

  return (
    <a {...rest} href={href} onClick={handleClick} rel="noopener noreferrer" title={href}>
      {children}
    </a>
  )
}

/** `components` prop for `<ReactMarkdown>` that routes links through {@link MarkdownLink}. */
export const markdownLinkComponents: Components = {
  a: MarkdownLink,
}
