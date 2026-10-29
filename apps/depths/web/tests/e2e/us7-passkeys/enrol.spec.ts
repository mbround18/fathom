/**
 * Spec 054 US4 (T075): the passkey gate after sign-in, against the fake backend and Chromium's
 * virtual authenticator.
 *
 * - An admin with no passkey is asked to register one; until they do, admin actions are refused
 *   with the server's reason and the banner stays.
 * - An admin with a passkey confirms it's them once; a cancelled prompt leaves the dialog with why.
 * - A plain user sees no gate.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'
import { cancelPrompts, seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const banner = (page: Page) => page.getByTestId('passkey-banner')
const passkeyDialog = (page: Page) => page.locator('[data-passkey-dialog]')
const action = (page: Page, name: string) => page.locator(`[role="toolbar"][aria-label="Stack actions"] [data-action="${name}"]`)

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web')])
}

test('an admin with no passkey registers one, and admin actions unlock', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  seed(backend)
  backend.passkeys.enable()
  await page.goto('/stacks/web')

  // Asked once, right after sign-in; "Not now" leaves the restricted view.
  await expect(passkeyDialog(page)).toHaveAttribute('data-passkey-dialog', 'enrol')
  await passkeyDialog(page).getByRole('button', { name: 'Not now' }).click()
  await expect(passkeyDialog(page)).toHaveCount(0)
  await expect(banner(page)).toHaveAttribute('data-state', 'enrol')
  await expect(banner(page)).toContainText('Admin actions locked.')

  // The server refuses admin actions and says why.
  await action(page, 'up').click()
  await expect(page.getByRole('dialog')).toContainText("Confirm it's you with a passkey first.")
  await page.keyboard.press('Escape')

  await banner(page).locator('[data-action="passkey-enrol-open"]').click()
  await passkeyDialog(page).getByLabel('Name').fill('Work laptop')
  await passkeyDialog(page).locator('[data-action="passkey-register"]').click()
  await expect(passkeyDialog(page)).toContainText('Passkey registered')
  expect((await backend.waitForCmd('PasskeyBegin')).data).toEqual({ purpose: { kind: 'register', name: 'Work laptop' } })
  await passkeyDialog(page).getByRole('button', { name: 'Done' }).click()

  await expect(banner(page)).toHaveCount(0)
  expect(backend.passkeys.items.map((p) => p.name)).toEqual(['Work laptop'])
  await action(page, 'up').click()
  await expect(page.getByRole('dialog')).toHaveAttribute('data-preview', 'up')
})

test('an admin with a passkey confirms it is them; a cancelled prompt says so and can be retried', async ({ page, backend }) => {
  const auth = await virtualAuthenticator(page)
  seed(backend)
  backend.passkeys.enable({ items: [await seedPasskey(auth, 'Phone')] })
  await cancelPrompts(auth)
  await page.goto('/stacks/web')

  await expect(passkeyDialog(page)).toHaveAttribute('data-passkey-dialog', 'verify')
  await expect(passkeyDialog(page)).toContainText('Confirm it’s you')
  await passkeyDialog(page).locator('[data-action="passkey-verify"]').click()
  await expect(passkeyDialog(page).locator('[data-passkey-error]')).toBeVisible()
  await expect(banner(page)).toHaveAttribute('data-state', 'verify')

  await cancelPrompts(auth, false)
  await passkeyDialog(page).locator('[data-action="passkey-verify"]').click()
  await expect(passkeyDialog(page)).toContainText('Confirmed')
  await passkeyDialog(page).getByRole('button', { name: 'Done' }).click()
  await expect(banner(page)).toHaveCount(0)
  expect(backend.passkeys.verified).toBe(true)
})

test('a plain user sees no gate', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'])
  backend.passkeys.enable()
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(banner(page)).toHaveCount(0)
  await expect(passkeyDialog(page)).toHaveCount(0)
})
