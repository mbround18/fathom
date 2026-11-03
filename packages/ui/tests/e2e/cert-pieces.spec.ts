/** ChipList, PemField and StageList, together as a certificate's page would use them. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'cert-pieces'
const CERT = '-----BEGIN CERTIFICATE-----\nTUlJQnB6Q0NBVTJnQXdJQkFnSVVaZmlsbGVyZmlsbGVyZmlsbGVy\n-----END CERTIFICATE-----\n'
const KEY = CERT.replace(/CERTIFICATE/g, 'PRIVATE KEY')

test('summary: names fold into +5 more, open from the keyboard, and fold back', async ({ page }) => {
  await openPiece(page, { piece, state: 'summary' })
  const names = page.getByRole('list', { name: 'Subject alternative names' })
  await expect(names.getByRole('listitem')).toHaveCount(3)
  const more = page.getByRole('button', { name: '+5 more' })
  await more.focus()
  await page.keyboard.press('Enter')
  await expect(names.getByRole('listitem')).toHaveCount(8)
  await expect(page.getByRole('button', { name: 'Show fewer' })).toHaveAttribute('aria-expanded', 'true')
})

test('summary: nothing scrolls sideways at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'summary' })
  await page.getByRole('button', { name: '+5 more' }).click()
  const [scroll, client] = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.clientWidth])
  expect(scroll).toBeLessThanOrEqual(client)
})

test('pem: says what it found, catches a key, and loads a file', async ({ page }) => {
  await openPiece(page, { piece, state: 'pem' })
  const field = page.getByLabel('Certificate chain')
  await expect(field).toHaveAccessibleDescription('The leaf first, then each intermediate.')
  await field.fill(KEY)
  await expect(field).toHaveAttribute('aria-invalid', 'true')
  await expect(field).toHaveAccessibleDescription('Expected CERTIFICATE, found PRIVATE KEY.')
  await page.locator('input[type="file"]').setInputFiles({ name: 'chain.pem', mimeType: 'application/x-pem-file', buffer: Buffer.from(CERT + CERT) })
  await expect(field).toHaveValue(CERT + CERT)
  await expect(field).toHaveAccessibleDescription('2 blocks: CERTIFICATE, CERTIFICATE')
})

test('stages: the stuck stage says so in words, and opens from the keyboard', async ({ page }) => {
  await openPiece(page, { piece, state: 'stages' })
  const stuck = page.locator('[data-state="stuck"]')
  await expect(stuck).toContainText('Stuck')
  await expect(stuck).toContainText('no TXT record')
  await expect(page.locator('[aria-current="step"]')).toHaveAttribute('data-stage', 'order')
  await page.keyboard.press('Tab')
  await expect(stuck.getByRole('button')).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.locator('[data-opened="challenge"]')).toBeVisible()
})

test('summary: no accessibility violations, folded and open', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'summary' })
  await expectAccessible(page, { piece, state: 'summary' }, { prepare: (p) => p.getByRole('button', { name: '+5 more' }).click() })
})

test('pem: no accessibility violations, empty and refusing a key', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'pem' })
  await expectAccessible(page, { piece, state: 'pem' }, { prepare: (p) => p.getByLabel('Certificate chain').fill(KEY) })
})

test('stages: no accessibility violations', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'stages' })
})
