import { expect, type Page, test } from '@playwright/test'

const LLM_LINE = '.traffic-llm path.recharts-line-curve'
const MCP_LINE = '.traffic-mcp path.recharts-line-curve'
const IN_PROGRESS = '.traffic-in-progress path.recharts-line-curve'
const ERRORS_LINE = '.traffic-errors path.recharts-line-curve'

/** Re-read the traffic timeline now (the chart is live; switching range reloads it). */
async function reloadTraffic(page: Page) {
  await page.getByRole('button', { name: 'last hour', exact: true }).click()
  await page.getByRole('button', { name: 'last 10 minutes', exact: true }).click()
}

/** The monitor's "Failed only" filter, in the event types popover (opened by this). */
async function failedOnly(page: Page) {
  await page.getByRole('region', { name: 'Request monitor' }).getByTestId('event-type-filter').click()
  return page.getByRole('checkbox', { name: 'Failed only' })
}

/** Re-read the monitor's event snapshot (a filter change reloads it). */
async function reloadMonitor(page: Page) {
  const toggle = await failedOnly(page)
  await toggle.click()
  await toggle.click()
  await page.keyboard.press('Escape')
}

/** Serve a fixed 10-second timeline whose newest bucket is the open one. */
async function fixedTimeline(
  page: Page,
  buckets: { requests: number; errors?: number }[],
) {
  await page.evaluate((buckets) => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = async (command: string, args: unknown) => {
      if (command !== 'get_traffic_timeline') return original(command, args)
      const current = Math.floor(Date.now() / 10_000) * 10
      const counts = buckets.map(({ requests, errors = 0 }) => ({
        llm_requests: requests,
        llm_errors: errors,
        mcp_requests: 0,
        mcp_errors: 0,
        cost_usd: 0,
      }))
      return {
        bucket_secs: 10,
        buckets: counts.map((c, i) => ({
          timestamp: current - (counts.length - 1 - i) * 10,
          counts: c,
        })),
        totals: counts.reduce((a, c) => ({
          llm_requests: a.llm_requests + c.llm_requests,
          llm_errors: a.llm_errors + c.llm_errors,
          mcp_requests: 0,
          mcp_errors: 0,
          cost_usd: 0,
        })),
      }
    }
  }, buckets)
  await reloadTraffic(page)
}

test.beforeEach(async ({ page }) => {
  await page.goto('/demo')
  await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible()
  await expect(
    page.getByRole('region', { name: 'Request traffic' }).locator('.traffic-llm'),
  ).toHaveCount(1)
})

test('time ranges use a compact line chart and the merged Monitor opens request details in a pane', async ({
  page,
}) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await expect(
    page.getByRole('button', { name: 'last 10 minutes', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true')
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per 10 seconds/,
  )
  const defaultLabel = await traffic.getByRole('img').getAttribute('aria-label')
  await page.getByRole('button', { name: 'last hour', exact: true }).click()
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per minute/,
  )
  // The chart is live: no refresh control, and the totals row is shown only
  // while the section is folded.
  await expect(traffic.getByRole('button', { name: 'Refresh activity' })).toHaveCount(0)
  await expect(traffic).not.toContainText('est. cost')
  expect(await traffic.getByRole('img').getAttribute('aria-label')).not.toBe(
    defaultLabel,
  )
  // An LLM line, an MCP line, an errors line only around errors; no areas,
  // bars or markers for the requests in progress.
  await expect(traffic.locator(LLM_LINE)).toHaveCount(1)
  await expect(traffic.locator(MCP_LINE)).toHaveCount(1)
  await expect(traffic.locator(ERRORS_LINE).first()).toBeVisible()
  await expect(traffic.locator('.recharts-area-curve')).toHaveCount(0)
  await expect(traffic.locator('.recharts-bar-rectangle')).toHaveCount(0)
  await expect(traffic.getByTestId('traffic-now')).toHaveCount(0)
  // A static legend names every line; errors are read off their own axis.
  await expect(traffic.getByTestId('traffic-legend').locator(':scope > span')).toHaveText([
    'LLM requests',
    'MCP requests',
    'Errors',
  ])
  for (const label of ['Most in progress', 'Finished per', 'in progress now', 'Idle now']) {
    await expect(traffic.getByText(label)).toHaveCount(0)
  }
  expect((await traffic.getByRole('img').boundingBox())!.height).toBeLessThanOrEqual(160)
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

test('30-day traffic has 8-hour points', async ({ page }) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await page.getByRole('button', { name: 'last 30 days', exact: true }).click()
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    /per 8 hours/,
  )
})

