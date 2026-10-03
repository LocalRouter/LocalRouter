import { test, expect } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('monitor.filter', JSON.stringify({ event_types: ['llm_call'] }))
    localStorage.setItem('theme', 'light')
  })
  await page.goto('/demo')
  await page.getByRole('button', { name: 'Monitor', exact: true }).click()
})

test('content previews truncate, single-type filter hides Type, and both payloads are visible', async ({ page }) => {
  await expect(page.getByRole('columnheader', { name: 'Question', exact: true })).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'Answer', exact: true })).toBeVisible()
  await expect(page.getByRole('columnheader', { name: 'Type', exact: true })).toHaveCount(0)
  const row = page.locator('tbody tr').first()
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
  await expect(page.getByRole('tab')).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Return validation and authentication errors immediately.')
  await expect(page.getByRole('button', { name: 'Copy request', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Copy response', exact: true })).toBeVisible()
  await page.screenshot({ path: 'test-results/monitor/monitor-llm.png' })
  await page.locator('summary').filter({ hasText: 'Event metadata' }).click()
  await expect(page.getByText('mon-001', { exact: true })).toBeVisible()
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
  await expect(page.getByRole('tab')).toHaveCount(0)
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
  await page.locator('tbody tr').first().click()
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Waiting for response')
  await page.evaluate(async () => {
    // Import the same event bus used by the demo and publish a completed event.
    const eventModule: string = '/src/stubs/tauri-api-event.ts'
    const { emit } = await import(/* @vite-ignore */ eventModule)
    const mockModule: string = '/src/components/demo/mockData.ts'
    const { mockData } = await import(/* @vite-ignore */ mockModule)
    const target = window as unknown as { __TAURI_IPC_HANDLER__: (cmd: string, args: unknown) => unknown }
    const previous = target.__TAURI_IPC_HANDLER__
    const complete = mockData.monitorEvents[0]
    target.__TAURI_IPC_HANDLER__ = (cmd, args) => cmd === 'get_monitor_event_detail' ? complete : previous(cmd, args)
    await emit('monitor-event-updated', JSON.stringify(complete))
  })
  await expect(page.getByRole('region', { name: 'Response', exact: true })).toContainText('Retry only transient failures')
  await expect(page.getByText('Waiting for response…')).toHaveCount(0)
})
