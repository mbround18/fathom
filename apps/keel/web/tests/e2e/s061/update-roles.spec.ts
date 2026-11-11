/**
 * Spec 061 US1 and US2 by role (T026, T052; FR-015, FR-024, SC-006). An admin sees the System tab
 * with the running build and nothing of spec 060's; with an update available, one read-only line
 * and nothing about the image: no dot, no digest or version, no Check now or Update. A user gets no
 * tab and no status.
 */
import { expect, test } from '../../support/fake-backend'
import { AVAILABLE_DIGEST, updateFixtures } from '../../support/fake-updates'

import { openSystem, settingsLink, signIn } from './support'

test('an admin sees the build panel and none of the System workspace', async ({ page, backend }) => {
  signIn(backend, 'admin', updateFixtures.status('up_to_date'))
  await openSystem(page)
  await expect(page.locator('[data-system]')).toHaveCount(0)
  await expect(page.locator('#system-workspace')).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'Updates' })).toHaveCount(0)
  // Up to date: an admin reads nothing about updates.
  await expect(page.locator('[data-update-line]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'GetSystem')).toBe(false)
  expect(backend.received.some((c) => c.type === 'GetUpdateStatus')).toBe(true)
})

test('an admin reads one line when an update is available, and nothing about the image', async ({ page, backend }) => {
  signIn(backend, 'admin', updateFixtures.status('update_available'))
  await openSystem(page)
  const line = page.locator('[data-update-line]')
  await expect(line).toHaveText('A newer Keel is available; an owner can update it.')
  const tab = page.getByRole('tabpanel')
  await expect(tab).not.toContainText('0.3.0')
  await expect(tab).not.toContainText('2'.repeat(12))
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
  await expect(settingsLink(page)).toHaveAccessibleName('Settings')
  await expect(page.getByRole('button', { name: 'Check now' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Hide the navigation dot' })).toHaveCount(0)
  // Nothing the admin was sent carried the image either.
  expect(backend.received.some((c) => c.type === 'CheckForUpdates' || c.type === 'PreviewKeelUpdate')).toBe(false)
})

test('an admin line follows the pushed status', async ({ page, backend }) => {
  signIn(backend, 'admin', updateFixtures.status('up_to_date'))
  await openSystem(page)
  await expect(page.locator('[data-update-line]')).toHaveCount(0)
  backend.updates.finishCheck(updateFixtures.status('update_available'))
  await expect(page.locator('[data-update-line]')).toBeVisible()
  backend.updates.finishCheck(updateFixtures.status('up_to_date'))
  await expect(page.locator('[data-update-line]')).toHaveCount(0)
})

test('an owner sees the card and the System workspace below the build', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  await openSystem(page)
  await expect(page.getByRole('region', { name: 'Updates' })).toBeVisible()
  await expect(page.locator('#system-workspace')).toBeVisible()
  await expect(page.locator('[data-update-line]')).toHaveCount(0)
  await expect(page.locator('[data-build="available"] [data-full]').last()).toHaveAttribute('data-full', AVAILABLE_DIGEST)
})

test('a user has no settings tabs and asks for no status', async ({ page, backend }) => {
  signIn(backend, 'user', updateFixtures.status('update_available'))
  await page.goto('/settings/system')
  await expect(page.getByRole('tablist', { name: 'Settings' })).toHaveCount(0)
  await expect(page.locator('[data-build-panel]')).toHaveCount(0)
  await expect(page.locator('[data-update-line]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'GetUpdateStatus')).toBe(false)
})