test('traffic folds away like usage limits and remembers it', async ({ page }) => {
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  const toggle = traffic.getByRole('button', { name: 'Traffic', exact: true })
  await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  await toggle.click()
  await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  await expect(traffic.getByRole('img')).toHaveCount(0)
  // The totals stay in the header.
  await expect(traffic).toContainText('requests')
  await page.reload()
  const reloaded = page.getByRole('region', { name: 'Request traffic' })
  await expect(
    reloaded.getByRole('button', { name: 'Traffic', exact: true }),
  ).toHaveAttribute('aria-expanded', 'false')
  await reloaded.getByRole('button', { name: 'Traffic', exact: true }).click()
  await expect(reloaded.getByRole('img')).toBeVisible()
  await expect(
    page
      .getByRole('region', { name: 'Usage limits' })
      .getByRole('button', { name: 'Usage limits', exact: true }),
  ).toHaveAttribute('aria-expanded', 'true')
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
  const inProgressNow = async () =>
    Number(
      (await traffic.getByRole('img').getAttribute('aria-label'))?.match(/(\d+) in progress now/)?.[1] ?? 0,
    )
  const baseline = await inProgressNow()
  const segments = () =>
    traffic.locator(IN_PROGRESS).evaluateAll((paths) => paths.map((p) => p.getAttribute('d')).join())
  const before = await segments()
  await publish('monitor-event-created', request)
  // The in-progress segment on the LLM line rises with the new request.
  await expect.poll(segments).not.toBe(before)
  await expect(traffic.getByRole('img')).toHaveAttribute(
    'aria-label',
    new RegExp(`${baseline + 1} in progress now`),
  )
  await publish('monitor-event-updated', {
    ...request,
    status: 'complete',
    duration_ms: 500,
  })
  await expect.poll(inProgressNow).toBe(baseline)
  await expect(traffic.getByText('Idle now')).toHaveCount(0)
})

