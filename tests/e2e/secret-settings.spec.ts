import { expect, test, type Locator } from '@playwright/test'

async function checkSelector(selector: Locator) {
  await expect(selector.getByRole('button')).toHaveText(['Redact', 'Ask', 'Notify', 'Off'])
  for (const [label, color] of [['Redact', 'bg-red-600'], ['Ask', 'bg-amber-500'], ['Notify', 'bg-blue-500'], ['Off', 'bg-emerald-500']]) {
    const button = selector.getByRole('button', { name: label, exact: true })
    await button.click()
    await expect(button).toHaveClass(new RegExp(color))
  }
}

test('global selector uses the GuardRails palette and Redact Ask Notify Off order', async ({ page }, testInfo) => {
  await page.goto('/demo')
  await page.getByRole('navigation').getByRole('button', { name: 'Optimize', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: 'Secret Scanning', exact: true }).click()
  const selector = page.getByRole('button', { name: 'Redact', exact: true }).locator('..')
  await checkSelector(selector)
  await page.getByRole('button', { name: 'Redact', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Redact', exact: true })).toHaveCSS('background-color', 'rgb(220, 38, 38)')
  await page.screenshot({ path: testInfo.outputPath('global-secret-settings.png') })
})

test('client selector retains Default then Redact Ask Notify Off', async ({ page }, testInfo) => {
  await page.goto('/demo')
  await page.getByRole('navigation').getByRole('button', { name: 'Clients', exact: true }).click()
  await page.getByRole('navigation').getByRole('button', { name: /Cursor/ }).click()
  await page.getByRole('tab', { name: 'Optimize', exact: true }).click()
  const selector = page.getByRole('button', { name: 'Redact', exact: true }).locator('..')
  await expect(selector.getByRole('button')).toHaveText(['Default (Off)', 'Redact', 'Ask', 'Notify', 'Off'])
  await page.getByRole('button', { name: 'Redact', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Redact', exact: true })).toHaveClass(/bg-red-600/)
  await expect(page.getByRole('button', { name: 'Redact', exact: true })).toHaveCSS('background-color', 'rgb(220, 38, 38)')
  await page.screenshot({ path: testInfo.outputPath('client-secret-settings.png') })
})
