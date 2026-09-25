import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, openHosted } from './helpers'

test('sign-out calls the transport and shows the signed-out screen', async ({ page }) => {
  await openHosted(page)
  await emit(
    page,
    batch([system(1, { type: 'ContextsUpdated', data: { contexts: ['c'], current: 'c' } })]),
  )
  await page.getByRole('button', { name: 'Account menu for alice' }).click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()
  await expect(page.getByRole('heading', { name: 'Signed out' })).toBeVisible()
  expect(await page.evaluate(() => window.__fathomFake?.signOuts())).toBe(1)
})
