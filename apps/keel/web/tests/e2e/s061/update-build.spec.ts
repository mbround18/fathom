/**
 * Spec 061 US1 (T025): the running build. Owners and admins see version, revision, digest and
 * build date in Settings → System; a development build and a deployment without a digest say so;
 * everyone sees the version under About in the account menu, and users get no System tab.
 */
import { expect, test } from '../../support/fake-backend'
import { RUNNING_DIGEST, updateFixtures } from '../../support/fake-updates'

import { aboutVersion, openSystem, signIn } from './support'

for (const role of ['owner', 'admin'] as const) {
  test(`an ${role} sees the running build`, async ({ page, backend }) => {
    signIn(backend, role, updateFixtures.status('up_to_date'))
    await openSystem(page)
    const panel = page.getByRole('region', { name: 'Running build' })
    await expect(panel.locator('[data-field="version"]')).toHaveText('0.2.0')
    await expect(panel.locator('[data-field="revision"] code')).toHaveText('abc1234')
    await expect(panel.locator('[data-field="revision"] [data-full]')).toHaveAttribute('data-full', 'abc1234def5678')
    await expect(panel.locator('[data-field="digest"] code')).toHaveText(`sha256:${'1'.repeat(12)}`)
    await expect(panel.locator('[data-field="digest"] [data-full]')).toHaveAttribute('data-full', RUNNING_DIGEST)
    await expect(panel.getByRole('button', { name: 'Copy full digest' })).toBeVisible()
    await expect(panel.locator('[data-field="built"]')).toContainText('2026-10-01 12:00 UTC')
    expect(backend.received.some((c) => c.type === 'GetKeelBuild')).toBe(true)
  })
}

test('a development build and a missing digest say so', async ({ page, backend }) => {
  const running = updateFixtures.running({ version: null, revision: null, built_at: null, digest: null, display: 'development build' })
  signIn(backend, 'owner', { ...updateFixtures.off(), running })
  await openSystem(page)
  const panel = page.getByRole('region', { name: 'Running build' })
  await expect(panel.locator('[data-field="version"]')).toHaveText('development build')
  await expect(panel.locator('[data-field="revision"]')).toContainText('development build')
  await expect(panel.locator('[data-field="built"]')).toContainText('development build')
  await expect(panel.locator('[data-field="digest"]')).toContainText('unknown')
  await expect(panel.locator('[data-hint="pin-digest"]')).toContainText('KEEL_IMAGE_DIGEST')
})

test('an untagged build shows its revision with no version', async ({ page, backend }) => {
  const running = updateFixtures.running({ version: null, display: 'abc1234' })
  signIn(backend, 'admin', { ...updateFixtures.off(), running })
  await openSystem(page)
  const panel = page.getByRole('region', { name: 'Running build' })
  await expect(panel.locator('[data-field="version"]')).toHaveText('abc1234')
  await expect(panel.locator('[data-field="revision"] code')).toHaveText('abc1234')
  await expect(panel.locator('[data-hint="pin-digest"]')).toHaveCount(0)
})

test('the version sits under About in the account menu', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.off())
  await page.goto('/runs')
  await expect(await aboutVersion(page)).toHaveText('Keel 0.2.0')
})

test('a user sees the version under About, and no System tab', async ({ page, backend }) => {
  signIn(backend, 'user', updateFixtures.status('update_available'))
  await page.goto('/runs')
  await expect(await aboutVersion(page)).toHaveText('Keel 0.2.0')
  await page.keyboard.press('Escape')
  await page.goto('/settings/system')
  await expect(page.getByRole('tab', { name: 'System' })).toHaveCount(0)
  await expect(page.locator('[data-build-panel]')).toHaveCount(0)
  await expect(page.locator('[data-update-dot]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'GetUpdateStatus')).toBe(false)
})

test('a Keel that does not know its version says so', async ({ page, backend }) => {
  signIn(backend, 'user', updateFixtures.off(), { keel_version: null })
  await page.goto('/runs')
  await expect(await aboutVersion(page)).toHaveText('Keel version unknown')
})
