import { codexRequest, codexStream, codexTruncated } from './codex-fixture'
import { test, expect } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('monitor.filter', JSON.stringify({ event_types: ['llm_call'] }))
    localStorage.setItem('theme', 'light')
  })
  await page.goto('/demo')
  await expect(page.getByRole('region', { name: 'Request monitor' })).toBeVisible()
})

test('content previews truncate, single-type filter hides Type, and both payloads are visible', async ({ page }) => {
  await expect(page.getByRole('columnheader', { name: 'Question', exact: true })).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'Answer', exact: true })).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'Type', exact: true })).toHaveCount(0)
  const row = page.getByRole('row').filter({ hasText: 'How should I retry failed API requests?' })
  const question = row.locator('td').nth(3)
  expect(await question.evaluate(el => ({
    overflow: el.scrollWidth > el.clientWidth,
    ellipsis: getComputedStyle(el).textOverflow,
    whitespace: getComputedStyle(el).whiteSpace,
    text: el.textContent,
  }))).toMatchObject({ overflow: true, ellipsis: 'ellipsis', whitespace: 'nowrap' })
  await row.click()
  await expect(page.getByRole('region', { name: 'Request', exact: true })).toBeVisible()
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toBeVisible()
  await expect(page.getByRole('tab', { name: 'Request & response' })).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Return validation and authentication errors immediately.')
  await expect(page.getByRole('button', { name: 'Copy request', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Copy response', exact: true })).toBeVisible()
  await page.screenshot({ path: 'test-results/monitor/monitor-llm.png' })
  await page.getByRole('tab', { name: 'Details' }).click()
  await expect(page.getByText('mon-001', { exact: true })).toBeVisible()
  await expect(page.getByRole('region', { name: 'Usage' })).toContainText('800 (67%)')
  // The chosen tab carries over to the next event that has it.
  await page.getByRole('row').filter({ hasText: 'Summarize the open pull requests.' }).click()
  await expect(page.getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true')
  await page.getByRole('tab', { name: 'Request & response' }).click()
})

test('multiple types restore Type and errors are shown within Response', async ({ page }) => {
  await page.getByRole('button', { name: 'LLM', exact: true }).click()
  await page.getByLabel('MCP', { exact: true }).click()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('columnheader', { name: 'Type', exact: true })).toBeVisible()
  await page.getByRole('row').filter({ hasText: 'GPT4All' }).click()
  const response = page.getByRole('region', { name: 'Response', exact: true })
  await expect(response).toContainText('Connection refused')
  await expect(response).toContainText('Request failed')
  await page.screenshot({ path: 'test-results/monitor/monitor-error.png' })
})

test('narrow detail panes stack the exchange without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 850, height: 950 })
  await page.locator('tbody tr').first().click()
  const request = await page.getByRole('region', { name: 'Request', exact: true }).boundingBox()
  const response = await page.getByRole('region', { name: 'Response', exact: true }).boundingBox()
  expect(response!.y).toBeGreaterThanOrEqual(request!.y + request!.height)
  expect(await page.getByTestId('event-detail-scroll').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: 'test-results/monitor/monitor-narrow.png' })
})

test('a pending selection updates to its response without switching a tab', async ({ page }) => {
  await page.evaluate(async () => {
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const original = target.__TAURI_IPC_HANDLER__
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => {
      const result = original(cmd, args)
      if (cmd === 'get_monitor_event_detail') {
        const event = result as { status: string; data: Record<string, unknown> }
        return { ...event, status: 'pending', data: { ...event.data, provider: undefined, response_body: undefined, content_preview: undefined, status_code: undefined } }
      }
      return result
    }
  })
  await page.getByRole('row').filter({ hasText: 'How should I retry failed API requests?' }).click()
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Waiting for response')
  await page.evaluate(async () => {
    // Import the same event bus used by the demo and publish a completed event.
    const eventModule: string = '/src/stubs/tauri-api-event.ts'
    const { emit } = await import(/* @vite-ignore */ eventModule)
    const mockModule: string = '/src/components/demo/mockData.ts'
    const { mockData } = await import(/* @vite-ignore */ mockModule)
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const previous = target.__TAURI_IPC_HANDLER__
    const complete = mockData.monitorEvents.find((event: { id: string }) => event.id === 'mon-001')
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => cmd === 'get_monitor_event_detail' ? complete : previous(cmd, args)
    await emit('monitor-event-updated', JSON.stringify(complete))
  })
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Retry only transient failures')
  await expect(page.getByText('Waiting for response…')).toHaveCount(0)
})