test('requests in progress stack on their line, and errors run from zero back to zero', async ({
  page,
}) => {
  await fixedTimeline(page, [
    { requests: 2 },
    { requests: 3, errors: 1 },
    { requests: 2 },
    { requests: 4 },
    { requests: 5, errors: 2 },
    { requests: 6, errors: 1 },
    { requests: 4 },
    { requests: 1 },
  ])
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await expect(traffic.getByRole('img')).toHaveAttribute('aria-label', /27 finished requests/)
  // An LLM request still running, so the LLM line has a segment in progress.
  await page.evaluate(async () => {
    const { emit } = await import(/* @vite-ignore */ '/src/stubs/tauri-api-event.ts')
    await emit(
      'monitor-event-created',
      JSON.stringify({
        id: 'stacked-in-flight',
        sequence: 902,
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
      }),
    )
  })
  await expect(traffic.getByRole('img')).toHaveAttribute('aria-label', /[1-9]\d* in progress now/)
  // In progress is drawn dashed, not only lighter.
  await expect(traffic.locator(IN_PROGRESS).first()).toHaveAttribute('stroke-dasharray', /\d/)
  // The lighter segment is shown from the previous bucket's point on the LLM
  // line to above the live bucket's point (finished so far plus in progress),
  // bending like the line and never dipping under it.
  const coords = (d: string) => d.match(/-?\d+(\.\d+)?/g)!.map(Number)
  const line = coords((await traffic.locator(LLM_LINE).getAttribute('d'))!)
  const [liveX, liveY] = line.slice(-2)
  // The last curve segment is `C cp1 cp2 end`: the previous point ends the one before.
  const [prevX] = line.slice(-8, -6)
  const segment = coords((await traffic.locator(IN_PROGRESS).first().getAttribute('d'))!)
  const [toX, toY] = segment.slice(-2)
  expect(toX).toBeCloseTo(liveX)
  expect(toY).toBeLessThan(liveY)
  const clip = await traffic
    .locator(IN_PROGRESS)
    .first()
    .evaluate((path) => {
      const rect = path.closest('g[clip-path]')
      const id = rect?.getAttribute('clip-path')?.match(/url\(#(.+)\)/)?.[1]
      return id ? Number(document.getElementById(id)?.querySelector('rect')?.getAttribute('x')) : null
    })
  expect(clip).toBeCloseTo(prevX)
  const gaps = await traffic.locator('.recharts-surface').evaluate(
    (svg, { from, to, llm, inProgress }) => {
      const sample = (path: SVGPathElement) => {
        const points: { x: number; y: number }[] = []
        const length = path.getTotalLength()
        for (let i = 0; i <= 400; i++) points.push(path.getPointAtLength((length * i) / 400))
        return points.filter((p) => p.x > from + 1 && p.x < to - 1)
      }
      const solid = sample(svg.querySelector(llm) as SVGPathElement)
      const dashed = sample(svg.querySelector(inProgress) as SVGPathElement)
      // SVG y grows downward: the dashed curve's y is at most the line's.
      return dashed.map((d) => {
        const nearest = solid.reduce((a, b) => (Math.abs(b.x - d.x) < Math.abs(a.x - d.x) ? b : a))
        return d.y - nearest.y
      })
    },
    { from: prevX, to: liveX, llm: LLM_LINE, inProgress: IN_PROGRESS },
  )
  expect(gaps.length).toBeGreaterThan(0)
  for (const gap of gaps) expect(gap).toBeLessThanOrEqual(0.5)
  // The errors line rises from zero around each run of errors and is not
  // drawn elsewhere: two runs, two pieces, each from zero back to zero.
  const pieces = traffic.locator(ERRORS_LINE)
  await expect(pieces).toHaveCount(2)
  for (const d of await pieces.evaluateAll((paths) => paths.map((p) => p.getAttribute('d')!))) {
    expect(d.match(/M/g)).toHaveLength(1)
  }
  // Error counts live in the tooltip, not as numbers over the peaks.
  await expect(traffic.locator('.recharts-label-list text')).toHaveCount(0)
  // Hovering the open bucket explains the provisional value.
  const grid = (await traffic.locator('.recharts-cartesian-grid').boundingBox())!
  await page.mouse.move(grid.x + grid.width - 1, grid.y + grid.height / 2)
  const tooltip = traffic.locator('.recharts-tooltip-wrapper')
  await expect(tooltip).toContainText('Now (in progress)')
  await expect(tooltip).toContainText('LLM requests1')
  await expect(tooltip).toContainText(/LLM in progress\d+/)
  // A finished bucket keeps the usual rows.
  await page.mouse.move(grid.x + (grid.width * 4) / 7, grid.y + grid.height / 2)
  await expect(tooltip).not.toContainText('Now (in progress)')
  await expect(tooltip).toContainText('LLM requests5')
  await expect(tooltip).toContainText('MCP requests0')
  await expect(tooltip).toContainText('Errors2')
})

test('a range without errors draws no error marks', async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = async (command: string, args: unknown) => {
      const result = await original(command, args)
      if (command !== 'get_traffic_timeline') return result
      for (const bucket of result.buckets) {
        if (bucket.counts) Object.assign(bucket.counts, { llm_errors: 0, mcp_errors: 0 })
      }
      Object.assign(result.totals, { llm_errors: 0, mcp_errors: 0 })
      return result
    }
  })
  await reloadTraffic(page)
  const traffic = page.getByRole('region', { name: 'Request traffic' })
  await expect(traffic.getByRole('img')).toHaveAttribute('aria-label', /, 0 errors\./)
  await expect(traffic.locator(ERRORS_LINE)).toHaveCount(0)
  await expect(traffic.locator('.recharts-label-list text')).toHaveCount(0)
  await expect(traffic.locator(LLM_LINE)).toHaveCount(1)
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
  await reloadMonitor(page)
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
        ['get_traffic_timeline', 'get_monitor_events'].includes(command) &&
        w.homeFailure
      )
        throw new Error('Test unavailable')
      if (command === 'get_traffic_timeline') {
        const zero = {
          llm_requests: 0,
          llm_errors: 0,
          mcp_requests: 0,
          mcp_errors: 0,
          cost_usd: 0,
        }
        const now = Math.floor(Date.now() / 10_000) * 10
        return {
          bucket_secs: 10,
          buckets: [
            { timestamp: now - 10, counts: zero },
            { timestamp: now, counts: zero },
          ],
          totals: zero,
        }
      }
      if (command === 'get_monitor_events') return { events: [], total: 0 }
      return original(command, args)
    }
  })
  // Settle the demo's in-flight requests: running requests are not "no traffic".
  await page.evaluate(async () => {
    const { emit } = await import(
      /* @vite-ignore */ '/src/stubs/tauri-api-event.ts'
    )
    const mockModule: string = '/src/components/demo/mockData.ts'
    const { mockData } = await import(/* @vite-ignore */ mockModule)
    const live = mockData.monitorEvents.filter(
      (event: { status: string }) => event.status === 'pending',
    )
    for (const event of live) {
      await emit(
        'monitor-event-updated',
        JSON.stringify({ ...event, data: undefined, status: 'complete', duration_ms: 10 }),
      )
    }
  })
  await reloadTraffic(page)
  await reloadMonitor(page)
  await expect(page.getByText('No events captured yet')).toBeVisible()
  await expect(
    page.getByText('No requests in this period.', { exact: false }),
  ).toBeVisible()
  await page.evaluate(() => {
    ;(window as any).homeFailure = true
  })
  await reloadTraffic(page)
  await reloadMonitor(page)
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

