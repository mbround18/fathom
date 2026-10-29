/**
 * Spec 062 US2 (T050): update the stacks you choose, one at a time, with a preview first.
 *
 * - Select stacks with a newer image; the preview lists each stack in order with its services,
 *   old and new image, the services left alone, and the 051 findings to acknowledge.
 * - Confirm asks for the passkey (054) and carries the assertion.
 * - The run shows each stack's live output and outcome; the summary names a failed middle stack
 *   while the stacks after it still update.
 * - A held image (a breaking-change label) shows its reason and "Apply anyway".
 * - A stack another operation holds is skipped. Stop lets the stack in progress finish and skips
 *   the rest.
 * - A batch, or the stack page's Update, with nothing left to update says everything is up to date:
 *   information, not an error.
 * - A hand-pulled image (no name left) is updated after a note that its old image may not come
 *   back if the update fails, in the batch preview and on the stack page.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'
import { seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const row = (page: Page, id: string) => page.locator(`[data-update-row="${id}"]`)
const batch = (page: Page) => page.getByTestId('updates-batch-dialog')
const run = (page: Page) => page.getByTestId('updates-run-dialog')
const runStack = (page: Page, stack: string) => run(page).locator(`[data-run-stack="${stack}"]`)
const stackPreview = (page: Page) => page.locator('[role="dialog"][data-preview="update"]')

const ROLLBACK_NOTE =
  'app: if this update fails, Depths may not be able to put the old image back. It was pulled by hand and has no tag left.'

/** Three stacks with a newer image (web, blog, media) and the rest of the US1 fixture. */
function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('blog'), f.stack('media')])
  const view = f.updatesView()
  view.services = [
    ...view.services,
    f.serviceUpdate('media', 'sonarr', 'available', { reference: 'lscr.io/linuxserver/sonarr:latest' }),
  ]
  backend.setUpdates(view)
}

async function selectAll(page: Page): Promise<void> {
  await page.locator('[data-action="select-all"]').check()
  await expect(page.locator('[data-action="preview-batch"]')).toHaveText('Update 3 stacks')
}

for (const width of [360, 1280]) {
  test(`at ${String(width)} px: preview, passkey, live progress and a failed middle stack`, async ({ page, backend }) => {
    await page.setViewportSize({ width, height: 900 })
    const auth = await virtualAuthenticator(page)
    const phone = await seedPasskey(auth, 'Phone')
    seed(backend)
    backend.passkeys.enable({ items: [phone], verified: true, stale: true })
    backend.updateRuns.scripts.blog = { outcome: { kind: 'rolled_back' }, stderr: 'health: unhealthy after 120 s' }
    await page.goto('/admin/updates')

    await selectAll(page)
    await page.locator('[data-action="preview-batch"]').click()
    const preview = await backend.waitForCmd('PreviewUpdateBatch')
    expect(preview.data.stacks).toEqual(['blog', 'media', 'web'])

    await expect(batch(page)).toContainText('Update 3 stacks')
    const plans = batch(page).locator('[data-plan]')
    await expect(plans).toHaveCount(3)
    await expect(plans.nth(0)).toHaveAttribute('data-plan', 'blog')
    const web = batch(page).locator('[data-plan="web"]')
    await expect(web.locator('[data-plan-service="app"]')).toContainText('1.0.0 (1111aaaa2222)')
    await expect(web.locator('[data-plan-service="app"]')).toContainText('1.1.0 (9999ffff8888)')
    await expect(web.locator('[data-plan-skipped="cache"]')).toContainText('cache: up to date')
    await expect(web.locator('[data-field="recreate-note"]')).toHaveText('Only services with a newer image are recreated.')
    await expect(batch(page).locator('[data-step-up]')).toContainText('Confirming asks for your passkey')

    await batch(page).locator('[data-action="confirm-batch"]').click()
    const confirm = await backend.waitForCmd('Confirm')
    expect(confirm.data.assertion?.credential.id).toBe(phone.id)

    await expect(batch(page)).toHaveCount(0)
    await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
    await expect(runStack(page, 'blog').locator('[data-field="outcome"]')).toHaveText('Rolled back')
    await expect(runStack(page, 'media').locator('[data-field="outcome"]')).toHaveText('Updated')
    await expect(runStack(page, 'web').locator('[data-field="outcome"]')).toHaveText('Updated')
    await expect(runStack(page, 'web').locator('[data-field="output"]')).toContainText('recreate app')
    await expect(run(page).locator('[data-field="run-summary"]')).toHaveText('2 of 3 updated. Not updated: blog (Rolled back).')

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)

    await run(page).locator('[data-action="close-run"]').click()
    await expect(row(page, 'web/app').locator('[data-update-state]')).toHaveText('Up to date')
    await expect(row(page, 'blog/ghost').locator('[data-update-state]')).toHaveText('Rejected')
    const runs = page.getByTestId('updates-runs')
    await runs.locator('summary').click()
    await expect(runs.locator('[data-run-stack="blog"]')).toHaveAttribute('data-outcome', 'rolled_back')
  })
}