for (const status of ['complete', 'error'] as const) {
  test(`duration increments in the list and detail until the request is ${status}`, async ({ page }) => {
    const start = new Date('2026-10-03T12:00:00Z')
    await page.clock.install({ time: start })
    await page.clock.pauseAt(start)
    await page.evaluate(async () => {
      const eventModule: string = '/src/stubs/tauri-api-event.ts'
      const { emit } = await import(/* @vite-ignore */ eventModule)
      const mockModule: string = '/src/components/demo/mockData.ts'
      const { mockData } = await import(/* @vite-ignore */ mockModule)
      const pending = { ...mockData.monitorEvents.find((event: { id: string }) => event.id === 'mon-001'),
        timestamp: new Date().toISOString(), status: 'pending', duration_ms: null }
      const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
      const original = target.__TAURI_IPC_HANDLER__
      target.__TAURI_IPC_HANDLER__ = (cmd, args) => cmd === 'get_monitor_event_detail' ? pending : original(cmd, args)
      await emit('monitor-event-updated', JSON.stringify(pending))
    })
    await page.clock.runFor(0)
    const row = page.getByRole('row').filter({ hasText: 'How should I retry failed API requests?' })
    const duration = row.locator('td').last()
    await expect(duration).toHaveText('0ms')
    await row.click()
    const detailDuration = page.getByTestId('event-detail-duration')
    await expect(detailDuration).toHaveText('0ms')
    await page.clock.runFor(1000)
    await expect(duration).toHaveText('1000ms')
    await expect(detailDuration).toHaveText('1000ms')
    await page.clock.runFor(1000)
    await expect(duration).toHaveText('2000ms')
    await expect(detailDuration).toHaveText('2000ms')
    await page.evaluate(async status => {
      const eventModule: string = '/src/stubs/tauri-api-event.ts'
      const { emit } = await import(/* @vite-ignore */ eventModule)
      const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
      const terminal = { ...(target.__TAURI_IPC_HANDLER__('get_monitor_event_detail', {}) as object), status, duration_ms: 1987 }
      const original = target.__TAURI_IPC_HANDLER__
      target.__TAURI_IPC_HANDLER__ = (cmd, args) => cmd === 'get_monitor_event_detail' ? terminal : original(cmd, args)
      await emit('monitor-event-updated', JSON.stringify(terminal))
    }, status)
    await page.clock.runFor(0)
    await expect(duration).toHaveText('1987ms')
    await expect(detailDuration).toHaveText('1987ms')
    await page.clock.runFor(2000)
    await expect(duration).toHaveText('1987ms')
    await expect(detailDuration).toHaveText('1987ms')
  })
}


test('Codex request and streamed answer display from truncated legacy captures', async ({ page }) => {
  await page.evaluate(({ request, stream, truncated }) => {
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const original = target.__TAURI_IPC_HANDLER__
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => {
      const result = original(cmd, args)
      if (cmd !== 'get_monitor_event_detail') return result
      const event = result as { data: Record<string, unknown> }
      return { ...event, status: 'complete', client_name: 'Codex', data: { ...event.data, model: 'codex-auto-review', provider: 'chatgpt.com', status_code: 200, input_tokens: 100, output_tokens: 20, finish_reason: 'completed', streamed: true, endpoint: '/backend-api/codex/responses', request_body: truncated,
        raw_request: JSON.stringify(request), response_body: truncated, raw_response: stream, content_preview: null, error: null } }
    }
  }, { request: codexRequest, stream: codexStream, truncated: codexTruncated })
  await page.locator('tbody tr').first().click()
  await expect(page.getByRole('region', { name: 'Request', exact: true })).toContainText('Assess this synthetic request.')
  const response = page.getByRole('region', { name: 'Response', exact: true })
  await expect(response).toContainText('Synthetic assessment')
  await expect(response).not.toContainText('synthetic-ciphertext')
  await expect(response).not.toContainText('_truncated')
  await page.screenshot({ path: 'test-results/monitor/monitor-codex-lite.png' })
})

test('the selected event shows below the list and ignores late details', async ({ page }) => {
  await page.locator('tbody tr').first().click()
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toBeVisible()
  await expect(page.getByRole('separator')).toHaveCount(1)
  const list = (await page.getByTestId('monitor-event-list').boundingBox())!
  const detail = page.getByTestId('monitor-event-detail')
  expect((await detail.boundingBox())!.y).toBeGreaterThanOrEqual(list.y + list.height - 1)
  await page.evaluate(() => {
    type Pending = { resolve: (value: unknown) => void; result: { data: Record<string, unknown> } }
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown; monitorPending: Pending[] }
    const original = target.__TAURI_IPC_HANDLER__
    target.monitorPending = []
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => {
      const result = original(cmd, args)
      return cmd === 'get_monitor_event_detail' ? new Promise(resolve => target.monitorPending.push({ resolve, result: result as Pending['result'] })) : result
    }
  })
  await page.locator('tbody tr').nth(1).click()
  await expect(page.getByRole('status').filter({ hasText: 'Loading event details' })).toBeVisible()
  await page.locator('tbody tr').nth(2).click()
  await page.evaluate(() => {
    const target = window as unknown as { monitorPending: { resolve: (value: unknown) => void; result: { data: Record<string, unknown> } }[] }
    const latest = target.monitorPending[1]
    latest.resolve({ ...latest.result, data: { ...latest.result.data, request_body: { input: 'Latest selection' }, response_body: { output_text: 'Latest answer' } } })
  })
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Latest answer')
  await page.evaluate(() => {
    const target = window as unknown as { monitorPending: { resolve: (value: unknown) => void; result: { data: Record<string, unknown> } }[] }
    const old = target.monitorPending[0]
    old.resolve({ ...old.result, data: { ...old.result.data, response_body: { output_text: 'Stale answer' } } })
  })
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Latest answer')
  await expect(page.getByText('Stale answer')).toHaveCount(0)
})

