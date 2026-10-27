/**
 * Account → Passkeys (spec 055): list, add, remove with step-up, and the last passkey refused.
 * Chromium's virtual authenticator answers the passkey prompts, so `navigator.credentials` runs
 * for real; the fake backend (support/fake-backend.ts) behaves as the server does.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test } from '../../support/fake-backend'
import { virtualAuthenticator } from '../../support/webauthn'

const OLD_PHONE = { id: 'b2xkLXBob25l', name: 'Old phone', created: '2026-09-01T08:00:00Z', last_used: null }
const DESK_KEY = {
  id: 'ZGVzay1rZXk',
  name: 'Desk key',
  created: '2026-09-02T08:00:00Z',
  last_used: '2026-10-26T17:30:00Z',
}

async function openFromMenu(page: Page): Promise<void> {
  await page.goto('/runs')
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: 'Passkeys', exact: true }).click()
  await expect(page).toHaveURL(/\/account\/passkeys$/)
  await expect(page.getByRole('heading', { name: 'Passkeys' })).toBeVisible()
}

const list = (page: Page) => page.getByRole('list', { name: 'Your passkeys' })

/** Registers a passkey on the virtual authenticator through the page's Add action. */
async function addHere(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'Add passkey' }).click()
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await enrol.getByLabel('Name').fill(name)
  await enrol.getByRole('button', { name: 'Register' }).click()
  await page.getByRole('dialog', { name: 'Passkey registered' }).getByRole('button', { name: 'Done' }).click()
  await expect(list(page).getByText(name, { exact: true })).toBeVisible()
}

test('lists your passkeys from the account menu', async ({ page, backend }) => {
  backend.passkeys = [OLD_PHONE, DESK_KEY]
  await openFromMenu(page)
  const rows = list(page).getByRole('listitem')
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toContainText('Old phone')
  await expect(rows.nth(0)).toContainText('Never used')
  await expect(rows.nth(0).locator('[data-field="created"] time')).toHaveAttribute('datetime', OLD_PHONE.created)
  await expect(rows.nth(1).locator('[data-field="last-used"] time')).toHaveAttribute('datetime', DESK_KEY.last_used)
  await expect(page.getByRole('button', { name: 'Remove Desk key' })).toBeVisible()
})

test('fits a phone screen', async ({ page, backend }) => {
  backend.passkeys = [OLD_PHONE, DESK_KEY]
  await page.setViewportSize({ width: 360, height: 720 })
  await page.goto('/account/passkeys')
  await expect(page.getByRole('button', { name: 'Remove Desk key' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Add passkey' })).toBeVisible()
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  expect(overflow).toBeLessThanOrEqual(0)
})

test('shows the empty state', async ({ page }) => {
  await page.goto('/account/passkeys')
  await expect(page.locator('[data-pane-state="empty"]')).toContainText('no passkeys yet')
})

test('shows a load error and retries', async ({ page, backend }) => {
  let fail = true
  backend.on('ListPasskeys', () =>
    fail ? [{ type: 'OpError', data: { code: 'db', message: 'The database is unavailable.' } }] : undefined,
  )
  backend.passkeys = [DESK_KEY]
  await page.goto('/account/passkeys')
  const error = page.locator('[data-pane-state="error"]')
  await expect(error).toContainText('The database is unavailable.')
  fail = false
  await error.getByRole('button', { name: 'Retry' }).click()
  await expect(list(page).getByRole('listitem')).toHaveCount(1)
})

test('adds a passkey and refreshes the list', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  await page.goto('/account/passkeys')
  await expect(page.locator('[data-pane-state="empty"]')).toBeVisible()
  await addHere(page, 'Work laptop')
  expect(backend.passkeys.map((p) => p.name)).toEqual(['Work laptop'])
  expect(backend.received.filter((c) => c.type === 'ListPasskeys').length).toBeGreaterThanOrEqual(2)
})

test('removes a passkey after confirming and checking it is you', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  await page.goto('/account/passkeys')
  await addHere(page, 'Work laptop')
  // A second passkey, on another device: the fake has it, the virtual authenticator does not.
  backend.passkeys.unshift(OLD_PHONE)
  backend.push({ type: 'Passkeys', data: { items: backend.passkeys } })
  await expect(list(page).getByRole('listitem')).toHaveCount(2)

  await page.getByRole('button', { name: 'Remove Old phone' }).click()
  const confirm = page.getByRole('dialog', { name: 'Remove passkey' })
  await expect(confirm).toContainText('Remove the passkey “Old phone”')
  await expect(confirm).toContainText('ask for one of your passkeys')
  await confirm.getByRole('button', { name: 'Remove' }).click()

  const stepUp = page.getByRole('dialog', { name: 'Verify it’s you' })
  await expect(stepUp).toContainText('Remove the passkey “Old phone”')
  await expect(confirm).toBeHidden()
  await stepUp.getByRole('button', { name: 'Use passkey' }).click()

  await expect(list(page).getByRole('listitem')).toHaveCount(1)
  await expect(list(page)).not.toContainText('Old phone')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  const sent = await backend.waitForCmd('Confirm')
  const workLaptop = backend.passkeys[0]
  expect(sent.data.step_up?.credential.id).toBe(workLaptop?.id)
  expect(backend.received.some((c) => c.type === 'PasskeyFinish' && 'authenticatorData' in c.data.credential.response)).toBe(false)
  await expect(page.getByRole('heading', { name: 'Passkeys' })).toBeVisible()
})

test('cancelling the passkey check keeps the passkey', async ({ page, backend }) => {
  backend.passkeys = [OLD_PHONE, DESK_KEY]
  await page.goto('/account/passkeys')
  await page.getByRole('button', { name: 'Remove Desk key' }).click()
  await page.getByRole('dialog', { name: 'Remove passkey' }).getByRole('button', { name: 'Remove' }).click()
  await page.getByRole('dialog', { name: 'Verify it’s you' }).getByRole('button', { name: 'Cancel' }).click()
  const confirm = page.getByRole('dialog', { name: 'Remove passkey' })
  await expect(confirm).toBeVisible()
  await confirm.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(list(page).getByRole('listitem')).toHaveCount(2)
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})

test('refuses to remove the last passkey', async ({ page, backend }) => {
  backend.passkeys = [DESK_KEY]
  await page.goto('/account/passkeys')
  await page.getByRole('button', { name: 'Remove Desk key' }).click()
  const dialog = page.getByRole('dialog', { name: 'Remove passkey' })
  await expect(dialog.getByRole('alert')).toHaveText('This is your last passkey, so it cannot be removed.')
  await dialog.getByRole('button', { name: 'OK' }).click()
  await expect(dialog).toBeHidden()
  await expect(list(page).getByRole('listitem')).toHaveCount(1)
  expect(backend.passkeys).toEqual([DESK_KEY])
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})

test('a session revoked during a removal signs out without a fight', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  backend.me = fixtures.me()
  await page.goto('/account/passkeys')
  await addHere(page, 'Work laptop')
  backend.passkeys.unshift(OLD_PHONE)
  backend.push({ type: 'Passkeys', data: { items: backend.passkeys } })
  backend.on('Confirm', (_cmd, { backend: b }) => {
    b.revoke()
    return []
  })
  await page.getByRole('button', { name: 'Remove Old phone' }).click()
  await page.getByRole('dialog', { name: 'Remove passkey' }).getByRole('button', { name: 'Remove' }).click()
  await page.getByRole('dialog', { name: 'Verify it’s you' }).getByRole('button', { name: 'Use passkey' }).click()

  await expect(page.locator('[data-notice="revoked"]')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  expect(backend.sockets).toHaveLength(1)
})