test('the output streams in while a stack updates', async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.pauseBefore = 'media'
  backend.updateRuns.scripts.media = { output: ['pull sonarr: done\n', 'recreate sonarr\n'] }
  await page.goto('/admin/updates')
  await selectAll(page)
  await page.locator('[data-action="preview-batch"]').click()
  await batch(page).locator('[data-action="confirm-batch"]').click()

  await expect(runStack(page, 'blog').locator('[data-field="outcome"]')).toHaveText('Updated')
  await expect(runStack(page, 'media').locator('[data-field="outcome"]')).toHaveText('running…')
  await expect(runStack(page, 'media').locator('[data-field="output"]')).toContainText('recreate sonarr')
  await expect(runStack(page, 'web').locator('[data-field="outcome"]')).toHaveText('waiting')
  await expect(run(page)).toHaveAttribute('data-run-finished', 'false')

  backend.updateRuns.resume()
  await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
  await expect(run(page).locator('[data-field="run-summary"]')).toHaveText('3 of 3 updated.')
})

test('Stop lets the stack in progress finish and skips the rest', async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.pauseBefore = 'blog'
  await page.goto('/admin/updates')
  await selectAll(page)
  await page.locator('[data-action="preview-batch"]').click()
  await batch(page).locator('[data-action="confirm-batch"]').click()

  await expect(runStack(page, 'blog').locator('[data-field="outcome"]')).toHaveText('running…')
  await run(page).locator('[data-action="stop-run"]').click()
  await backend.waitForCmd('Cancel')
  await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
  await expect(runStack(page, 'blog').locator('[data-field="outcome"]')).toHaveText('Updated')
  await expect(runStack(page, 'media').locator('[data-field="outcome"]')).toHaveText('Skipped: cancelled')
  await expect(runStack(page, 'web').locator('[data-field="outcome"]')).toHaveText('Skipped: cancelled')
  await expect(run(page).locator('[data-field="run-summary"]')).toContainText('1 of 3 updated.')
})

test('a busy stack is skipped and named in the summary', async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.busy.media = 'busy: alice deploy'
  await page.goto('/admin/updates')
  await selectAll(page)
  await page.locator('[data-action="preview-batch"]').click()
  await batch(page).locator('[data-action="confirm-batch"]').click()

  await expect(run(page)).toHaveAttribute('data-run-finished', 'true')
  await expect(runStack(page, 'media').locator('[data-field="outcome"]')).toHaveText('Skipped: busy: alice deploy')
  await expect(runStack(page, 'media').locator('[data-field="output"]')).toHaveCount(0)
  await expect(run(page).locator('[data-field="run-summary"]')).toHaveText(
    '2 of 3 updated. Not updated: media (Skipped: busy: alice deploy).',
  )
})

test('a batch with nothing left to update says everything is up to date', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  // Updated elsewhere since the page last heard: the server has nothing left to recreate.
  const view = backend.updates
  if (view === null) throw new Error('no updates view')
  view.services = view.services.map((s) => (s.stack === 'web' ? { ...s, state: 'up_to_date', available: null } : s))
  await page.locator('[data-action="preview-batch"]').click()

  await expect(batch(page).locator('[data-up-to-date]')).toHaveText('Everything is up to date.')
  await expect(batch(page).locator('[role="alert"]')).toHaveCount(0)
  await expect(batch(page).locator('[data-action="confirm-batch"]')).toHaveCount(0)
  await expect(batch(page).locator('[data-plan-skipped]').first()).toContainText('up to date')
  await batch(page).getByRole('button', { name: 'Close' }).first().click()
  await expect(batch(page)).toHaveCount(0)
})

test('findings must be acknowledged before the batch starts', async ({ page, backend }) => {
  seed(backend)
  backend.composePolicy = 'enforce'
  backend.lint.web = [{ service: 'app', code: 'privileged', severity: 'escape', message: 'runs privileged: it can reach the host', line: 4 }]
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  await expect(page.locator('[data-action="preview-batch"]')).toHaveText('Update 1 stack')
  await page.locator('[data-action="preview-batch"]').click()

  await expect(batch(page).locator('[data-plan="web"] [data-field="findings"]')).toContainText(
    'app: runs privileged: it can reach the host',
  )
  const go = batch(page).locator('[data-action="confirm-batch"]')
  await expect(go).toBeDisabled()
  await batch(page).locator('[data-field="acknowledge"]').check()
  await go.click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.acknowledge_findings).toBe(true)
  await expect(run(page).locator('[data-field="run-summary"]')).toHaveText('1 of 1 updated.')
})