test('failed and missing detail loads keep the pane available for retry', async ({ page }) => {
  await page.evaluate(() => {
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const original = target.__TAURI_IPC_HANDLER__
    let calls = 0
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => {
      if (cmd === 'get_monitor_event_detail') {
        if (++calls === 1) return Promise.reject(new Error('Synthetic failure'))
        if (calls === 2) return null
      }
      return original(cmd, args)
    }
  })
  await page.locator('tbody tr').first().click()
  await expect(page.getByRole('status')).toContainText('Unable to load event details.')
  await expect(page.getByTestId('monitor-event-detail')).toBeVisible()
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('This event is no longer available.')
  await page.getByRole('button', { name: 'Retry', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toBeVisible()
})


test('large captured bodies show labeled question and answer excerpts when raw content is unavailable', async ({ page }) => {
  await page.evaluate(() => {
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const original = target.__TAURI_IPC_HANDLER__
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => {
      const result = original(cmd, args)
      if (cmd !== 'get_monitor_event_detail') return result
      const event = result as { data: Record<string, unknown> }
      const marker = { _truncated: true, _original_size: 100_000, _preview: '{partial',
        _monitor_preview: { question: 'Question excerpt', answer: 'Answer excerpt' } }
      return { ...event, data: { ...event.data, request_body: marker, response_body: marker, raw_request: undefined, raw_response: undefined, content_preview: null } }
    }
  })
  await page.locator('tbody tr').first().click()
  await expect(page.getByRole('region', { name: 'Request', exact: true })).toContainText('Question excerpt')
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Answer excerpt')
  await expect(page.getByText('Captured excerpt', { exact: true })).toHaveCount(2)
})

test('a translated call shows its client and upstream APIs in the list and detail tabs', async ({ page }) => {
  const row = page.getByRole('row').filter({ hasText: 'Summarize the open pull requests.' })
  await expect(row).toContainText('Responses')
  await expect(row).toContainText('→ Messages')
  await row.click()
  await expect(page.getByTestId('api-flow')).toContainText('Responses')
  await expect(page.getByTestId('api-flow')).toContainText('Anthropic Messages')
  await expect(page.getByTestId('api-flow')).toContainText('translated')
  await page.getByRole('tab', { name: 'Details' }).click()
  await expect(page.getByTestId('api-path')).toContainText('Codex')
  await expect(page.getByText('LocalRouter translated this request from Responses to Anthropic Messages', { exact: false })).toBeVisible()
  await expect(page.getByRole('region', { name: 'Model' })).toContainText('localrouter/auto')
  await page.screenshot({ path: 'test-results/monitor/monitor-details.png' })
  await page.getByRole('tab', { name: /Routing/ }).click()
  await expect(page.getByRole('region', { name: 'Decision' })).toContainText('Multi-step repository task')
  await page.getByRole('tab', { name: /Parameters & tools/ }).click()
  await expect(page.getByText('list_pull_requests')).toBeVisible()
  await page.getByRole('tab', { name: /Transformations/ }).click()
  await expect(page.getByRole('region', { name: 'Sent upstream' })).toBeVisible()
  // Keyboard navigation moves between tabs.
  await page.getByRole('tab', { name: /Transformations/ }).press('ArrowRight')
  await expect(page.getByRole('tab', { name: 'Raw' })).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('region', { name: 'Full event' })).toBeVisible()
  await page.screenshot({ path: 'test-results/monitor/monitor-translated.png' })
})

test('the detail docks on the right by toggle or by dragging, and back below', async ({ page }) => {
  await page.locator('tbody tr').first().click()
  const list = page.getByTestId('monitor-event-list')
  const detail = page.getByTestId('monitor-event-detail')
  await page.getByRole('button', { name: 'Dock details on the right' }).click()
  let l = (await list.boundingBox())!
  let d = (await detail.boundingBox())!
  expect(d.x).toBeGreaterThanOrEqual(l.x + l.width - 1)
  expect(await page.evaluate(() => localStorage.getItem('monitor.detailDock'))).toBe('right')
  await page.screenshot({ path: 'test-results/monitor/monitor-docked-right.png' })

  // Drag the grip towards the bottom edge to dock below again.
  const grip = page.getByRole('button', { name: 'Drag to move details' })
  const box = (await grip.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  const body = (await page.getByRole('region', { name: 'Request monitor' }).boundingBox())!
  await page.mouse.move(body.x + body.width / 2, body.y + body.height - 10, { steps: 8 })
  await expect(page.getByText('Dock below')).toBeVisible()
  await page.mouse.up()
  l = (await list.boundingBox())!
  d = (await detail.boundingBox())!
  expect(d.y).toBeGreaterThanOrEqual(l.y + l.height - 1)

  await page.getByRole('button', { name: 'Close details' }).click()
  await expect(detail).toHaveCount(0)
})
