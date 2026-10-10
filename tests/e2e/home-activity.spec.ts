import { expect, test } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.goto('/demo')
  await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
  await expect(
    page.getByRole('button', { name: 'Refresh activity' }),
  ).toBeEnabled()
})

test('time ranges use a compact bar chart and the merged Monitor opens request details in a pane', async ({
  page,
}) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await expect(
    page.getByRole('button', { name: 'last 10 minutes', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true')
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per minute/,
  )
  const defaultLabel = await traffic.getByRole('img').getAttribute('aria-label')
  await page.getByRole('button', { name: 'last hour', exact: true }).click()
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per 5 minutes/,
  )
  await expect(
    page.getByRole('button', { name: 'Refresh activity' }),
  ).toBeEnabled()
  expect(await traffic.getByRole('img').getAttribute('aria-label')).not.toBe(
    defaultLabel,
  )
  await expect(traffic.locator('.recharts-bar-rectangle').first()).toBeVisible()
  await expect(traffic.locator('.recharts-area-curve')).toHaveCount(0)
  expect((await traffic.getByRole('img').boundingBox())!.height).toBeLessThanOrEqual(120)
  const monitor = page.getByRole('region', { name: 'Request monitor' })
  await expect(monitor.getByRole('table')).toBeVisible()
  await expect(
    monitor.getByRole('columnheader', { name: 'Question', exact: true }),
  ).toBeVisible()
  await expect(
    monitor.getByRole('columnheader', { name: 'Answer', exact: true }),
  ).toBeVisible()
  await expect(
    monitor.getByRole('columnheader', { name: 'Client', exact: true }),
  ).toBeVisible()
  await expect(monitor.getByRole('button', { name: 'Clear' })).toHaveCount(0)
  await expect(
    monitor.getByRole('button', { name: 'Intercept' }),
  ).toBeVisible()
  await expect(monitor.getByPlaceholder('Search...')).toBeVisible()
  await monitor.getByRole('row').filter({ hasText: 'Cursor' }).first().click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(monitor).toContainText('filesystem__read_file')
})

test('30-day traffic has one point per day', async ({ page }) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await page.getByRole('button', { name: 'last 30 days', exact: true }).click()
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per day/,
  )
})

test('the page fills the window and the detail splits the monitor without overlapping', async ({
  page,
}) => {
  const monitor = page.getByRole('region', { name: 'Request monitor' })
  const region = (await monitor.boundingBox())!
  expect(region.y + region.height).toBeLessThanOrEqual(1080 + 1)
  await monitor.locator('tbody tr').first().click()
  const list = (await monitor.getByTestId('monitor-event-list').boundingBox())!
  const detail = (await monitor.getByTestId('monitor-event-detail').boundingBox())!
  expect(detail.y).toBeGreaterThanOrEqual(list.y + list.height - 1)
  expect(detail.y + detail.height).toBeLessThanOrEqual(region.y + region.height + 1)
  await expect(monitor.getByRole('separator')).toHaveCount(1)
  await monitor.getByRole('button', { name: 'Try It Out' }).click()
  await expect(monitor.getByText('Select a client to get started')).toBeVisible()
})

test('in-flight requests are shown live on the chart and clear when they finish', async ({
  page,
}) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  const request = {
    id: 'in-flight-test',
    sequence: 901,
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
  const publish = (name: string, payload: object) =>
    page.evaluate(
      async ({ name, payload }) => {
        const { emit } = await import(
          /* @vite-ignore */ '/src/stubs/tauri-api-event.ts'
        )
        await emit(name, JSON.stringify(payload))
      },
      { name, payload },
    )
  const baseline = Number(
    (await traffic.getByText(/in progress|Idle/).textContent())?.match(/\d+/)?.[0] ?? 0,
  )
  await publish('monitor-event-created', request)
  await expect(traffic).toContainText(`${baseline + 1} in progress`)
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    new RegExp(`${baseline + 1} in progress`),
  )
  await publish('monitor-event-updated', {
    ...request,
    status: 'complete',
    duration_ms: 500,
  })
  await expect(traffic).not.toContainText(`${baseline + 1} in progress`)
})

test('the monitor filter selection is remembered across reloads', async ({
  page,
}) => {
  const monitor = page.getByRole('region', { name: 'Request monitor' })
  await monitor.getByPlaceholder('Search...').fill('cursor')
  await monitor.getByRole('button', { name: 'All Events' }).click()
  await page.getByLabel('Proxy Passthrough', { exact: true }).click()
  await page.keyboard.press('Escape')
  await expect(
    monitor.getByRole('button', { name: '9 types' }),
  ).toBeVisible()
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem('monitor.filter') ?? '{}')
            .event_types?.length,
      ),
    )
    .toBe(25)
  await page.reload()
  await expect(
    page.getByRole('region', { name: 'Request monitor' }).getByPlaceholder('Search...'),
  ).toHaveValue('cursor')
  await expect(
    page.getByRole('region', { name: 'Request monitor' }).getByRole('button', { name: '9 types' }),
  ).toBeVisible()
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
  // Settle the demo's in-flight request: a running request is not "no traffic".
  await page.evaluate(async () => {
    const { emit } = await import(
      /* @vite-ignore */ '/src/stubs/tauri-api-event.ts'
    )
    const mockModule: string = '/src/components/demo/mockData.ts'
    const { mockData } = await import(/* @vite-ignore */ mockModule)
    const live = mockData.monitorEvents.find(
      (event: { id: string }) => event.id === 'mon-live',
    )
    await emit(
      'monitor-event-updated',
      JSON.stringify({ ...live, data: undefined, status: 'complete', duration_ms: 10 }),
    )
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
    page.getByText('Activity is unavailable.', { exact: false }),
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
  await expect(
    page.getByRole('region', { name: 'Request monitor' }),
  ).toContainText('no longer available')
  for (const width of [1100, 900, 700]) {
    await page.setViewportSize({ width, height: 900 })
    const dimensions = await page
      .locator('main')
      .evaluate((el) => ({ width: el.clientWidth, scroll: el.scrollWidth }))
    expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.width + 1)
  }
})
