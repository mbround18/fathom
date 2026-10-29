/**
 * Spec 062 US5 (T077): replaced images are removed when cleanup is on.
 *
 * - The cleanup setting is changed in the settings sheet, previewed, and confirmed with the passkey.
 * - After an update, each removed image is listed in recent runs and in the audit log.
 * - An old image another container still uses is kept, and the run output says so.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'
import { seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const change = (page: Page) => page.getByTestId('updates-settings-dialog')
const run = (page: Page) => page.getByTestId('updates-run-dialog')

function seed(backend: FakeBackend, cleanup: boolean): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('blog')])
  backend.setUpdates(f.updatesView({ settings: f.updateSettings({ checks_enabled: true, cleanup }) }))
}

for (const width of [360, 1280]) {
  test(`at ${String(width)} px: cleanup is turned on with the passkey`, async ({ page, backend }) => {
    await page.setViewportSize({ width, height: 900 })
    const auth = await virtualAuthenticator(page)
    const phone = await seedPasskey(auth, 'Phone')
    seed(backend, false)
    backend.passkeys.enable({ items: [phone], verified: true, stale: true })
    await page.goto('/admin/updates')

    await expect(page.locator('[data-field="cleanup-state"]')).toHaveText('Replaced images are kept')
    await page.locator('[data-action="edit-settings"]').click()
    const sheet = page.getByTestId('updates-settings-sheet')
    await expect(sheet).toContainText('An image another container still uses is kept.')
    await sheet.locator('[data-field="cleanup"]').check()
    await sheet.locator('[data-action="preview-settings"]').click()

    const preview = await backend.waitForCmd('PreviewUpdateChange')
    expect(preview.data.kind).toBe('settings')
    await expect(change(page).locator('[data-field="summary"]')).toHaveText('remove replaced images: off -> on')
    await expect(change(page).locator('[data-step-up]')).toBeVisible()
    await change(page).locator('[data-action="confirm"]').click()
    const confirm = await backend.waitForCmd('Confirm')
    expect(confirm.data.assertion?.credential.id).toBe(phone.id)

    await expect(change(page)).toHaveCount(0)
    await expect(page.locator('[data-field="cleanup-state"]')).toHaveText('Replaced images are removed')
    expect(backend.updates?.settings.cleanup).toBe(true)

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
  })
}

test('removed images show in recent runs and the audit log; an image in use is kept', async ({ page, backend }) => {
  seed(backend, true)
  backend.updateRuns.scripts.web = {
    removed: [{ id: 'sha256:1111aaaa2222', digests: ['nginx@sha256:1111aaaa2222bbbb'] }],
    keptInUse: [{ id: 'sha256:3333cccc4444', why: 'used by container web-worker-1' }],
  }
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  await page.locator('[data-action="preview-batch"]').click()
  await page.getByTestId('updates-batch-dialog').locator('[data-action="confirm-batch"]').click()

  await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
  const output = run(page).locator('[data-run-stack="web"] [data-field="output"]')
  await expect(output.locator('[data-kept]')).toHaveText('kept: in use: sha256:3333cccc4444 (used by container web-worker-1)')
  await run(page).locator('[data-action="close-run"]').click()

  const runs = page.getByTestId('updates-runs')
  await runs.locator('summary').click()
  await expect(runs.locator('[data-run]').first().locator('[data-run-stack="web"] [data-field="removed"]')).toHaveText(
    'removed sha256:1111aaaa2222 (nginx@sha256:1111aaaa2222bbbb)',
  )

  await page.goto('/admin/audit')
  const removed = page.locator('[data-audit-row][data-action="image_remove"]')
  await expect(removed).toContainText('Old image removed')
  await expect(removed).toContainText('docker image rm sha256:1111aaaa2222')
  await expect(removed).toContainText('removed sha256:1111aaaa2222 (nginx@sha256:1111aaaa2222bbbb)')
  const updated = page.locator('[data-audit-row][data-action="image_update"]')
  await expect(updated).toContainText('app: ')
  await expect(updated).toContainText('->')
})

test('with cleanup off nothing is removed and no removal is listed', async ({ page, backend }) => {
  seed(backend, false)
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  await page.locator('[data-action="preview-batch"]').click()
  await page.getByTestId('updates-batch-dialog').locator('[data-action="confirm-batch"]').click()
  await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
  await run(page).locator('[data-action="close-run"]').click()
  const runs = page.getByTestId('updates-runs')
  await runs.locator('summary').click()
  await expect(runs.locator('[data-run-stack="web"]')).toHaveAttribute('data-outcome', 'updated')
  await expect(runs.locator('[data-field="removed"]')).toHaveCount(0)
})
