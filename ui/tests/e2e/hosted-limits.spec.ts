import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, openHosted } from './helpers'

test('LimitReached shows a dismissible notice', async ({ page }) => {
  await openHosted(page)
  await emit(
    page,
    batch([system(1, { type: 'ContextsUpdated', data: { contexts: ['c'], current: 'c' } })]),
  )
  await emit(page, batch([system(2, { type: 'LimitReached', data: { kind: 'watches', max: 7 } })]))
  const notice = page.getByTestId('limit-reached')
  await expect(notice).toContainText('at most 7 live watches')
  await page.getByRole('button', { name: 'Dismiss limit notice' }).click()
  await expect(notice).toHaveCount(0)
})
