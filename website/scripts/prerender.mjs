import { readFile, writeFile, mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(root, 'dist')
const escapeHtml = (value) => value.replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char])
const server = await createServer({ root, mode: 'production', server: { middlewareMode: true, hmr: false }, appType: 'custom', optimizeDeps: { noDiscovery: true, include: [] } })
try {
  const { sitePages, siteOrigin, notFoundPage, render } = await server.ssrLoadModule('/src/entry-static.tsx')
  const template = await readFile(path.join(dist, 'index.html'), 'utf8')
  function htmlFor(page, content) {
    const url = `${siteOrigin}${page.path}`
    return template
      .replace(/<title>[^<]*<\/title>/, () => `<title>${escapeHtml(page.title)}</title>`)
      .replace(/<meta\s+(name|property)="(title|description|og:title|og:description|og:url|twitter:title|twitter:description|twitter:url)"\s+content="[^"]*"\s*\/?\s*>/g, (_, attr, key) => {
        const value = key.includes('description') ? page.description : key.includes('url') ? url : page.title
        return `<meta ${attr}="${key}" content="${escapeHtml(value)}" />`
      })
      .replace(/<link rel="canonical"[^>]*>/, page === notFoundPage ? '' : `<link rel="canonical" href="${escapeHtml(url)}" />`)
      .replace('</head>', `<meta name="robots" content="${page.indexable ? 'index,follow' : 'noindex,follow'}" />\n  </head>`)
      .replace('<div id="root"></div>', () => `<div id="root">${content}</div>`)
  }
  // Render everything before writing so any rendering error fails the build.
  const output = sitePages.map((page) => ({ page, html: htmlFor(page, page.path === '/demo/' ? '' : render(page.path)) }))
  for (const { page, html } of output) {
    const directory = path.join(dist, page.path)
    await mkdir(directory, { recursive: true })
    await writeFile(path.join(directory, 'index.html'), html)
  }
  await writeFile(path.join(dist, '404.html'), htmlFor(notFoundPage, render('/404.html')))
  const intro = sitePages.find((page) => page.path === '/docs/introduction/')
  await mkdir(path.join(dist, 'docs'), { recursive: true })
  await writeFile(path.join(dist, 'docs/index.html'), htmlFor(intro, render(intro.path)).replace('</head>', '<meta http-equiv="refresh" content="0;url=/docs/introduction/" /></head>'))
  const urls = sitePages.filter((page) => page.indexable).map((page) => `  <url><loc>${escapeHtml(siteOrigin + page.path)}</loc></url>`)
  await writeFile(path.join(dist, 'sitemap.xml'), `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join('\n')}\n</urlset>\n`)
  await writeFile(path.join(dist, 'robots.txt'), `User-agent: *\nAllow: /\n\nSitemap: ${siteOrigin}/sitemap.xml\n`)
  await writeFile(path.join(dist, 'CNAME'), 'localrouter.ai\n')
  await writeFile(path.join(dist, '.nojekyll'), '')
  await mkdir(path.join(root, '.prerender'), { recursive: true })
  await writeFile(path.join(root, '.prerender/pages.json'), JSON.stringify(sitePages, null, 2))
  console.log(`Prerendered ${sitePages.length - 1} public pages; generated sitemap with ${urls.length} URLs, demo shell and 404.`)
} finally {
  await server.close()
}
