import { test, expect } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  // The demo must work entirely with its local mock; prevent remote assets or traffic.
  await page.route('**/*', route => new URL(route.request().url()).origin === 'http://127.0.0.1:1433' ? route.continue() : route.abort())
  await page.goto('/demo')
})

test('a shared profile is created, edited as a draft, published, attached and detached without changing client controls', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', e => errors.push(e.message))
  await page.getByRole('navigation').getByRole('button', { name: 'Optimize', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: 'Routing', exact: true }).click()
  await page.getByRole('textbox', { name: 'Profile name', exact: true }).fill('Coding')
  await page.getByRole('button', { name: 'Create profile', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Coding', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Add fallback model', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Add fallback model', exact: true })
  await dialog.getByRole('combobox', { name: 'Search policy models' }).fill('gpt-4o-mini')
  await page.screenshot({ path: '/tmp/localrouter-policy-models.png' })
  await dialog.getByRole('listbox').getByRole('option').first().click()
  await page.getByRole('button', { name: 'Save draft routing', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Publish draft', exact: true })).toBeEnabled()
  await page.getByRole('textbox', { name: 'Publication label' }).fill('Coder defaults')
  await page.getByRole('button', { name: 'Publish draft', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Apply', exact: true }).click()
  await expect(page.getByText('Revision 2 · 0 linked clients', { exact: true })).toBeVisible()
  await page.screenshot({ path: '/tmp/localrouter-routing-profile.png', fullPage: true })

  await page.getByRole('navigation').getByRole('button', { name: 'Clients', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: /Cursor/ }).click()
  await page.getByRole('tab', { name: 'LLM', exact: true }).click()
  const baseline = await page.evaluate(async () => {
    const w = window as unknown as { __TAURI_IPC_HANDLER__: (c: string, a: unknown) => Promise<unknown> }
    const state = await w.__TAURI_IPC_HANDLER__('list_strategies', {}) as Array<Record<string, unknown>>
    return state.map(s => ({ id: s.id, permissions: s.model_permissions, free: s.free_tier_only, fallback: s.free_tier_fallback, limits: s.rate_limits, alias: (s.auto_config as { model_name?: string })?.model_name }))
  })
  await page.getByRole('combobox', { name: 'Routing profile', exact: true }).selectOption({ label: 'Coding · revision 2' })
  await page.getByRole('button', { name: 'Apply profile', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Apply', exact: true }).click()
  await expect(page.getByText(/Using Coding, revision 2/)).toBeVisible()
  const after = await page.evaluate(async () => {
    const w = window as unknown as { __TAURI_IPC_HANDLER__: (c: string, a: unknown) => Promise<unknown> }
    const state = await w.__TAURI_IPC_HANDLER__('list_strategies', {}) as Array<Record<string, unknown>>
    return state.map(s => ({ id: s.id, permissions: s.model_permissions, free: s.free_tier_only, fallback: s.free_tier_fallback, limits: s.rate_limits, alias: (s.auto_config as { model_name?: string })?.model_name }))
  })
  expect(after).toEqual(baseline)
  await expect(page.getByRole('button', { name: 'Save routing policy', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Make independent copy', exact: true }).click()
  await page.getByRole('alertdialog').getByRole('button', { name: 'Apply', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Make independent copy', exact: true })).toHaveCount(0)
  await page.screenshot({ path: '/tmp/localrouter-routing-client.png', fullPage: true })
  expect(errors).toEqual([])
})

test('policy pickers search, filter free models and preserve capability separation', async ({ page }) => {
  await page.getByRole('navigation').getByRole('button', { name: 'Clients', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: /Cursor/ }).click()
  await page.getByRole('tab', { name: 'LLM', exact: true }).click()
  await page.getByRole('button', { name: 'Choose a decision model', exact: true }).click()
  const picker = page.getByRole('dialog')
  await expect(picker.getByRole('listbox').getByRole('option').filter({ hasText: 'gpt-4o' })).toHaveCount(0)
  await picker.getByRole('combobox', { name: 'Search policy models' }).fill('laya')
  await expect(picker.getByRole('listbox').getByRole('option').first()).toContainText('laya')
  await picker.getByRole('listbox').getByRole('option').first().click()
  await expect(page.getByRole('button', { name: 'Choose a decision model', exact: true })).toContainText('laya')
  await page.getByRole('button', { name: /Add model to/ }).first().click()
  await expect(page.getByRole('dialog').getByRole('listbox').getByRole('option').filter({ hasText: 'laya:en' })).toHaveCount(0)
  await page.getByRole('dialog').getByRole('checkbox', { name: 'Free eligible only' }).check()
  await page.getByRole('dialog').getByRole('combobox', { name: 'Search policy models' }).fill('zz-no-model')
  await expect(page.getByText('No matching models.', { exact: true })).toBeVisible()
  await page.screenshot({ path: '/tmp/localrouter-policy-picker.png' })
})

test('connection setup displays the actual port and custom auto alias, then updates when stopped', async ({ page }) => {
  await page.evaluate(() => {
    const w = window as unknown as { __TAURI_IPC_HANDLER__: (c: string, a: unknown) => Promise<unknown>; routingTestStopped?: boolean }
    const original = w.__TAURI_IPC_HANDLER__
    w.__TAURI_IPC_HANDLER__ = async (c, a) => {
      if (c === 'get_server_config') return { host: '127.0.0.1', port: 3625, actual_port: w.routingTestStopped ? null : 3627, enable_cors: true }
      if (c === 'get_server_status') return w.routingTestStopped ? 'stopped' : 'running'
      const value = await original(c, a)
      if (c === 'get_strategy') { const s = value as { auto_config: { model_name: string } }; s.auto_config.model_name = 'coding-auto' }
      return value
    }
  })
  await page.getByRole('navigation').getByRole('button', { name: 'Clients', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: /Cursor/ }).click()
  await page.getByRole('tab', { name: 'Connect', exact: true }).click()
  const summary = page.getByLabel('Gateway connection summary')
  await expect(summary).toContainText('http://127.0.0.1:3627/v1')
  await expect(summary).toContainText('coding-auto')
  await expect(summary).toContainText('Configured port 3625 was occupied')
  await expect(summary.getByRole('button', { name: 'Copy URL', exact: true })).toBeEnabled()
  await page.evaluate(async () => {
    const w = window as unknown as { routingTestStopped?: boolean }
    w.routingTestStopped = true
    const importPath = '/src/stubs/tauri-api-event.ts'
    const mod = await import(/* @vite-ignore */ importPath)
    mod.emit('server-status-changed', 'stopped')
  })
  await expect(summary).toContainText('Server stopped')
  await expect(summary.getByRole('button', { name: 'Copy URL', exact: true })).toBeDisabled()
})

test('publication updates two linked clients, preserves their different permissions, and leaves another profile unchanged', async ({ page }) => {
  const result = await page.evaluate(async () => {
    type Profiles = import('../../src/types/tauri-commands').RoutingProfilesState
    type Strategy = import('../../src/types/tauri-commands').Strategy
    const w = window as unknown as { __TAURI_IPC_HANDLER__: (c: string, a: unknown) => Promise<unknown> }
    const call = async (c: string, a: unknown) => await w.__TAURI_IPC_HANDLER__(c, a) as Profiles
    const original = await w.__TAURI_IPC_HANDLER__('list_strategies', {}) as Strategy[]
    const ids = original.slice(0, 3).map(s => s.id)
    const fixturePath = '/src/components/demo/mockData.ts'
    const fixture = await import(/* @vite-ignore */ fixturePath)
    fixture.mockData.clients[1].strategy_id = ids[1]
    let state = await call('create_routing_profile', { name: 'Coding' })
    const codingId = state.profiles[0].profile.id
    state = await call('create_routing_profile', { name: 'Chatbot' })
    const chatbotId = state.profiles[1].profile.id
    for (const [index, strategyId] of ids.entries()) {
      state = await call('get_routing_profiles', { strategyId })
      const p = state.profiles.find(p => p.profile.id === (index < 2 ? codingId : chatbotId))!
      await call('bind_routing_profile', { strategyId, profileId: p.profile.id, expectedStrategyFingerprint: state.strategy_fingerprint, expectedProfileFingerprint: p.fingerprint })
    }
    const chatbotBefore = await w.__TAURI_IPC_HANDLER__('get_strategy', { strategyId: ids[2] })
    state = await call('get_routing_profiles', {})
    let p = state.profiles.find(p => p.profile.id === codingId)!
    const snapshot = { ...p.profile.active, prioritized_models: [['openai-primary', 'gpt-4o']] }
    state = await call('mutate_routing_profile', { profileId: codingId, expectedFingerprint: p.fingerprint, operation: { action: 'save_draft', snapshot } })
    p = state.profiles.find(p => p.profile.id === codingId)!
    const impact = p.clients
    const reviewedFingerprint = p.fingerprint
    state = await call('mutate_routing_profile', { profileId: codingId, expectedFingerprint: reviewedFingerprint, operation: { action: 'publish', label: 'Shared publication' } })
    let rejectedStale = false
    try { await call('mutate_routing_profile', { profileId: codingId, expectedFingerprint: reviewedFingerprint, operation: { action: 'rollback', revision: 1 } }) } catch { rejectedStale = true }
    const updated = await Promise.all(ids.map(strategyId => w.__TAURI_IPC_HANDLER__('get_strategy', { strategyId }))) as Strategy[]
    return { original, updated, chatbotBefore, impact, rejectedStale }
  })
  expect(result.updated[0].auto_config?.prioritized_models).toEqual([['openai-primary', 'gpt-4o']])
  expect(result.updated[1].auto_config?.prioritized_models).toEqual([['openai-primary', 'gpt-4o']])
  expect(result.updated[2]).toEqual(result.chatbotBefore)
  expect(result.impact.flatMap(c => c.issues)).toContain('Model not permitted: openai-primary/gpt-4o')
  expect(result.rejectedStale).toBe(true)
  for (const updated of result.updated) {
    const before = result.original.find(s => s.id === updated.id)!
    expect(updated.model_permissions).toEqual(before.model_permissions)
    expect(updated.rate_limits).toEqual(before.rate_limits)
    expect(updated.free_tier_only).toEqual(before.free_tier_only)
    expect(updated.auto_config?.model_name).toEqual(before.auto_config?.model_name)
  }
})
