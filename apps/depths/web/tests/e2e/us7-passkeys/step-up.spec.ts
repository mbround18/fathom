/**
 * Spec 054 US5 (T075): a fresh passkey check before risky actions, once the last check is out of
 * its window (`backend.passkeys.stale`).
 *
 * - A preview that asks for one says so; Confirm asks the authenticator and carries the assertion.
 *   A cancelled prompt changes nothing and keeps the preview.
 * - A shell asks in the passkey dialog and opens with the assertion; cancelling ends the tab.
 * - A user change asks, finishes the check, then saves.
 * - Spec 062: the Updates settings change carries the assertion on its Confirm.
 * - A plain user with no passkey is asked to register one.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'
import { cancelPrompts, seedPasskey, virtualAuthenticator, type VirtualAuthenticator } from '../../support/webauthn'

const passkeyDialog = (page: Page) => page.locator('[data-passkey-dialog]')
const container = (page: Page, name: string) => page.locator(`[data-container="${name}"]`)

function seed(backend: FakeBackend): void {
  const web = f.stack('web', {
    services: [{ name: 'app', image: 'nginx:1.29', containers: [f.container({ name: 'web-app-1', project: 'web', service: 'app' })] }],
    running_services: 1,
    total_services: 1,
  })
  backend.stacks = f.stackList([web])
}

/** A verified admin whose check is out of the window, with one passkey on this device. */
async function stale(page: Page, backend: FakeBackend): Promise<{ auth: VirtualAuthenticator; id: string }> {
  const auth = await virtualAuthenticator(page)
  const phone = await seedPasskey(auth, 'Phone')
  seed(backend)
  backend.passkeys.enable({ items: [phone], verified: true, stale: true })
  return { auth, id: phone.id }
}

test('a stack operation asks for the passkey on Confirm and carries the assertion', async ({ page, backend }) => {
  const { auth, id } = await stale(page, backend)
  await page.goto('/stacks/web')
  await page.locator('[role="toolbar"][aria-label="Stack actions"] [data-action="restart"]').click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-step-up]')).toContainText('Confirming asks for your passkey')

  // Cancelled: nothing sent, the preview stays.
  await cancelPrompts(auth)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toContainText('Not confirmed, nothing changed.')
  expect(backend.received.filter((c) => c.type === 'Confirm')).toEqual([])

  await cancelPrompts(auth, false)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.assertion?.credential.id).toBe(id)
  await expect(dialog.locator('[data-step-up]')).toHaveCount(0)
  expect(backend.passkeys.stale).toBe(false)
})

test('a shell asks for the passkey, then opens; cancelling ends the tab with a new-session button', async ({ page, backend }) => {
  await stale(page, backend)
  await page.goto('/stacks/web')

  await container(page, 'web-app-1').locator('[data-open="shell"]').click()
  await expect(passkeyDialog(page)).toHaveAttribute('data-passkey-dialog', 'step_up')
  await expect(passkeyDialog(page)).toContainText('Opening a shell needs a fresh passkey check.')
  await passkeyDialog(page).getByRole('button', { name: 'Cancel' }).click()
  const tab = page.locator('[data-tab-body="shell"]')
  await expect(tab.locator('[data-field="end-reason"]')).toContainText('Not confirmed, no shell opened.')

  await tab.getByRole('button', { name: 'New session' }).click()
  await passkeyDialog(page).locator('[data-action="passkey-verify"]').click()
  await expect(tab).toHaveAttribute('data-shell-status', 'active')
  const opens = backend.received.filter((c) => c.type === 'OpenShell')
  expect(opens.at(-1)?.data.assertion).toBeDefined()
})

test('a user change asks for a fresh check, then saves', async ({ page, backend }) => {
  await stale(page, backend)
  backend.users = { users: [], pending: [f.user('discord:3003', { username: 'cleo', role: 'pending' })] }
  await page.goto('/admin/users')

  await page.locator('[data-user="discord:3003"]').getByRole('button', { name: 'Admit cleo' }).click()
  const form = page.getByRole('form', { name: 'Access for cleo' })
  await form.getByRole('checkbox', { name: 'web' }).check()
  await form.getByRole('button', { name: 'Save' }).click()
  await expect(passkeyDialog(page)).toContainText('Changing a user needs a fresh passkey check.')
  await passkeyDialog(page).locator('[data-action="passkey-verify"]').click()

  await expect(passkeyDialog(page)).toHaveCount(0)
  await expect(page.locator('[data-user="discord:3003"]')).not.toContainText('Admit')
  const puts = backend.received.filter((c) => c.type === 'PutUser')
  expect(puts).toHaveLength(2)
  expect(backend.received.filter((c) => c.type === 'PasskeyFinish')).toHaveLength(1)
})

test('062: the Updates settings Confirm carries the assertion', async ({ page, backend }) => {
  const { id } = await stale(page, backend)
  backend.setUpdates(f.updatesView({ settings: f.updateSettings(), next_check: null }))
  await page.goto('/admin/updates')

  await page.locator('[data-action="turn-checks-on"]').click()
  const dialog = page.getByTestId('updates-settings-dialog')
  await expect(dialog.locator('[data-step-up]')).toBeVisible()
  await dialog.locator('[data-action="confirm"]').click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.assertion?.credential.id).toBe(id)
  await expect(dialog).toBeHidden()
  expect(backend.audit.some((e) => e.action === 'update_settings')).toBe(true)
})

test('a plain user with no passkey is asked to register one before a shell', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  seed(backend)
  backend.me = sessions.user(['web'])
  backend.passkeys.enable({ stale: true })
  await page.goto('/stacks/web')

  await container(page, 'web-app-1').locator('[data-open="shell"]').click()
  await expect(passkeyDialog(page)).toHaveAttribute('data-passkey-dialog', 'enrol')
  await expect(page.locator('[data-tab-body="shell"] [data-field="end-reason"]')).toContainText('Opening a shell needs a passkey.')
})
