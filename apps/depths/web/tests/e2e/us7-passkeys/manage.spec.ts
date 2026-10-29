/**
 * Spec 054 US4 (T075): your passkeys, on the sessions page.
 *
 * - The list names each passkey, when it was added and when it was last used; another can be added.
 * - Removing previews first; the last passkey cannot be removed. When the session is stale, the
 *   removal asks for a passkey and `Confirm` carries the assertion.
 * - A recovery code lets someone who lost every passkey register a new one.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const panel = (page: Page) => page.getByTestId('passkeys-panel')
const items = (page: Page) => panel(page).locator('[data-passkey]')
const passkeyDialog = (page: Page) => page.locator('[data-passkey-dialog]')
const removeDialog = (page: Page) => page.locator('[data-remove-dialog]')

for (const width of [360, 1280]) {
  test(`lists, adds and removes passkeys at ${String(width)} px`, async ({ page, backend }) => {
    await page.setViewportSize({ width, height: 900 })
    // Passkeys on other devices: this authenticator holds none, so it can add one.
    await virtualAuthenticator(page)
    const phone = { id: 'cGhvbmU', name: 'Phone', created: '2026-10-01T09:00:00Z', last_used: null }
    const key = { id: 'a2V5', name: 'Security key', created: '2026-10-02T09:00:00Z', last_used: '2026-10-27T08:00:00Z' }
    backend.passkeys.enable({ items: [phone, key], verified: true })
    await page.goto('/account/sessions')

    await expect(items(page)).toHaveCount(2)
    await expect(items(page).first().locator('[data-field="name"]')).toHaveText('Phone')
    await expect(items(page).first().locator('[data-field="last-used"]')).toHaveText('Never used')
    await expect(items(page).nth(1).locator('[data-field="last-used"]')).toContainText('Last used')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)

    await panel(page).locator('[data-action="passkey-add"]').click()
    await passkeyDialog(page).getByLabel('Name').fill('Laptop')
    await passkeyDialog(page).locator('[data-action="passkey-register"]').click()
    await expect(passkeyDialog(page)).toContainText('Passkey registered')
    await passkeyDialog(page).getByRole('button', { name: 'Done' }).click()
    await expect(items(page)).toHaveCount(3)

    await panel(page).getByRole('button', { name: 'Remove Phone' }).click()
    await expect(removeDialog(page)).toHaveAttribute('data-remove-dialog', 'ready')
    await removeDialog(page).locator('[data-confirm]').click()
    await expect(removeDialog(page)).toHaveCount(0)
    await expect(items(page)).toHaveCount(2)
    expect(backend.passkeys.items.map((p) => p.name)).toEqual(['Security key', 'Laptop'])
  })
}

test('the last passkey cannot be removed', async ({ page, backend }) => {
  const auth = await virtualAuthenticator(page)
  backend.passkeys.enable({ items: [await seedPasskey(auth, 'Phone')], verified: true })
  await page.goto('/account/sessions')
  await expect(items(page)).toHaveCount(1)
  await expect(panel(page).getByRole('button', { name: 'Remove Phone' })).toBeDisabled()
  await expect(panel(page)).toContainText('Your only passkey cannot be removed. Add another first.')
})

test('a removal on a stale session asks for a passkey and carries the assertion', async ({ page, backend }) => {
  const auth = await virtualAuthenticator(page)
  const phone = await seedPasskey(auth, 'Phone')
  const key = await seedPasskey(auth, 'Security key')
  backend.passkeys.enable({ items: [phone, key], verified: true, stale: true })
  await page.goto('/account/sessions')

  await panel(page).getByRole('button', { name: 'Remove Security key' }).click()
  await expect(removeDialog(page)).toContainText('Depths will ask for one of your passkeys to confirm.')
  await removeDialog(page).locator('[data-confirm]').click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.assertion?.credential.id).toBe(phone.id)
  await expect(removeDialog(page)).toHaveCount(0)
  await expect(items(page)).toHaveCount(1)
})

test('a recovery code lets someone register a new passkey', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  // The passkey on file is on a lost device: not on this authenticator.
  backend.passkeys.enable({ items: [{ id: 'bG9zdA', name: 'Lost phone', created: '2026-09-01T09:00:00Z', last_used: null }] })
  await page.goto('/account/sessions')
  await passkeyDialog(page).getByRole('button', { name: 'Not now' }).click()

  const recovery = page.getByTestId('passkey-recovery')
  await recovery.getByPlaceholder('Recovery code').fill('WRONGCODE99')
  await recovery.locator('[data-action="passkey-recover"]').click()
  await expect(recovery.getByRole('alert')).toHaveText('That recovery code is not valid, or was already used.')

  await recovery.getByPlaceholder('Recovery code').fill(backend.passkeys.recoveryCode)
  await recovery.locator('[data-action="passkey-recover"]').click()
  await expect(passkeyDialog(page)).toHaveAttribute('data-passkey-dialog', 'enrol')
  await passkeyDialog(page).getByLabel('Name').fill('New phone')
  await passkeyDialog(page).locator('[data-action="passkey-register"]').click()
  await expect(passkeyDialog(page)).toContainText('Passkey registered')
  await passkeyDialog(page).getByRole('button', { name: 'Done' }).click()

  await expect(page.getByTestId('passkey-banner')).toHaveCount(0)
  await expect(items(page)).toHaveCount(2)
})

test('registering on a device that already holds a passkey says so', async ({ page, backend }) => {
  const auth = await virtualAuthenticator(page)
  backend.passkeys.enable({ items: [await seedPasskey(auth, 'Phone')], verified: true })
  await page.goto('/account/sessions')
  await panel(page).locator('[data-action="passkey-add"]').click()
  await passkeyDialog(page).getByLabel('Name').fill('Phone again')
  await passkeyDialog(page).locator('[data-action="passkey-register"]').click()
  await expect(passkeyDialog(page).locator('[data-passkey-error]')).toHaveText(
    'This device already holds one of your passkeys. Use another device or a security key.',
  )
  expect(backend.passkeys.items).toHaveLength(1)
})