test('a held image shows its reason, and Apply anyway updates it after a preview', async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.scripts.web = { outcome: { kind: 'held' }, hold: 'held: the new image is labelled as a breaking change (2.0.0)' }
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  await page.locator('[data-action="preview-batch"]').click()
  await batch(page).locator('[data-action="confirm-batch"]').click()
  await expect(runStack(page, 'web').locator('[data-field="outcome"]')).toHaveText('Held')
  await run(page).locator('[data-action="close-run"]').click()

  const held = row(page, 'web/app')
  await expect(held.locator('[data-update-state]')).toHaveText('Held')
  await expect(held.locator('[data-field="reason"]')).toContainText('labelled as a breaking change')
  await held.locator('[data-action="apply-held"]').click()
  const change = await backend.waitForCmd('PreviewUpdateChange')
  expect(change.data).toEqual({ kind: 'apply_held', data: { stack: 'web', service: 'app', digest: 'sha256:9999ffff8888eeee7777dddd' } })

  const dialog = page.getByTestId('updates-settings-dialog')
  await expect(dialog).toContainText('Apply the held image')
  await dialog.locator('[data-action="confirm"]').click()
  await expect(run(page).locator('[data-field="run-summary"]')).toHaveText('1 of 1 updated.')
  await run(page).locator('[data-action="close-run"]').click()
  await expect(row(page, 'web/app').locator('[data-update-state]')).toHaveText('Up to date')
})

test('a user sees no selection and no batch bar', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'])
  await page.goto('/stacks/web')
  await expect(page.getByTestId('updates-batch-bar')).toHaveCount(0)
  await expect(page.locator('[data-select-stack]')).toHaveCount(0)
})

test('a hand-pulled image is updated, after a note that its old image may not come back', async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.unnamed.add('web/app')
  await page.goto('/admin/updates')
  await page.locator('[data-select-stack="web"]').first().check()
  await page.locator('[data-action="preview-batch"]').click()

  const web = batch(page).locator('[data-plan="web"]')
  await expect(web.locator('[data-rollback-note="app"]')).toHaveText(ROLLBACK_NOTE)
  // A note, not a compose finding: nothing to acknowledge.
  await expect(web.locator('[data-field="findings"]')).toHaveCount(0)
  await expect(batch(page).locator('[data-field="acknowledge"]')).toHaveCount(0)
  await batch(page).locator('[data-action="confirm-batch"]').click()
  await expect(run(page).locator('[data-field="run-summary"]')).toHaveText('1 of 1 updated.')
})

test("the stack page's Update shows the same note, and Confirm still runs it", async ({ page, backend }) => {
  seed(backend)
  backend.updateRuns.unnamed.add('web/app')
  await page.goto('/stacks/web')
  await page.locator('[role="toolbar"][aria-label="Stack actions"] [data-action="update"]').click()

  await expect(stackPreview(page).locator('[data-rollback-note="app"]')).toHaveText(ROLLBACK_NOTE)
  await expect(stackPreview(page).locator('[data-findings]')).toHaveCount(0)
  await stackPreview(page).locator('[data-confirm]').click()
  await backend.waitForCmd('Confirm')
  await expect(stackPreview(page)).toHaveCount(0)
  await expect(page.locator('[data-operation="update"]')).toBeVisible()
})

test("the stack page's Update with nothing to install says everything is up to date", async ({ page, backend }) => {
  seed(backend)
  // Updated elsewhere since the page last heard: the server has nothing left to recreate.
  const view = backend.updates
  if (view === null) throw new Error('no updates view')
  view.services = view.services.map((s) => (s.stack === 'web' ? { ...s, state: 'up_to_date', available: null } : s))
  await page.goto('/stacks/web')
  await page.locator('[role="toolbar"][aria-label="Stack actions"] [data-action="update"]').click()

  await expect(stackPreview(page).locator('[data-up-to-date]')).toHaveText('Everything is up to date.')
  await expect(stackPreview(page).locator('[data-plan-skipped="web/app"]')).toHaveText('app: up to date')
  await expect(stackPreview(page).locator('[data-plan-skipped="web/cache"]')).toHaveText('cache: up to date')
  await expect(stackPreview(page).locator('[role="alert"]')).toHaveCount(0)
  await expect(stackPreview(page).locator('[data-confirm]')).toHaveCount(0)
  await stackPreview(page).getByRole('button', { name: 'Close' }).first().click()
  await expect(stackPreview(page)).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})
