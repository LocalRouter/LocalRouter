import { expect, test } from '@playwright/test'

const SECRET = 'AKIAZ7Q3VJ2M4XK9PLR6'
const MARKER = '[REDACTED: secret removed by LocalRouter]'

test.beforeEach(async ({ page }) => { await page.goto('/__preview/secret-redaction') })

test('Redact once continues with a marker and asks again on the next request', async ({ page }) => {
  await page.getByRole('button', { name: 'Redact once', exact: true }).click()
  await expect(page.getByTestId('outgoing-payload')).toContainText(MARKER)
  await expect(page.getByTestId('outgoing-payload')).not.toContainText(SECRET)
  await expect(page.getByTestId('stored-hash')).toHaveCount(0)
  await page.getByRole('button', { name: 'Send the same request again' }).click()
  await expect(page.getByRole('button', { name: 'Redact once', exact: true })).toBeVisible()
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
})

test('Always redact this secret saves a hash and redacts subsequent requests', async ({ page }) => {
  await page.getByRole('button', { name: 'More redaction options' }).click()
  await page.getByRole('menuitem', { name: 'Always redact this secret', exact: true }).click()
  await expect(page.getByTestId('stored-hash')).toHaveText(/^[a-f0-9]{64}$/)
  await expect(page.getByTestId('stored-hash')).not.toContainText(SECRET)
  await expect(page.getByTestId('outgoing-payload')).toContainText(MARKER)
  await page.getByRole('button', { name: 'Send the same request again' }).click()
  await expect(page.getByRole('status')).toContainText('request continued')
  await expect(page.getByTestId('outgoing-payload')).not.toContainText(SECRET)
  await expect(page.getByRole('button', { name: 'Redact once', exact: true })).toHaveCount(0)
})

test('Always allow lives in the Allow dropdown and only remembers this value', async ({ page }) => {
  await expect(page.getByRole('checkbox')).toHaveCount(0)
  await page.getByRole('button', { name: 'More allow options' }).click()
  await page.getByRole('menuitem', { name: 'Always allow this secret' }).click()
  await expect(page.getByTestId('stored-hash')).toBeVisible()
  await expect(page.getByTestId('outgoing-payload')).toContainText(SECRET)
  await page.getByRole('button', { name: 'Send the same request again' }).click()
  await expect(page.getByRole('button', { name: 'Allow', exact: true })).toHaveCount(0)
  await expect(page.getByTestId('outgoing-payload')).toContainText(SECRET)
})

test('Block and Allow preserve their original behavior', async ({ page }) => {
  await page.getByRole('button', { name: 'Block', exact: true }).click()
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
  await page.getByRole('button', { name: 'Reset preview' }).click()
  await page.getByRole('button', { name: 'Allow', exact: true }).click()
  await expect(page.getByTestId('outgoing-payload')).toContainText(SECRET)
})

const SECOND_SECRET = 'AKIAX9Q7W5E3R1T8Y6U4'

