import { expect, test } from '@playwright/test'
import type { UsageSnapshot } from '../../src/types/tauri-commands'

for (const width of [300, 360, 720]) {
  test(`daily usage chart stays inside a ${width}px card without overlapping its meter`, async ({ page }) => {
    await page.goto('/demo')
    await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
    await page.evaluate(() => {
      const w = window as unknown as {
        __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => Promise<unknown>
      }
      const original = w.__TAURI_IPC_HANDLER__
      w.__TAURI_IPC_HANDLER__ = async (cmd, args) => {
        const result = await original(cmd, args)
        if (cmd !== 'get_usage_limits') return result
        const snapshot = result as UsageSnapshot
        const account = snapshot.accounts.find((a) => a.provider === 'openai' && a.kind === 'subscription')!
        const five = account.windows.find((window) => window.id === 'five_hour')!
        const week = account.windows.find((window) => window.id === 'seven_day')!
        account.windows = [week, {
          ...five, id: 'daily', label: '1 day', window_secs: 86400,
          slots: Array.from({ length: 24 }, (_, i) => i < 22 ? i % 4 : 0),
          current_slot: 21, slot_secs: 3600,
        }, five]
        return snapshot
      }
    })
    const region = page.getByRole('region', { name: 'Usage limits' })
    await region.getByRole('button', { name: 'Refresh usage' }).click()
    const card = region.getByRole('article').filter({ has: page.getByRole('heading', { name: 'ChatGPT Pro', exact: true }) })
    await expect(card.getByRole('meter', { name: '1 day usage', exact: false })).toBeVisible()
    await card.evaluate((el, width) => {
      el.parentElement!.style.width = `${width}px`
    }, width)

    const charts = card.getByRole('img', { name: /^Per / })
    await expect(charts).toHaveCount(3)
    for (const [index, count] of [7, 24, 5].entries()) {
      const chart = charts.nth(index)
      await expect(chart.locator(':scope > div')).toHaveCount(count)
      const geometry = await chart.evaluate((el) => {
        const bounds = el.getBoundingClientRect()
        const row = el.parentElement!
        const meter = row.querySelector('[role="meter"]')!.getBoundingClientRect()
        const card = el.closest('article')!.getBoundingClientRect()
        const labels = [...el.querySelectorAll('span')]
          .filter((s) => getComputedStyle(s).visibility !== 'hidden')
          .map((s) => s.getBoundingClientRect())
        return {
          inside: [...el.children].every((bar) => {
            const r = bar.getBoundingClientRect()
            return r.left >= bounds.left - 1 && r.right <= bounds.right + 1
          }) && bounds.left >= card.left && bounds.right <= card.right,
          separated: bounds.top >= meter.bottom || bounds.left >= meter.right,
          labelsFit: labels.every((r, i) => i === 0 || r.left >= labels[i - 1].right),
          below: bounds.top >= meter.bottom,
        }
      })
      expect(geometry.inside).toBe(true)
      expect(geometry.separated).toBe(true)
      expect(geometry.labelsFit).toBe(true)
      expect(geometry.below).toBe(false)
    }
    await expect(charts.nth(1)).toHaveAccessibleName(/Per hour:.*ahead/)
    await card.evaluate((el) => { el.parentElement!.style.removeProperty('width') })
    await page.screenshot({ path: `test-results/usage-layout-${width}.png`, fullPage: true })
  })
}

for (const failStartupRead of [false, true]) {
  test(`both sections restore from the backend in a fresh browser context (failed read: ${failStartupRead})`, async ({ browser }) => {
    const saved: Record<string, unknown> = {}
    const usageKey = 'localrouter.dashboard.usage-limits.collapsed'
    const trafficKey = 'localrouter.dashboard.traffic.collapsed'
    const openSession = async (failLoad: boolean) => {
      const context = await browser.newContext()
      await context.exposeBinding('uiStateBackend', async (_source, cmd: string, args: { key: string; value: unknown }) => {
        if (cmd === 'get_ui_state') {
          if (failLoad) throw new Error('Startup read unavailable')
          return { ...saved }
        }
        saved[args.key] = args.value
        return null
      })
      // Route the demo's IPC adapter to a backend shared by fresh browser contexts.
      await context.route('**/stubs/tauri-api-core.ts*', async (route) => {
        const response = await route.fetch()
        const source = await response.text()
        const body = source.replace('return handler(cmd, args)',
          `return cmd === 'get_ui_state' || cmd === 'set_ui_state'
            ? window.uiStateBackend(cmd, args) : handler(cmd, args)`)
        expect(body).not.toBe(source)
        await route.fulfill({ response, body })
      })
      const page = await context.newPage()
      await page.goto('/demo')
      await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
      const usage = page.getByRole('button', { name: 'Usage limits', exact: true })
      const traffic = page.getByRole('button', { name: 'Traffic', exact: true })
      return { context, page, usage, traffic }
    }
    const first = await openSession(failStartupRead)
    await first.usage.click()
    await expect.poll(() => saved[usageKey]).toBe(true)
    await first.context.close()

    const second = await openSession(false)
    await expect(second.usage).toHaveAttribute('aria-expanded', 'false')
    await expect(second.traffic).toHaveAttribute('aria-expanded', 'true')
    await second.usage.click()
    await second.traffic.click()
    await expect.poll(() => saved[usageKey]).toBe(false)
    await expect.poll(() => saved[trafficKey]).toBe(true)
    await second.context.close()

    const third = await openSession(false)
    await expect(third.usage).toHaveAttribute('aria-expanded', 'true')
    await expect(third.traffic).toHaveAttribute('aria-expanded', 'false')
    await third.context.close()
  })
}

for (const width of [720, 1100, 1440]) {
  for (const count of [2, 6]) {
    test(`account grid packs ${count} cards at viewport ${width} without empty leading rows`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1080 })
      await page.goto('/demo')
      await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
      const region = page.getByRole('region', { name: 'Usage limits' })
      await expect(region.getByRole('article')).toHaveCount(6)
      if (count === 2) {
        await page.evaluate(() => {
          const w = window as any
          const original = w.__TAURI_IPC_HANDLER__
          w.__TAURI_IPC_HANDLER__ = async (cmd: string, args: unknown) => {
            const result = await original(cmd, args)
            if (cmd === 'get_usage_limits') result.accounts = result.accounts.slice(0, 2)
            return result
          }
        })
        await region.getByRole('button', { name: 'Refresh usage' }).click()
      }
      await expect(region.getByRole('article')).toHaveCount(count)
      const geometry = await region.getByRole('article').evaluateAll((cards) => {
        const grid = cards[0].parentElement!.getBoundingClientRect()
        const rects = cards.map((el) => el.getBoundingClientRect())
        const columns = rects.filter((r) => Math.abs(r.top - rects[0].top) < 1).length
        return {
          fillsWidth: Math.abs(rects[columns - 1].right - grid.right) < 1,
          uniform: rects.every((r) => Math.abs(r.width - rects[0].width) < 1),
          packed: rects.every((r, i) => Math.abs(r.left - rects[i % columns].left) < 1
            && Math.abs(r.top - rects[Math.floor(i / columns) * columns].top) < 1),
          contained: rects.every((r) => r.left >= grid.left && r.right <= grid.right + 1),
        }
      })
      expect(geometry).toEqual({ fillsWidth: true, uniform: true, packed: true, contained: true })
      await page.screenshot({ path: `test-results/usage-grid-${width}-${count}.png`, fullPage: true })
    })
  }
}
