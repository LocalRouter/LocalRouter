import { test, expect } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.goto('/demo')
  await page.getByRole('button', { name: 'MCPs', exact: true }).click()
  await page.getByTitle('Add MCP', { exact: true }).click()
})

test('Atlassian offers the official v2 endpoint and browser login', async ({ page }) => {
  await page.getByRole('button', { name: /Atlassian/ }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('https://mcp.atlassian.com/v2/mcp')
  await expect(dialog).toContainText("authenticate in your browser")
})

test('Datadog DDSQL selection changes the saved URL and starts browser OAuth', async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: any) => {
      if (command === 'create_mcp_server') w.createdMcpArgs = args
      if (command === 'start_mcp_oauth_browser_flow') w.startedMcpOAuth = args
      return original(command, args)
    }
  })
  await page.getByRole('button', { name: /Datadog/ }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('checkbox', { name: 'Core', exact: true }).uncheck()
  await dialog.getByRole('checkbox', { name: 'DDSQL', exact: true }).check()
  await expect(dialog).toContainText('mcp?toolsets=ddsql')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await page.waitForFunction(() => (window as any).createdMcpArgs)
  const created = await page.evaluate(() => (window as any).createdMcpArgs)
  expect(created.transportConfig.url).toBe('https://mcp.datadoghq.com/api/unstable/mcp-server/mcp?toolsets=ddsql')
  expect(created.authConfig.type).toBe('oauth_browser')
  await page.waitForFunction(() => (window as any).startedMcpOAuth)
})

test('custom HTTP servers expose browser login without requiring client credentials', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('select').first().selectOption('Sse')
  const auth = dialog.locator('select').nth(1)
  await expect(auth.locator('option', { hasText: 'OAuth (Browser login)' })).toHaveCount(1)
  await auth.selectOption('oauth_browser')
  await expect(dialog.locator('input[type=password]')).toHaveCount(0)
})
