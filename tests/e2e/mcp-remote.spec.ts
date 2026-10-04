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

test('custom discovery detects HTTP and fills server name and browser login, with editable overrides', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('URL or command', { exact: true }).fill('https://example.com/mcp?toolsets=ddsql')
  await expect(dialog.locator('select').first()).toHaveValue('Sse')
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await expect(dialog.getByRole('status')).toContainText('Discovered MCP Server')
  await expect(dialog.getByPlaceholder('My MCP Server')).toHaveValue('Discovered MCP Server')
  await expect(dialog.locator('select').nth(1)).toHaveValue('oauth_browser')
  await dialog.locator('select').nth(1).selectOption('bearer')
  await dialog.getByPlaceholder('your-bearer-token').fill('manual-token')
  await expect(dialog.locator('select').nth(1)).toHaveValue('bearer')
})

test('custom command discovery keeps environment, directory and manual server name', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('URL or command', { exact: true }).fill('python3 "my server.py"')
  await dialog.getByPlaceholder('My MCP Server').fill('My chosen name')
  await dialog.getByPlaceholder('/Users/demo', { exact: true }).fill('/tmp/mcp-project')
  await dialog.getByPlaceholder('KEY', { exact: true }).fill('SERVER_TOKEN')
  await dialog.getByPlaceholder('VALUE', { exact: true }).fill('my-credential')
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: any) => {
      if (command === 'discover_mcp_connection') w.discoveryArgs = args
      return original(command, args)
    }
  })
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await expect(dialog.getByRole('status')).toContainText('STDIO subprocess')
  await expect(dialog.getByPlaceholder('My MCP Server')).toHaveValue('My chosen name')
  const args = await page.evaluate(() => (window as any).discoveryArgs)
  expect(args.target).toBe('python3 "my server.py"')
  expect(args.transportOverride).toBe('stdio')
  expect(args.env).toEqual({ SERVER_TOKEN: 'my-credential' })
  expect(args.cwd).toBe('/tmp/mcp-project')
})

test('manual authentication survives discovery and network errors leave manual setup available', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('URL or command', { exact: true }).fill('https://example.com/mcp')
  await dialog.locator('select').nth(1).selectOption('none')
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await expect(dialog.getByRole('status')).toContainText('OAuth browser login')
  await expect(dialog.locator('select').nth(1)).toHaveValue('none')
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: any) => {
      if (command === 'discover_mcp_connection') throw new Error('Discovery unavailable')
      return original(command, args)
    }
  })
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await expect(dialog.getByRole('alert')).toBeVisible()
  await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeEnabled()
})

test('late discovery cannot overwrite a new URL or settings edited while discovering', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('URL or command', { exact: true }).fill('https://example.com/old')
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = (command: string, args: any) => {
      if (command === 'discover_mcp_connection') return new Promise(resolve => { w.finishDiscovery = async () => resolve(await original(command, args)) })
      return original(command, args)
    }
  })
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await page.waitForFunction(() => (window as any).finishDiscovery)
  await dialog.getByLabel('URL or command', { exact: true }).fill('https://example.com/new')
  await page.evaluate(() => (window as any).finishDiscovery())
  await expect(dialog.getByRole('status')).toHaveCount(0)
  await expect(dialog.locator('select').nth(1)).toHaveValue('none')
  await expect(dialog.getByPlaceholder('My MCP Server')).toHaveValue('')
  await dialog.getByRole('button', { name: 'Discover', exact: true }).click()
  await page.waitForFunction(() => (window as any).finishDiscovery)
  await dialog.getByPlaceholder('My MCP Server').fill('Edited while waiting')
  await page.evaluate(() => (window as any).finishDiscovery())
  await expect(dialog.getByRole('alert')).toContainText('your edits were kept')
  await expect(dialog.getByPlaceholder('My MCP Server')).toHaveValue('Edited while waiting')
  await expect(dialog.locator('select').nth(1)).toHaveValue('none')
})
