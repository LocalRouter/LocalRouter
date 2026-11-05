import { useEffect } from 'react'
import { useLocation } from 'react-router-dom'
import { getSitePage, notFoundPage, siteOrigin } from '../site-pages'

export default function PageMetadata() {
  const { pathname } = useLocation()
  useEffect(() => {
    const page = getSitePage(pathname) ?? notFoundPage
    document.title = page.title
    const setMeta = (attribute: 'name' | 'property', key: string, content: string) => {
      let element = document.head.querySelector<HTMLMetaElement>(`meta[${attribute}="${key}"]`)
      if (!element) {
        element = document.createElement('meta')
        element.setAttribute(attribute, key)
        document.head.appendChild(element)
      }
      element.content = content
    }
    for (const key of ['title', 'twitter:title']) setMeta('name', key, page.title)
    for (const key of ['description', 'twitter:description']) setMeta('name', key, page.description)
    setMeta('property', 'og:title', page.title)
    setMeta('property', 'og:description', page.description)
    setMeta('name', 'robots', page.indexable ? 'index,follow' : 'noindex,follow')
    const url = `${siteOrigin}${page.path}`
    setMeta('property', 'og:url', url)
    setMeta('name', 'twitter:url', url)
    let canonical = document.head.querySelector<HTMLLinkElement>('link[rel="canonical"]')
    if (page === notFoundPage) {
      canonical?.remove()
    } else {
      if (!canonical) {
        canonical = document.createElement('link')
        canonical.rel = 'canonical'
        document.head.appendChild(canonical)
      }
      canonical.href = url
    }
  }, [pathname])
  return null
}
