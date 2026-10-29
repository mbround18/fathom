/**
 * Spec 062 US3 (T062): Auto updates in a maintenance window, per stack.
 *
 * - The policy sheet (Off, Notify, Auto) previews the change and confirms with the passkey.
 * - The host window is edited in the settings sheet; a stack can have its own window.
 * - The zone is shown; when it is UTC the page says to mount /etc/localtime.
 * - Recent runs show who a scheduled run acted for, and each stack's outcome: rolled back,
 *   rollback failed (red, with its output) and interrupted.
 * - A policy whose author is no longer an administrator is paused, and says so.
 */
import type { Page } from '@playwright/test'

import type { UpdateRun, UpdatesView } from '../../../src/protocol/generated'
import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'
import { seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const row = (page: Page, id: string) => page.locator(`[data-update-row="${id}"]`)
const change = (page: Page) => page.getByTestId('updates-settings-dialog')
const policies = (page: Page) => page.getByTestId('updates-policies')
const runs = (page: Page) => page.getByTestId('updates-runs')

const alice = { provider: 'github' as const, subject: '1001', username: 'alice' }

function seed(backend: FakeBackend, over: Partial<UpdatesView> = {}): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('blog'), f.stack('media')])
  const view = f.updatesView(over)
  view.services = [...view.services, f.serviceUpdate('media', 'sonarr', 'available')]
  backend.setUpdates(view)
}

async function openPolicies(page: Page): Promise<void> {
  const details = policies(page)
  if ((await details.getAttribute('open')) === null) await details.locator('summary').click()
}

for (const width of [360, 1280]) {
  test(`at ${String(width)} px: a stack set to Auto with its own window, confirmed with the passkey`, async ({ page, backend }) => {
    await page.setViewportSize({ width, height: 900 })
    const auth = await virtualAuthenticator(page)
    const phone = await seedPasskey(auth, 'Phone')
    seed(backend)
    backend.passkeys.enable({ items: [phone], verified: true, stale: true })
    await page.goto('/admin/updates')

    await expect(row(page, 'web/app').locator('[data-field="policy"]')).toHaveText('Notify · default')
    await openPolicies(page)
    await policies(page).locator('[data-policy-row="web"] [data-action="edit-policy"]').click()
    const sheet = page.getByTestId('updates-policy-sheet')
    await expect(sheet).toContainText('Checked, and installed in the maintenance window on your behalf.')
    await sheet.locator('[data-policy-option="auto"]').check()
    await sheet.locator('[data-field="own-window"]').check()
    for (const day of ['mon', 'tue', 'wed', 'thu', 'fri']) await sheet.locator(`[data-day="${day}"]`).uncheck()
    await sheet.locator('[data-field="window-start"]').fill('23:00')
    await sheet.locator('[data-field="window-end"]').fill('02:00')
    await expect(sheet).toContainText('ends the next day')
    await sheet.locator('[data-action="preview-policy"]').click()

    const preview = await backend.waitForCmd('PreviewUpdateChange')
    expect(preview.data).toEqual({
      kind: 'policy',
      data: { stack: 'web', policy: 'auto', window: { days: ['sat', 'sun'], start: '23:00', end: '02:00' } },
    })
    await expect(change(page)).toContainText('Change the update policy')
    await expect(change(page).locator('[data-field="summary"]')).toContainText('update policy of web: notify (default) -> auto')
    await expect(change(page).locator('[data-field="summary"]')).toContainText('maintenance window: the host window -> sat,sun 23:00-02:00')
    await expect(change(page).locator('[data-step-up]')).toBeVisible()
    await change(page).locator('[data-action="confirm"]').click()
    const confirm = await backend.waitForCmd('Confirm')
    expect(confirm.data.assertion?.credential.id).toBe(phone.id)

    await expect(change(page)).toHaveCount(0)
    const web = policies(page).locator('[data-policy-row="web"]')
    await expect(web).toHaveAttribute('data-policy', 'auto')
    await expect(web.locator('[data-field="own-window"]')).toHaveText('Sat, Sun 23:00–02:00')
    await expect(web).toContainText('set by')
    await expect(row(page, 'web/app').locator('[data-field="policy"]')).toHaveText('Auto · stack policy')

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
  })
}

test('the host window is edited in the settings sheet, and an empty window cannot be saved', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')
  await page.locator('[data-action="edit-settings"]').click()
  const sheet = page.getByTestId('updates-settings-sheet')
  await expect(sheet.locator('[data-action="preview-settings"]')).toBeDisabled()
  for (const day of ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']) await sheet.locator(`[data-day="${day}"]`).uncheck()
  await expect(sheet.locator('[data-field="window-problem"]')).toHaveText('Choose at least one day.')
  await expect(sheet.locator('[data-action="preview-settings"]')).toBeDisabled()
  await sheet.locator('[data-day="sun"]').check()
  await sheet.locator('[data-field="window-start"]').fill('01:00')
  await sheet.locator('[data-action="preview-settings"]').click()

  await expect(change(page).locator('[data-field="summary"]')).toHaveText(
    'maintenance window: mon,tue,wed,thu,fri,sat,sun 03:00-05:00 -> sun 01:00-05:00',
  )
  await change(page).locator('[data-action="confirm"]').click()
  await expect(change(page)).toHaveCount(0)
  expect(backend.updates?.settings.window).toEqual({ days: ['sun'], start: '01:00', end: '05:00' })
})