test('multiple secrets appear sequentially; block stops without showing later secrets', async ({ page }) => {
  await page.goto('/__preview/secret-redaction?multiple')
  await expect(page.getByText('AKIA************PLR6', { exact: true }).first()).toBeVisible()
  await expect(page.getByText('AKIA************Y6U4', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Block', exact: true }).click()
  await expect(page.getByTestId('approval-popup')).toHaveCount(0)
  await expect(page.getByRole('status')).toContainText('remaining secrets were not shown')
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
})

test('redact then allow only forwards after both secrets are reviewed', async ({ page }) => {
  await page.goto('/__preview/secret-redaction?multiple')
  await page.getByRole('button', { name: 'Redact once', exact: true }).click()
  await expect(page.getByText('AKIA************Y6U4', { exact: true }).first()).toBeVisible()
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
  await page.getByRole('button', { name: 'Allow', exact: true }).click()
  await expect(page.getByTestId('outgoing-payload')).toContainText(MARKER)
  await expect(page.getByTestId('outgoing-payload')).not.toContainText(SECRET)
  await expect(page.getByTestId('outgoing-payload')).toContainText(SECOND_SECRET)
})

test('Always redact only remembers the displayed secret; remaining secret still asks next time', async ({ page }) => {
  await page.goto('/__preview/secret-redaction?multiple')
  await page.getByRole('button', { name: 'More redaction options' }).click()
  await page.getByRole('menuitem', { name: 'Always redact this secret', exact: true }).click()
  await expect(page.getByText('AKIA************Y6U4', { exact: true }).first()).toBeVisible()
  await page.getByRole('button', { name: 'Allow', exact: true }).click()
  await page.getByRole('button', { name: 'Send the same request again' }).click()
  await expect(page.getByTestId('approval-popup').getByText('AKIA************Y6U4', { exact: true }).first()).toBeVisible()
  await expect(page.getByTestId('approval-popup').getByText('AKIA************PLR6', { exact: true })).toHaveCount(0)
  await expect(page.getByTestId('stored-hash')).toHaveCount(1)
})

test('a failed policy save keeps the current popup open and does not forward', async ({ page }) => {
  await page.goto('/__preview/secret-redaction?saveError')
  await page.getByRole('button', { name: 'More redaction options' }).click()
  await page.getByRole('menuitem', { name: 'Always redact this secret', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('Could not save')
  await expect(page.getByTestId('approval-popup')).toBeVisible()
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
  await page.getByRole('button', { name: 'Block', exact: true }).click()
  await expect(page.getByTestId('approval-popup')).toHaveCount(0)
})

test('dark yellow action has white text and secrets match regular popup text size', async ({ page }) => {
  const redact = page.getByRole('button', { name: 'Redact once', exact: true })
  await expect(redact).toHaveCSS('background-color', 'rgb(128, 112, 0)')
  await expect(redact).toHaveCSS('color', 'rgb(255, 255, 255)')
  await expect(page.getByText('AKIA************PLR6', { exact: true }).first()).toHaveCSS('font-size', '12px')
  await page.getByRole('button', { name: 'Reveal secret', exact: true }).click()
  await expect(page.getByText(SECRET, { exact: true }).first()).toHaveCSS('font-size', '12px')
  await expect(page.getByText('Redaction replaces this secret with', { exact: false })).toHaveCount(0)
})

test('Modify in other approval popups shares the dark yellow and white styling', async ({ page }) => {
  await page.goto('/')
  const modify = page.getByRole('button', { name: 'Modify', exact: true }).first()
  await expect(modify).toHaveCSS('background-color', 'rgb(128, 112, 0)')
  await expect(modify).toHaveCSS('color', 'rgb(255, 255, 255)')
})


test('event is already visible inline and centered on the secret, with complete source and request scrolling', async ({ page }) => {
  const popup = page.getByTestId('approval-popup')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(popup.getByRole('button', { name: /View event|Jump to secret/ })).toHaveCount(0)
  await expect(popup).not.toContainText('Scan time:')
  await expect(popup).not.toContainText('Findings:')
  const source = popup.getByLabel('Complete detected source')
  await expect(popup).toContainText('Message 2 · user message · Line 63 · First occurrence')
  await expect(source.locator('mark')).toBeInViewport()
  expect(await source.evaluate(node => node.scrollTop)).toBeGreaterThan(0)
  await expect(source).toContainText('Beginning of the deployment request')
  await expect(source).toContainText('End of the deployment request')
  await source.evaluate(node => { node.scrollTop = 0 })
  await expect(source.locator('mark')).not.toBeInViewport()
  await source.evaluate(node => { node.scrollTop = node.scrollHeight })
  await expect(source.locator('mark')).not.toBeInViewport()
  await popup.getByRole('tab', { name: 'Full request', exact: true }).click()
  const request = popup.getByLabel('Complete request JSON')
  await expect(request).toContainText('Previous conversation message')
  await expect(request).toContainText('Beginning of the deployment request')
  await expect(request).toContainText('End of the deployment request')
  await expect(request).toContainText('"temperature": 0.7')
  await expect(request).not.toContainText(SECRET)
  await expect(request).toContainText(MARKER)
  await popup.getByRole('tab', { name: 'Detected source', exact: true }).click()
  await expect(source.locator('mark')).toBeInViewport()
  await popup.getByRole('button', { name: 'Reveal secret', exact: true }).click()
  await expect(source.locator('mark')).toHaveText(SECRET)
  await popup.getByRole('button', { name: 'Hide secret', exact: true }).click()
  await expect(source).not.toContainText(SECRET)
  await expect(popup.getByRole('button', { name: 'Block', exact: true })).toBeInViewport()
  await expect(popup.getByRole('button', { name: 'Redact once', exact: true })).toBeInViewport()
  await expect(popup.getByRole('button', { name: 'Allow', exact: true })).toBeInViewport()
  await expect(page.getByTestId('outgoing-payload')).toHaveText('Not forwarded')
})

test('notify popups show the event inline and still dismiss directly', async ({ page }) => {
  await page.goto('/__preview/secret-redaction?notify')
  const popup = page.getByTestId('approval-popup')
  await expect(popup.locator('mark')).toHaveText('AKIA************PLR6')
  await expect(popup.locator('mark')).toBeInViewport()
  await popup.getByRole('tab', { name: 'Full request', exact: true }).click()
  await expect(popup.getByLabel('Complete request JSON')).toContainText('End of the deployment request')
  await page.getByRole('button', { name: 'Dismiss', exact: true }).click()
  await expect(popup).toHaveCount(0)
})
