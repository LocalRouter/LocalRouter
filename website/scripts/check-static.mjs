import assert from 'node:assert/strict'
import { readFile, access } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(root, 'dist')
const pages = JSON.parse(await readFile(path.join(root, '.prerender/pages.json'), 'utf8'))
const origin = 'https://localrouter.ai'
const decode = (text) => text.replace(/&(amp|lt|gt|quot|#39);/g, (_, key) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'" })[key])
const sitemap = await readFile(path.join(dist, 'sitemap.xml'), 'utf8')
const locations = [...sitemap.matchAll(/<loc>(.*?)<\/loc>/g)].map((match) => decode(match[1]))
assert.equal(new Set(pages.map((page) => page.path)).size, pages.length, 'duplicate routes')
assert.equal(new Set(pages.map((page) => page.title)).size, pages.length, 'duplicate titles')
assert.equal(new Set(pages.map((page) => page.description)).size, pages.length, 'duplicate descriptions')
assert.deepEqual(locations.sort(), pages.filter((page) => page.indexable).map((page) => origin + page.path).sort())
assert(!sitemap.includes('<lastmod>'), 'do not fabricate page modification dates')
for (const page of pages) {
  const html = await readFile(path.join(dist, page.path, 'index.html'), 'utf8')
  const label = page.path
  assert.equal(decode(html.match(/<title>(.*?)<\/title>/s)?.[1] ?? ''), page.title, label)
  assert.equal((html.match(/rel="canonical"/g) ?? []).length, 1, label)
  assert(html.includes(`rel="canonical" href="${origin}${page.path}"`), label)
  for (const [attribute, key, value] of [
    ['name', 'description', page.description], ['property', 'og:title', page.title],
    ['property', 'og:description', page.description], ['property', 'og:url', origin + page.path],
    ['name', 'twitter:title', page.title], ['name', 'twitter:description', page.description],
    ['name', 'twitter:url', origin + page.path], ['name', 'robots', page.indexable ? 'index,follow' : 'noindex,follow'],
  ]) {
    const tags = [...html.matchAll(new RegExp(`<meta ${attribute}="${key}" content="([^"]*)"`, 'g'))]
    assert.equal(tags.length, 1, `${label} ${key}`)
    assert.equal(decode(tags[0][1]), value, `${label} ${key}`)
  }
  if (page.indexable) {
    const body = html.split('<div id="root">')[1]?.split('</body>')[0] ?? ''
    assert(body.length > 2000, `${label} empty content`)
    assert.match(body, /<h[12]\b/, `${label} missing heading`)
    const main = html.match(/<main\b[^>]*>([\s\S]*?)<\/main>/)?.[1] ?? ''
    assert(decode(main.replace(/<[^>]*>/g, '')).length > 500, `${label} missing article content`)
    assert(!html.includes('data-msg='), `${label} SSR error serialized into page`)
    for (const match of html.matchAll(/href="(\/[^"#?]*)/g)) {
      const href = decode(match[1])
      if (href.startsWith('//')) continue
      // Download files and the separately bundled Windows XP demo are assets.
      if (href.startsWith('/assets/') || href.startsWith('/winxp') || /\.[a-z]+$/i.test(href)) continue
      const route = href === '/' ? '/' : `${href.replace(/\/+$/, '')}/`
      assert(route === '/docs/' || pages.some((candidate) => candidate.path === route), `${label} broken internal link ${href}`)
    }
  }
  for (const match of html.matchAll(/(?:src|href)="(\/assets\/[^"?#]*)/g)) await access(path.join(dist, match[1]))
}
const missing = await readFile(path.join(dist, '404.html'), 'utf8')
assert(missing.includes('noindex,follow') && missing.includes('Page not found'))
assert(!missing.includes('rel="canonical"'))
const docs = await readFile(path.join(dist, 'docs/index.html'), 'utf8')
assert(docs.includes('0;url=/docs/introduction/') && docs.includes('https://localrouter.ai/docs/introduction/'))
const robots = await readFile(path.join(dist, 'robots.txt'), 'utf8')
assert(robots.includes('Allow: /') && robots.includes(`${origin}/sitemap.xml`))
console.log(`Verified ${pages.length} route outputs: content, unique metadata, canonical URLs, sitemap coverage, internal links, assets, docs alias and noindex fallback.`)
