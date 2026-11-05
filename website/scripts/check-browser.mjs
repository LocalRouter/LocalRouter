import assert from 'node:assert/strict'
import { readFile, stat } from 'node:fs/promises'
import http from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from '@playwright/test'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(root, 'dist')
const pages = JSON.parse(await readFile(path.join(root, '.prerender/pages.json'), 'utf8'))
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png' }
const server = http.createServer(async (request, response) => {
  try {
    let filename = path.join(dist, decodeURIComponent(new URL(request.url, 'http://localhost').pathname))
    if (!filename.startsWith(dist + path.sep)) throw new Error('outside build')
    if ((await stat(filename)).isDirectory()) filename = path.join(filename, 'index.html')
    response.setHeader('Content-Type', types[path.extname(filename)] ?? 'application/octet-stream')
    response.end(await readFile(filename))
  } catch {
    response.writeHead(404, { 'Content-Type': 'text/html' })
    response.end(await readFile(path.join(dist, '404.html')))
  }
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const base = `http://127.0.0.1:${server.address().port}`
let browser
try {
  browser = await chromium.launch({ channel: 'chromium' })
  const noJs = await browser.newContext({ javaScriptEnabled: false })
  const page = await noJs.newPage()
  for (const route of pages.filter((candidate) => candidate.indexable)) {
    const response = await page.goto(base + route.path)
    assert.equal(response.status(), 200, route.path)
    assert.equal(await page.title(), route.title)
    assert.equal(await page.locator('link[rel="canonical"]').getAttribute('href'), `https://localrouter.ai${route.path}`)
    assert((await page.locator('main').innerText()).length > 500, `${route.path} needs JavaScript for content`)
  }
  const missingResponse = await page.goto(base + '/does-not-exist/')
  assert.equal(missingResponse.status(), 404)
  assert.equal(await page.locator('h1').innerText(), 'Page not found')
  const client = await browser.newContext()
  const live = await client.newPage()
  const errors = []
  live.on('pageerror', (error) => errors.push(error.message))
  await live.goto(base + '/')
  await live.getByRole('navigation').getByRole('link', { name: 'Research', exact: true }).click()
  await live.waitForURL('**/research')
  await live.waitForFunction(() => document.title.includes('LocalRouter Research'))
  await live.getByRole('link', { name: /Reversible Secret Redaction Keeping/ }).click()
  await live.waitForURL('**/research/reversible-secret-redaction')
  await live.waitForFunction(() => document.querySelector('link[rel="canonical"]').href === 'https://localrouter.ai/research/reversible-secret-redaction/')
  await live.getByRole('navigation').getByRole('link', { name: 'Download', exact: true }).first().click()
  await live.waitForFunction(() => document.title.includes('Download LocalRouter'))
  assert.equal(await live.locator('meta[name="robots"]').getAttribute('content'), 'index,follow')
  await live.goto(base + '/demo/')
  await live.waitForFunction(() => document.title === 'Interactive LocalRouter Demo')
  await live.waitForFunction(() => document.querySelector('#root').textContent.length > 100)
  assert.equal(await live.locator('meta[name="robots"]').getAttribute('content'), 'noindex,follow')
  await live.goto(base + '/docs/not-a-section/')
  await live.waitForFunction(() => document.title === 'Page Not Found — LocalRouter')
  assert.equal(await live.locator('h1').innerText(), 'Page not found')
  assert.equal(await live.locator('link[rel="canonical"]').count(), 0)
  await live.getByRole('link', { name: 'Home', exact: true }).last().click()
  await live.waitForFunction(() => document.querySelector('meta[name="robots"]').content === 'index,follow')
  await live.goto(base + '/docs/firewall/#approval-flow')
  await live.waitForFunction(() => document.querySelector('#approval-flow') !== null)
  await live.getByText('Tool Approval', { exact: true }).first().waitFor()
  assert.deepEqual(errors, [], 'browser runtime errors')
  console.log(`Browser checks passed: ${pages.filter((candidate) => candidate.indexable).length} pages without JavaScript, 404, client navigation metadata, demo noindex, recovery and interactive docs.`)
} finally {
  await browser?.close()
  await new Promise((resolve) => server.close(resolve))
}
