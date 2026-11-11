/**
 * Shared set-up for spec 061's specs: sign in as a role with the update status scripted, and open
 * Settings → System. The owner's tab also holds spec 060's System workspace, so its fakes are there.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, type FakeBackend } from '../../support/fake-backend'
import { governance } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { FakeSettings } from '../../support/fake-settings'
import type { Me, UpdateStatus } from '../../../src/protocol/wire'

export function signIn(
  backend: FakeBackend,
  instance_role: Me['instance_role'],
  status: UpdateStatus,
  over: Partial<Me> = {},
): FakeSettings {
  backend.me = fixtures.me({
    instance_role,
    keel_version: status.running.display,
    verification: fixtures.verification({ verified: true }),
    ...over,
  })
  backend.updates.status = status
  governance(backend, (gov) => {
    gov.workspace('ops')
  })
  new FakeHosts(backend)
  return new FakeSettings(backend)
}

export async function openSystem(page: Page): Promise<void> {
  await page.goto('/settings/system')
  await expect(page).toHaveURL(/\/settings\/system(\?ws=[^#]*)?$/)
  await expect(page.locator('[data-build-panel]')).toBeVisible()
}

/** The Settings entry in the navigation. */
export function settingsLink(page: Page) {
  return page.getByRole('navigation').getByRole('link', { name: /^Settings/ })
}

/** The account menu's About line. */
export async function aboutVersion(page: Page) {
  await page.getByRole('button', { name: 'Account' }).click()
  return page.locator('[data-keel-version]')
}

/** Registers a passkey on the virtual authenticator through the account menu. */
export async function registerPasskey(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: 'Register a passkey' }).click()
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await enrol.getByLabel('Name').fill('Work laptop')
  await enrol.getByRole('button', { name: 'Register' }).click()
  await expect(page.getByRole('dialog', { name: 'Passkey registered' })).toBeVisible()
  await page.getByRole('button', { name: 'Done' }).click()
}
