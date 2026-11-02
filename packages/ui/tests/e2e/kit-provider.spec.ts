/**
 * The harness itself and the kit's provider: the index lists every piece, a piece without a state
 * lists its states, and a state renders inside the harness's provider.
 */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

test('the index lists every piece, and a piece lists its states', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { level: 1, name: 'Kit pieces' })).toBeVisible()
  await page.getByRole('link', { name: 'kit-provider' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'kit-provider' })).toBeVisible()
  await expect(page.getByRole('link', { name: 'default' })).toBeVisible()
})

test('a state renders inside the provider', async ({ page }) => {
  await openPiece(page, { piece: 'kit-provider', state: 'default' })
  await expect(page.locator('[data-guide-href]')).toHaveText('/help/widgets#add-a-widget')
  await expect(page.locator('[data-glossary-size]')).toHaveText('2')
})

test('the harness is accessible', async ({ page }) => {
  await expectAccessible(page, { piece: 'kit-provider', state: 'default' })
})