test('the errors filter shows only failed events, live ones included, and is remembered', async ({
  page,
}) => {
  const monitor = page.getByRole('region', { name: 'Request monitor' })
  const rows = monitor.locator('tbody tr')
  await expect(monitor).toContainText('filesystem__read_file')
  const failed = await page.evaluate(async () => {
    const mockModule: string = '/src/components/demo/mockData.ts'
    const { mockData } = await import(/* @vite-ignore */ mockModule)
    return mockData.monitorEvents.filter((event: { status: string }) => event.status === 'error')
      .length as number
  })
  expect(failed).toBeGreaterThan(1)
  const toggle = await failedOnly(page)
  await expect(toggle).not.toBeChecked()
  await toggle.click()
  await expect(toggle).toBeChecked()
  await page.keyboard.press('Escape')
  await expect(monitor.getByTestId('event-type-filter')).toContainText('Failed')
  // Failed LLM calls and error events of every type, nothing that succeeded.
  await expect(rows).toHaveCount(failed)
  await expect(monitor).not.toContainText('filesystem__read_file')
  await expect(monitor).toContainText('Connection refused')
  await expect(monitor).toContainText('Rate limit exceeded')
  // Live events: a running request is hidden until it fails.
  const request = {
    id: 'live-failure',
    sequence: 990,
    timestamp: new Date().toISOString(),
    event_type: 'llm_call',
    client_id: 'test',
    client_name: 'Live client',
    session_id: null,
    status: 'pending',
    duration_ms: null,
    summary: 'A request that fails',
    question: 'Will this one fail?',
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
  await publish('monitor-event-created', request)
  await expect(
    page.getByRole('region', { name: 'Request traffic' }).getByRole('img'),
  ).toHaveAttribute('aria-label', /[1-9]\d* in progress now/)
  await expect(monitor).not.toContainText('Will this one fail?')
  await publish('monitor-event-updated', { ...request, status: 'error', duration_ms: 20 })
  await expect(monitor).toContainText('Will this one fail?')
  await expect(rows).toHaveCount(failed + 1)
  // Remembered across reloads, like the other filters.
  await page.reload()
  const reloaded = page.getByRole('region', { name: 'Request monitor' })
  await expect(reloaded.locator('tbody tr')).toHaveCount(failed)
  const reloadedToggle = await failedOnly(page)
  await expect(reloadedToggle).toBeChecked()
  await reloadedToggle.click()
  await page.keyboard.press('Escape')
  await expect(reloaded).toContainText('filesystem__read_file')
})

test('a filter with no matches says so instead of "no events captured"', async ({ page }) => {
  const monitor = page.getByRole('region', { name: 'Request monitor' })
  await monitor.getByPlaceholder('Search...').fill('no event says this')
  await expect(monitor).toContainText('No events match these filters')
  await expect(monitor).not.toContainText('No events captured yet')
})
