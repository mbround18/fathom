import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, open, openHosted } from './helpers'

const ctx = batch([
  system(1, { type: 'ContextsUpdated', data: { contexts: ['a', 'b'], current: 'a' } }),
])

test('hosted hides fleet and the context picker', async ({ page }) => {
  await openHosted(page)
  await emit(page, ctx)
  await expect(page.getByRole('button', { name: 'Account menu for alice' })).toBeVisible()
  await expect(page.getByTestId('fleet-open')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Fleet dashboard' })).toHaveCount(0)
})

test('desktop still shows fleet', async ({ page }) => {
  await open(page, 'manual')
  await emit(page, ctx)
  await expect(page.getByTestId('fleet-open')).toBeVisible()
})
