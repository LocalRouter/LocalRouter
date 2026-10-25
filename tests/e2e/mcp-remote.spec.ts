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

test('custom HTTP servers allow optional registered browser OAuth credentials', async ({ page }) => {
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.locator('select').first().selectOption('Sse')
  const auth = dialog.locator('select').nth(1)
  await expect(auth.locator('option', { hasText: 'OAuth (Browser login)' })).toHaveCount(1)
  await auth.selectOption('oauth_browser')
  await expect(dialog.getByPlaceholder('your-oauth-client-id')).not.toHaveAttribute('required', '')
  await expect(dialog.getByPlaceholder('your-oauth-client-secret')).not.toHaveAttribute('required', '')
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

async function captureAuthUpdates(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const w = window as any
    const original = w.__TAURI_IPC_HANDLER__
    w.authUpdates = []
    w.__TAURI_IPC_HANDLER__ = async (command: string, args: any) => {
      if (command === 'create_mcp_server') w.createdMcpArgs = args
      if (command === 'start_mcp_oauth_browser_flow') w.startedMcpOAuth = args
      if (command === 'update_mcp_server') {
        w.authUpdates.push(structuredClone(args))
        if (w.holdAuthSave) {
          w.holdAuthSave = false
          await new Promise<void>(resolve => { w.releaseAuthSave = resolve })
        }
        if (w.failAuthSave) throw new Error('Simulated configuration write failure')
      }
      return original(command, args)
    }
  })
}

