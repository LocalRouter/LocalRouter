import { test, expect } from '@playwright/test'

for (const width of [390, 1440]) {
 for (const colorScheme of ["light", "dark"] as const) {
  test(`secret round trip is readable at ${width}px in ${colorScheme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 })
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme })
    await page.goto('/')
    const section = page.locator('section[aria-labelledby="secret-placeholders-title"]')
    await section.scrollIntoViewIfNeeded()
    await expect(section.getByRole('heading', { name: /Keep the secret.*Send the placeholder/  })).toBeVisible()
    await expect(section.getByText('1. Hold locally')).toBeVisible()
    await expect(section.getByText('2. Send the reference')).toBeVisible()
    await expect(section.getByText('3. Restore on return')).toBeVisible()
    await expect(section.locator('code').filter({ hasText: 'API_KEY=demo-only-secret' })).toHaveCount(2)
    await expect(section.locator('code').filter({ hasText: 'Unmasked when returned verbatim' })).toHaveText('<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    await section.screenshot({ path: `/tmp/localrouter-secret-logs/diagram-${width}-${colorScheme}.png` })
    await section.getByRole('link', { name: 'Read the research' }).click()
    await expect(page).toHaveURL(/research\/reversible-secret-redaction/)
    await expect(page.getByRole('heading', { name: 'Reversible Secret Redaction', exact: true })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Recognition Algorithm', exact: true })).toBeVisible()
  })
 }
}

test('landing Research card opens the full paper', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('link').filter({ has: page.getByRole('heading', { name: 'Reversible Secret Redaction', exact: true }) }).click()
  await expect(page).toHaveURL(/research\/reversible-secret-redaction/)
  await expect(page.getByRole('heading', { name: 'Evaluation and Reproduction', exact: true })).toBeVisible()
})

test('Ask can select a restorable placeholder without a stored digest', async ({ page }) => {
  await page.goto('/__preview/secret-redaction')
  await page.getByRole('button', { name: 'More redaction options' }).click()
  await page.getByRole('menuitem', { name: 'Use placeholder', exact: true }).click()
  await expect(page.getByTestId('outgoing-payload')).toContainText('<localrouter-redacted')
  await expect(page.getByTestId('outgoing-payload')).not.toContainText('6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr')
  await expect(page.getByTestId('stored-hash')).toHaveCount(0)
})
