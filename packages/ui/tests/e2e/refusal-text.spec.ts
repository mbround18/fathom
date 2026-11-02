/** RefusalText: a denial, a server error and the app being offline, each as one alert. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'refusal-text'

test('a denial names the action and the reason code', async ({ page }) => {
  await openPiece(page, { piece, state: 'denied' })
  const alert = page.getByRole('alert')
  await expect(alert).toHaveText('Denied: delete_widget (no_grant)')
  await expect(alert).toHaveAttribute('data-refusal', 'denied')
  await expect(alert.locator('[data-denied-action]')).toHaveText('delete_widget')
  await expect(alert.locator('[data-denied-reason]')).toHaveText('no_grant')
})

test('an error shows the message as sent, and its code', async ({ page }) => {
  await openPiece(page, { piece, state: 'error' })
  const alert = page.getByRole('alert')
  await expect(alert).toHaveText('The widget changed since you opened it. (conflict)')
  await expect(alert).toHaveAttribute('data-refusal', 'error')
  await expect(alert.locator('[data-error-code]')).toHaveText('conflict')
})

test('offline reads as a plain sentence, with no code', async ({ page }) => {
  await openPiece(page, { piece, state: 'offline' })
  const alert = page.getByRole('alert')
  await expect(alert).toHaveText('Not connected. Changes wait until the connection is back.')
  await expect(alert.locator('[data-error-code]')).toHaveCount(0)
})

test('the lead is read out in the same alert, before the reason', async ({ page }) => {
  await openPiece(page, { piece, state: 'lead' })
  await expect(page.getByRole('alert')).toHaveText('Could not save the widget. The widget changed since you opened it. (conflict)')
})

for (const state of ['denied', 'error', 'offline', 'lead']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