async function createPregeneratedServer(page: import('@playwright/test').Page) {
  await captureAuthUpdates(page)
  await page.getByRole('tab', { name: 'Custom', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByPlaceholder('My MCP Server').fill('OAuth regression server')
  await dialog.locator('select').first().selectOption('Sse')
  await dialog.getByPlaceholder('https://api.example.com/mcp').fill('https://example.com/mcp')
  await dialog.locator('select').nth(1).selectOption('oauth_pregenerated')
  await dialog.getByPlaceholder('your-oauth-client-id').fill('registered-client')
  await dialog.getByPlaceholder('your-oauth-client-secret').fill('test-secret')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  await page.getByRole('tab', { name: 'Settings', exact: true }).last().click()
}

test('Slack template accepts app credentials and begins browser authorization with the saved app', async ({ page }) => {
  await captureAuthUpdates(page)
  await page.getByRole('dialog').getByRole('button', { name: /slack logo Slack/i }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('https://mcp.slack.com/mcp')
  await dialog.getByLabel('Client ID', { exact: true }).fill('slack-app-client')
  await dialog.getByLabel('Client Secret', { exact: true }).fill('slack-app-secret')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await page.waitForFunction(() => (window as any).createdMcpArgs)
  const created = await page.evaluate(() => (window as any).createdMcpArgs)
  expect(created.transportConfig.url).toBe('https://mcp.slack.com/mcp')
  expect(created.authConfig).toMatchObject({ type: 'oauth_browser', client_id: 'slack-app-client', client_secret: 'slack-app-secret' })
  await page.waitForFunction(() => (window as any).startedMcpOAuth?.serverId)
})

test('pre-generated OAuth reloads from o_auth, auto-saves credential edits, and clears auth without JSON null', async ({ page }) => {
  await createPregeneratedServer(page)
  const auth = page.locator('select').nth(1)
  await expect(auth).toHaveValue('oauth_pregenerated')
  await expect(page.getByPlaceholder('your-oauth-client-id')).toHaveValue('registered-client')
  await page.getByPlaceholder('your-oauth-client-id').fill('edited-client')
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
  let updates = await page.evaluate(() => (window as any).authUpdates)
  expect(updates.at(-1).updates.auth_config).toMatchObject({ type: 'oauth', client_id: 'edited-client', client_secret: '' })
  await page.getByRole('tab', { name: 'Info', exact: true }).click()
  await page.getByRole('tab', { name: 'Settings', exact: true }).last().click()
  await expect(auth).toHaveValue('oauth_pregenerated')
  await expect(page.getByPlaceholder('your-oauth-client-id')).toHaveValue('edited-client')
  await auth.selectOption('none')
  await page.waitForFunction(() => (window as any).authUpdates.at(-1)?.updates.auth_config?.type === 'none')
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
  updates = await page.evaluate(() => (window as any).authUpdates)
  expect(updates.at(-1).updates.auth_config).toEqual({ type: 'none' })
  const count = updates.length
  await page.waitForTimeout(1400) // More than two debounce windows: a server refresh must not trigger another save.
  expect(await page.evaluate(() => (window as any).authUpdates.length)).toBe(count)
  await auth.selectOption('oauth_pregenerated')
  await expect(page.getByText(/Not saved — .*Client ID and Client Secret/)).toBeVisible()
})

test('credential edits made during an in-flight auto-save are eventually persisted', async ({ page }) => {
  await createPregeneratedServer(page)
  await page.evaluate(() => { (window as any).holdAuthSave = true })
  await page.getByPlaceholder('your-oauth-client-id').fill('first-edit')
  await page.waitForFunction(() => (window as any).releaseAuthSave)
  await page.getByPlaceholder('your-oauth-client-id').fill('second-edit')
  await page.waitForTimeout(800) // Let the second debounce expire while the first save is held.
  await page.evaluate(() => (window as any).releaseAuthSave())
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
  const updates = await page.evaluate(() => (window as any).authUpdates)
  expect(updates).toHaveLength(2)
  expect(updates.at(-1).updates.auth_config.client_id).toBe('second-edit')
})

test('auto-save failures remain visible and a subsequent edit retries successfully', async ({ page }) => {
  await createPregeneratedServer(page)
  await page.evaluate(() => { (window as any).failAuthSave = true })
  await page.getByPlaceholder('your-oauth-client-id').fill('retry-client')
  await expect(page.getByText(/Not saved — .*Simulated configuration write failure/)).toBeVisible()
  await page.evaluate(() => { (window as any).failAuthSave = false })
  await page.getByPlaceholder('your-oauth-client-id').fill('retry-client-success')
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
})

test('OAuth Authentication appears after Server Configuration and browser credentials auto-save', async ({ page }) => {
  await captureAuthUpdates(page)
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click()
  await page.getByText('GitHub', { exact: true }).first().click()
  await page.getByRole('tab', { name: 'Settings', exact: true }).last().click()
  const configuration = page.getByText('Server Configuration', { exact: true })
  const oauth = page.getByText('OAuth Authentication', { exact: true })
  await expect(configuration).toBeVisible()
  await expect(oauth).toBeAttached()
  expect(await configuration.evaluate((node, other) => !!(node.compareDocumentPosition(other as Node) & Node.DOCUMENT_POSITION_FOLLOWING), await oauth.elementHandle())).toBe(true)
  await page.getByPlaceholder('your-oauth-client-id').fill('github-registered-client')
  await page.getByPlaceholder('Enter new secret to update (leave empty to keep existing)').fill('github-app-secret')
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
  const updates = await page.evaluate(() => (window as any).authUpdates)
  expect(updates.at(-1).updates.auth_config).toMatchObject({ type: 'oauth_browser', client_id: 'github-registered-client', client_secret: 'github-app-secret' })
})

test('reverting a credential during an in-flight save restores the persisted value', async ({ page }) => {
  await createPregeneratedServer(page)
  await page.evaluate(() => { (window as any).holdAuthSave = true })
  await page.getByPlaceholder('your-oauth-client-id').fill('temporary-client')
  await page.waitForFunction(() => (window as any).releaseAuthSave)
  await page.getByPlaceholder('your-oauth-client-id').fill('registered-client')
  await page.waitForTimeout(800)
  await page.evaluate(() => (window as any).releaseAuthSave())
  await expect(page.getByText('All changes saved', { exact: true })).toBeVisible()
  const updates = await page.evaluate(() => (window as any).authUpdates)
  expect(updates).toHaveLength(2)
  expect(updates.at(-1).updates.auth_config.client_id).toBe('registered-client')
})


for (const service of [
  { name: 'Gmail', icon: 'gmail', host: 'gmailmcp', scopes: ['gmail.readonly', 'gmail.compose'] },
  { name: 'Google Drive', icon: 'google-drive', host: 'drivemcp', scopes: ['drive.readonly', 'drive.file'] },
]) {
  test(`${service.name} template saves personal OAuth credentials with documented scopes`, async ({ page }) => {
    await captureAuthUpdates(page)
    await page.getByRole('dialog').getByRole('button', { name: new RegExp(`${service.icon} logo ${service.name}`, 'i') }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog).toContainText('Google Workspace Developer Preview')
    await expect(dialog).toContainText('http://localhost:8080/callback')
    await expect(dialog.getByRole('button', { name: 'Open official setup guide' })).toBeVisible()
    await expect(dialog.getByLabel('Client ID', { exact: true })).toHaveAttribute('required', '')
    await expect(dialog.getByLabel('Client Secret', { exact: true })).toHaveAttribute('required', '')
    await dialog.getByLabel('Client ID', { exact: true }).fill('personal-google-client')
    await dialog.getByLabel('Client Secret', { exact: true }).fill('personal-google-secret')
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await page.waitForFunction(() => (window as any).createdMcpArgs)
    const created = await page.evaluate(() => (window as any).createdMcpArgs)
    expect(created.transportConfig.url).toBe(`https://${service.host}.googleapis.com/mcp/v1`)
    expect(created.transportConfig.headers).toEqual({})
    expect(created.authConfig).toMatchObject({ type: 'oauth_browser', client_id: 'personal-google-client', client_secret: 'personal-google-secret', scopes: service.scopes.map(scope => `https://www.googleapis.com/auth/${scope}`) })
    await page.waitForFunction(() => (window as any).startedMcpOAuth?.serverId)
  })
}
