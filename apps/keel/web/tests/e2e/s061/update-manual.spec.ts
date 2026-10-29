/**
 * Spec 061 US5 (T079, FR-021): when the update can't run as a Keel run, the card says what is
 * missing in one line, gives the digest reference and the steps to copy, offers no Update button,
 * and links to the System workspace setup on the same tab.
 */
import { expect, test } from '../../support/fake-backend'
import { AVAILABLE_DIGEST, MANUAL_REASONS, updateFixtures } from '../../support/fake-updates'

import { openSystem, signIn } from './support'

const MISSING: Record<(typeof MANUAL_REASONS)[number], string> = {
  no_system_workspace: 'There is no System workspace',
  no_keel_host: 'No host in the System workspace is enrolled with the keel-server profile.',
  profile_not_allowed: 'The System workspace does not allow the keel-server profile.',
  job_missing_param: 'keel_server_image_digest',
}

for (const missing of MANUAL_REASONS) {
  test(`${missing}: the card names it and gives the manual steps`, async ({ page, backend }) => {
    signIn(backend, 'owner', updateFixtures.status('update_available', { update_path: updateFixtures.manualPath(missing) }))
    await openSystem(page)
    const manual = page.getByRole('region', { name: 'Updates' }).locator(`[data-update-manual="${missing}"]`)
    await expect(manual.locator('[data-missing]')).toContainText(MISSING[missing])
    await expect(manual.locator('[data-copy="digest-ref"]')).toContainText(`mbround18/keel@${AVAILABLE_DIGEST}`)
    await expect(manual.locator('[data-copy="step-1"]')).toContainText(`docker pull mbround18/keel@${AVAILABLE_DIGEST}`)
    await expect(manual.locator('[data-copy="step-2"]')).toContainText('docker compose up -d keel')
    await expect(page.getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
  })
}

test('the steps copy, and the setup link goes to the System workspace on this tab', async ({ page, backend, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  signIn(backend, 'owner', updateFixtures.status('update_available', { update_path: updateFixtures.manualPath('no_keel_host') }))
  await openSystem(page)
  const manual = page.locator('[data-update-manual]')
  await manual.getByRole('button', { name: 'Copy image to pin' }).click()
  await expect(manual.getByRole('button', { name: 'Copy image to pin' })).toContainText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`mbround18/keel@${AVAILABLE_DIGEST}`)

  await manual.getByRole('link', { name: 'Set up the System workspace so Keel can update itself' }).click()
  await expect(page).toHaveURL(/\/settings\/system#system-workspace$/)
  await expect(page.locator('#system-workspace')).toBeInViewport()
  expect(backend.received.some((c) => c.type === 'PreviewKeelUpdate')).toBe(false)
})
