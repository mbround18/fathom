/**
 * Spec 061 US2 (T042, T051): the owner's update card and the navigation dot. Checks off, not yet
 * checked and up to date; a pushed update shows the dot and both builds; Check now, and its
 * once-a-minute limit; hiding the dot survives a reload and comes back for a newer image; an older
 * build, a rebuild and can't tell warn without a dot; a failed check; release notes or why not.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { AVAILABLE_DIGEST, updateFixtures } from '../../support/fake-updates'

import { openSystem, settingsLink, signIn } from './support'

const card = (page: Page) => page.getByRole('region', { name: 'Updates' })
const NEWER = `sha256:${'3'.repeat(64)}`

test('with checks off the card says how to turn them on, and offers no Check now', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.off())
  await openSystem(page)
  await expect(card(page).locator('[data-update-state]')).toHaveText('Update checks are off.')
  await expect(card(page)).toContainText('KEEL_UPDATE_CHECK=true')
  await expect(card(page).getByRole('button', { name: 'Check now' })).toHaveCount(0)
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
})

test('not yet checked, then up to date with when it checked and when it checks next', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.notChecked())
  await openSystem(page)
  await expect(card(page).locator('[data-update-state]')).toHaveText('Not checked yet.')
  await expect(card(page).locator('[data-next-check]')).toHaveText('Next check 2026-10-28 18:00 UTC')
  backend.updates.finishCheck(updateFixtures.status('up_to_date'))
  await expect(card(page).locator('[data-update-state]')).toHaveText('Keel is up to date.')
  await expect(card(page).locator('[data-checked-at]')).toHaveText('Checked 2026-10-28 12:00 UTC')
  await expect(card(page)).toContainText('docker.io/mbround18/keel:latest')
  await expect(card(page).locator('[data-build]')).toHaveCount(0)
  await expect(card(page).getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
})

test('a pushed update shows the dot and both builds', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('up_to_date'))
  await openSystem(page)
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
  backend.updates.finishCheck(updateFixtures.status('update_available'))
  await expect(card(page).locator('[data-update-state]')).toHaveText('A newer Keel is available.')
  await expect(page.locator('[data-update-dot]')).toBeVisible()
  await expect(settingsLink(page)).toHaveAccessibleName('System Settings (update available)')
  const running = card(page).locator('[data-build="running"]')
  const available = card(page).locator('[data-build="available"]')
  await expect(running.locator('[data-field="version"]')).toHaveText('0.2.0')
  await expect(available.locator('[data-field="version"]')).toHaveText('0.3.0')
  await expect(available.locator('[data-field="revision"] code')).toHaveText('fedcba9')
  await expect(available.locator('[data-field="digest"] code')).toHaveAttribute('data-full', AVAILABLE_DIGEST)
  await expect(card(page).locator('[data-release-notes]')).toHaveText('Release notes for 0.3.0')
  await expect(card(page).locator('[data-release-notes]')).toHaveAttribute('href', /keel-v0\.3\.0$/)
})

test('the dot shows on every page, not only in Settings', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  await page.goto('/runs')
  await expect(page.locator('[data-update-dot]')).toBeVisible()
})

test('Check now starts a check, and a second within the minute says when to try again', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('up_to_date'))
  await openSystem(page)
  await card(page).getByRole('button', { name: 'Check now' }).click()
  await expect(card(page).locator('[data-update-state]')).toHaveText('Checking for a newer Keel…')
  await expect(card(page).getByRole('button', { name: 'Check now' })).toBeDisabled()
  expect(backend.updates.checks).toBe(1)
  backend.updates.finishCheck(updateFixtures.status('update_available'))
  await expect(card(page).locator('[data-update-state]')).toHaveText('A newer Keel is available.')

  backend.updates.tooSoon = true
  await card(page).getByRole('button', { name: 'Check now' }).click()
  await expect(card(page).locator('[data-check-too-soon]')).toHaveText(
    'Check now runs at most once a minute. Try again at 2026-10-28 12:01 UTC.',
  )
  expect(backend.updates.checks).toBe(1)
})

test('hiding the dot survives a reload and comes back for a newer image', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  await openSystem(page)
  await expect(page.locator('[data-update-dot]')).toBeVisible()
  await card(page).getByRole('button', { name: 'Hide the navigation dot' }).click()
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
  const sent = await backend.waitForCmd('DismissUpdateDot')
  expect(sent.data.digest).toBe(AVAILABLE_DIGEST)
  // The card still says so: hiding the dot hides nothing else.
  await expect(card(page).locator('[data-update-state]')).toHaveText('A newer Keel is available.')
  await expect(card(page).getByRole('button', { name: 'Hide the navigation dot' })).toHaveCount(0)

  await page.reload()
  await expect(page.locator('[data-build-panel]')).toBeVisible()
  await expect(card(page).locator('[data-update-state]')).toHaveText('A newer Keel is available.')
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)

  backend.updates.finishCheck(
    updateFixtures.status('update_available', { available: updateFixtures.available({ digest: NEWER, version: '0.3.1' }) }),
  )
  await expect(page.locator('[data-update-dot]')).toBeVisible()
})

test('an older build warns that updating goes back, with no dot', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('older_build'))
  await openSystem(page)
  await expect(card(page).locator('[data-update-state]')).toHaveText(
    'The watched image is an older build than the one running. Updating to it goes back.',
  )
  await expect(card(page).locator('[data-build="available"] [data-field="version"]')).toHaveText('0.1.9')
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
})

test('a rebuild of the running revision says so, with no dot', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('rebuilt'))
  await openSystem(page)
  await expect(card(page).locator('[data-update-state]')).toHaveText(
    'The watched image is a rebuild of the running revision abc1234: same source, a different image.',
  )
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
})

test('can’t tell says why, and an untagged image has no release notes', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('cant_tell'))
  await openSystem(page)
  await expect(card(page).locator('[data-update-state]')).toHaveText(
    'Keel can’t tell whether the watched image is newer: the image carries no revision label.',
  )
  await expect(card(page).locator('[data-build="available"] [data-field="version"]')).toHaveText('untagged')
  await expect(card(page).locator('[data-release-notes-note]')).toHaveText('The image has no version label.')
  await expect(card(page).locator('[data-release-notes]')).toHaveCount(0)
  await expect(card(page).getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
})

test('an untagged newer build shows its revision and says there are no release notes', async ({ page, backend }) => {
  const available = updateFixtures.available({ version: null, release_notes: null, release_notes_note: null })
  signIn(backend, 'owner', updateFixtures.status('update_available', { available }))
  await openSystem(page)
  await expect(card(page).locator('[data-build="available"] [data-field="revision"] code')).toHaveText('fedcba9')
  await expect(card(page).locator('[data-release-notes-note]')).toHaveText('No release notes for untagged builds.')
  await expect(card(page).locator('[data-release-notes]')).toHaveCount(0)
})

test('without its own digest Keel says it compared by revision', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available', { digest_compare: false }))
  await openSystem(page)
  await expect(card(page).locator('[data-compared-by="revision"]')).toBeVisible()
})

test('a failed check says when and why, keeps the last result and names the next attempt', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.failed('rate_limited'))
  await openSystem(page)
  const error = card(page).locator('[data-update-error="rate_limited"]')
  await expect(error).toContainText('The last check failed at 2026-10-28 12:00 UTC: the registry is rate-limiting Keel.')
  await expect(error).toContainText('Next attempt 2026-10-28 18:00 UTC.')
  await expect(card(page).locator('[data-update-state]')).toHaveText('Keel is up to date.')
})

test('a first check that failed says none has succeeded', async ({ page, backend }) => {
  signIn(backend, 'owner', { ...updateFixtures.notChecked(), last_error: { code: 'network', at: '2026-10-28T12:00:00Z' } })
  await openSystem(page)
  const error = card(page).locator('[data-update-error="network"]')
  await expect(error).toContainText('the registry could not be reached')
  await expect(error).toContainText('No check has succeeded yet.')
})