test('the zone is shown, and UTC says to mount /etc/localtime', async ({ page, backend }) => {
  seed(backend, { zone: 'UTC (no /etc/localtime)' })
  await page.goto('/admin/updates')
  await expect(page.locator('[data-field="zone-utc"]')).toHaveText('Windows are in UTC: mount /etc/localtime to use host time.')
  await page.locator('[data-action="edit-settings"]').click()
  await expect(page.getByTestId('updates-settings-sheet').locator('[data-field="zone-utc"]')).toBeVisible()
})

test('a zone from /etc/localtime shows no UTC note', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')
  await expect(page.getByTestId('updates-schedule')).toBeVisible()
  await expect(page.locator('[data-field="zone-utc"]')).toHaveCount(0)
})

test('recent runs show rolled back, rollback failed in red with its output, and interrupted', async ({ page, backend }) => {
  const scheduled: UpdateRun = {
    run_id: 'run-old',
    trigger: { kind: 'scheduled', on_behalf_of: { web: alice } },
    stacks: [{ stack: 'web', services: [], operation_id: null, phase: 'done', outcome: { kind: 'updated' } }],
    started_at: '2026-10-27T03:00:00Z',
    finished_at: '2026-10-27T03:02:00Z',
  }
  seed(backend, { recent_runs: [scheduled] })
  backend.updateRuns.scripts.blog = { outcome: { kind: 'rolled_back' }, stderr: 'rolled back: ghost unhealthy after 120 s' }
  backend.updateRuns.scripts.media = {
    outcome: { kind: 'rollback_failed' },
    stderr: 'sonarr unhealthy after 120 s; rollback failed: image sha256:aaaa1111 is gone',
  }
  backend.updateRuns.scripts.web = { outcome: { kind: 'interrupted' }, stderr: 'Depths restarted during the update' }
  await page.goto('/admin/updates')

  await runs(page).locator('summary').click()
  await expect(runs(page).locator('[data-run="run-old"]')).toContainText('scheduled, on behalf of alice')

  await page.locator('[data-action="select-all"]').check()
  await page.locator('[data-action="preview-batch"]').click()
  await page.getByTestId('updates-batch-dialog').locator('[data-action="confirm-batch"]').click()
  const live = page.getByTestId('updates-run-dialog')
  await expect(live).toHaveAttribute('data-run-finished', 'true')
  await expect(live.locator('[data-field="run-summary"]')).toHaveText(
    '0 of 3 updated. Not updated: blog (Rolled back), media (Rollback failed), web (Interrupted).',
  )
  await live.locator('[data-action="close-run"]').click()

  const latest = runs(page).locator('[data-run]').first()
  await expect(latest.locator('[data-run-stack="blog"]')).toHaveAttribute('data-outcome', 'rolled_back')
  await expect(latest.locator('[data-run-stack="blog"] [data-field="run-output"]')).toHaveText('rolled back: ghost unhealthy after 120 s')
  const media = latest.locator('[data-run-stack="media"]')
  await expect(media).toHaveAttribute('data-outcome', 'rollback_failed')
  await expect(media).toContainText('Rollback failed')
  await expect(media.locator('[data-field="run-output"]')).toContainText('rollback failed: image sha256:aaaa1111 is gone')
  await expect(media.locator('[data-field="run-output"]')).toHaveClass(/text-status-err/)
  await expect(latest.locator('[data-run-stack="web"]')).toHaveAttribute('data-outcome', 'interrupted')
  await expect(row(page, 'media/sonarr').locator('[data-update-state]')).toHaveText('Rollback failed')
})

test('a policy whose author is no longer an administrator is paused until it is saved again', async ({ page, backend }) => {
  seed(backend, {
    policies: { web: { policy: 'auto', window: null, set_by: alice, set_at: '2026-10-01T09:00:00Z' } },
    paused_policies: ['web'],
  })
  await page.goto('/admin/updates')

  await expect(policies(page)).toHaveAttribute('open', '')
  await expect(policies(page).locator('summary')).toContainText('1 paused')
  const web = policies(page).locator('[data-policy-row="web"]')
  await expect(web.locator('[data-field="paused"]')).toHaveText('paused: author no longer an administrator')
  await expect(web).toContainText('set by alice')
  await web.locator('[data-action="resume-policy"]').click()
  const preview = await backend.waitForCmd('PreviewUpdateChange')
  expect(preview.data).toEqual({ kind: 'policy', data: { stack: 'web', policy: 'auto' } })
  await change(page).locator('[data-action="confirm"]').click()
  await expect(web.locator('[data-field="paused"]')).toHaveCount(0)
})
