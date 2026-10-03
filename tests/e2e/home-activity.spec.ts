import { expect, test } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.goto('/demo')
  await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Refresh activity' }),
  ).toBeEnabled()
})

test('time ranges use area charts and the shared Monitor snapshot opens request details', async ({
  page,
}) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  const dailyLabel = await traffic.getByRole('img').getAttribute('aria-label')
  await page.getByRole('button', { name: 'last hour', exact: true }).click()
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per 5 minutes/,
  )
  await expect(
    page.getByRole('button', { name: 'Refresh activity' }),
  ).toBeEnabled()
  expect(await traffic.getByRole('img').getAttribute('aria-label')).not.toBe(
    dailyLabel,
  )
  await expect(traffic.locator('.recharts-area-curve')).toHaveCount(2)
  await expect(traffic.locator('.recharts-bar')).toHaveCount(0)
  const snapshot = page.getByRole('region', { name: 'Monitor snapshot' })
  await expect(snapshot.getByRole('table')).toBeVisible()
  await expect(
    snapshot.getByRole('columnheader', { name: 'Question', exact: true }),
  ).toBeVisible()
  await expect(
    snapshot.getByRole('columnheader', { name: 'Answer', exact: true }),
  ).toBeVisible()
  await expect(snapshot.getByRole('row')).toHaveCount(9)
  await expect(
    snapshot.getByRole('columnheader', { name: 'Client', exact: true }),
  ).toBeVisible()
  await snapshot.getByRole('row').filter({ hasText: 'Cursor' }).first().click()
  await expect(page.getByRole('dialog')).toContainText('filesystem__read_file')
  await expect(page.getByRole('dialog')).not.toContainText(
    'Loading event details',
  )
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toHaveCount(0)
})

test('live completion wins over an older in-flight snapshot', async ({
  page,
}) => {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: unknown) => {
      if (command !== 'get_monitor_events') return original(command, args)
      return new Promise((resolve) => {
        w.resolveHomeSnapshot = () => resolve({ events: [], total: 0 })
      })
    }
  })
  await page.getByRole('button', { name: 'Refresh activity' }).click()
  await page.waitForFunction(() => (window as any).resolveHomeSnapshot)
  await page.evaluate(async () => {
    const { emit } = await import(
      /* @vite-ignore */ '/src/stubs/tauri-api-event.ts'
    )
    const event = {
      id: 'live-test',
      sequence: 900,
      timestamp: new Date().toISOString(),
      event_type: 'llm_call',
      client_id: 'test',
      client_name: 'Live client',
      session_id: null,
      status: 'pending',
      duration_ms: null,
      summary: 'A live request',
      question: 'A live question',
      answer: '',
    }
    await emit('monitor-event-created', JSON.stringify(event))
    await emit(
      'monitor-event-updated',
      JSON.stringify({
        ...event,
        status: 'complete',
        duration_ms: 750,
        answer: 'A live answer',
      }),
    )
  })
  await expect(
    page.getByRole('row').filter({ hasText: 'Live client' }),
  ).toContainText('750ms')
  await page.evaluate(() => (window as any).resolveHomeSnapshot())
  await expect(
    page.getByRole('row').filter({ hasText: 'Live client' }),
  ).toContainText('750ms')
  await expect(
    page.getByRole('row').filter({ hasText: 'Live client' }),
  ).toHaveCount(1)
})

test('empty traffic and failed loads are distinct and retry recovers', async ({
  page,
}) => {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.homeFailure = false
    w.__TAURI_IPC_HANDLER__ = (command: string, args: unknown) => {
      if (
        [
          'get_global_metrics',
          'get_global_mcp_metrics',
          'get_monitor_events',
        ].includes(command) &&
        w.homeFailure
      )
        throw new Error('Test unavailable')
      if (['get_global_metrics', 'get_global_mcp_metrics'].includes(command))
        return {
          labels: ['2026-10-02 12:00', '2026-10-02 13:00'],
          datasets: [{ label: 'Requests', data: [0, 0] }],
        }
      if (command === 'get_monitor_events') return { events: [], total: 0 }
      return original(command, args)
    }
  })
  await page.getByRole('button', { name: 'Refresh activity' }).click()
  await expect(page.getByText('No events captured yet')).toBeVisible()
  await expect(
    page.getByText('No requests in this period.', { exact: false }),
  ).toBeVisible()
  await page.evaluate(() => {
    ;(window as any).homeFailure = true
  })
  await page.getByRole('button', { name: 'Refresh activity' }).click()
  await expect(page.getByText('Request traffic is unavailable')).toBeVisible()
  await expect(
    page.getByText('Activity is unavailable', { exact: true }),
  ).toBeVisible()
  await page.evaluate(() => {
    ;(window as any).homeFailure = false
  })
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByText('No events captured yet')).toBeVisible()
})

test('missing event details have an explicit state and compact windows do not overflow', async ({
  page,
}) => {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: unknown) =>
      command === 'get_monitor_event_detail' ? null : original(command, args)
  })
  await page.getByRole('row').filter({ hasText: 'Claude Code' }).first().click()
  await expect(page.getByRole('dialog')).toContainText('no longer available')
  await page.keyboard.press('Escape')
  for (const width of [1100, 900, 700]) {
    await page.setViewportSize({ width, height: 900 })
    const dimensions = await page
      .locator('main')
      .evaluate((el) => ({ width: el.clientWidth, scroll: el.scrollWidth }))
    expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.width + 1)
  }
})
