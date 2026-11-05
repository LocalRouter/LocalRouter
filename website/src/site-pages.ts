import { sections } from './pages/Docs'
import { papers } from './pages/Research'

export const siteOrigin = 'https://localrouter.ai'

export interface SitePage {
  path: string
  title: string
  description: string
  indexable: boolean
}

export const sitePages: SitePage[] = [
  {
    path: '/',
    title: 'LocalRouter — Open-source AI Gateway, MCP & Skills',
    description: 'Manage AI providers, MCP servers and skills in one open-source desktop app. Route models, protect secrets and monitor requests locally on macOS, Windows and Linux.',
    indexable: true,
  },
  {
    path: '/download/',
    title: 'Download LocalRouter for macOS, Windows & Linux',
    description: 'Download LocalRouter, the free, open-source desktop app for AI model routing, MCP servers and skills. Available for macOS, Windows and Linux.',
    indexable: true,
  },
  {
    path: '/research/',
    title: 'LocalRouter Research — AI Routing, MCP & Security',
    description: 'Technical research behind LocalRouter: decision-model routing, unified MCP gateways, streaming JSON repair, secret redaction and context compression.',
    indexable: true,
  },
  ...papers.map((paper) => ({
    path: `/research/${paper.id}/`,
    title: `${paper.title} — LocalRouter Research`,
    description: paper.tagline,
    indexable: true,
  })),
  ...sections.map((section) => ({
    path: `/docs/${section.id}/`,
    title: `${section.title} — LocalRouter Documentation`,
    description: `Learn about ${section.title.toLowerCase()} in LocalRouter. ${section.subsections.map((subsection) => subsection.title).join(', ')}.`.slice(0, 160),
    indexable: true,
  })),
  {
    path: '/demo/',
    title: 'Interactive LocalRouter Demo',
    description: 'Explore the LocalRouter interface with bundled sample data. This interactive demo does not connect to live AI providers.',
    indexable: false,
  },
]

export function getSitePage(pathname: string): SitePage | undefined {
  const path = pathname === '/' ? '/' : `${pathname.replace(/\/+$/, '')}/`
  if (path === '/docs/') return sitePages.find((page) => page.path === '/docs/introduction/')
  return sitePages.find((page) => page.path === path)
}

export const notFoundPage: SitePage = {
  path: '/404.html',
  title: 'Page Not Found — LocalRouter',
  description: 'This LocalRouter page could not be found. Browse the documentation, research or downloads.',
  indexable: false,
}
