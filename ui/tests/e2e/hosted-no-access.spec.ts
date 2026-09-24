import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, openHosted } from './helpers'

test('a zero-permission user sees the no-access state', async ({ page }) => {
  await openHosted(page, 'user=carol')
  await emit(
    page,
    batch([system(1, { type: 'ContextsUpdated', data: { contexts: [], current: null } })]),
  )
  await expect(page.getByRole('heading', { name: 'No access' })).toBeVisible()
  await expect(page.getByText(/carol is signed in/)).toBeVisible()
})
