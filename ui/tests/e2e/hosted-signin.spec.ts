/** 031 US1: hosted shell shows the signed-in identity; an ended session shows sign-in-again. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, openHosted } from './helpers'

test('identity is shown in the user menu', async ({ page }) => {
  await openHosted(page)
  await emit(
    page,
    batch([system(1, { type: 'ContextsUpdated', data: { contexts: ['c'], current: 'c' } })]),
  )
  await page.getByRole('button', { name: 'Account menu for alice' }).click()
  await expect(page.getByText('dev, ops')).toBeVisible()
})

test('SessionEnded shows the sign-in gate', async ({ page }) => {
  await openHosted(page)
  await emit(page, batch([system(1, { type: 'SessionEnded', data: { reason: 'expired' } })]))
  await expect(page.getByRole('heading', { name: 'Session expired' })).toBeVisible()
  await expect(page.getByRole('link', { name: /sign in/i })).toBeVisible()
})
