/**
 * Spec 061 US4 (T066): Update Keel. The owner previews the update, confirms with a passkey, and
 * the card follows the run, whose page shows the digest it moves to. An older build warns in the
 * preview; a run already in progress is opened rather than started again; a failed run says so
 * with a link and offers to try again. Admins and users never see Update.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { FakeRuns } from '../../support/fake-runs'
import { AVAILABLE_DIGEST, updateFixtures } from '../../support/fake-updates'
import { virtualAuthenticator } from '../../support/webauthn'

import { openSystem, registerPasskey, signIn } from './support'

const card = (page: Page) => page.getByRole('region', { name: 'Updates' })

/** The run `Confirm` queued, once the fake has it, added to the runs model so its page opens. */
async function queuedRun(backend: FakeBackend, runs: FakeRuns): Promise<string> {
  await backend.waitForCmd('Confirm')
  await expect.poll(() => backend.updates.status.update_run).not.toBeNull()
  const id = backend.updates.status.update_run ?? ''
  runs.add(updateFixtures.run(id, AVAILABLE_DIGEST))
  return id
}

test('an owner previews the update, confirms with a passkey and follows the run', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  const runs = new FakeRuns(backend)
  await page.goto('/runs')
  await registerPasskey(page)
  // The session's verification has lapsed: confirming asks for the passkey again.
  const verification = fixtures.verification({ enrolled: true, verified: false })
  backend.me = { ...(backend.me ?? fixtures.me()), verification }
  backend.push({ type: 'Verification', data: verification })

  await openSystem(page)
  await card(page).getByRole('button', { name: 'Update Keel' }).click()
  const dialog = page.getByRole('dialog', { name: 'Update Keel' })
  await expect(dialog).toContainText('Update Keel from 0.2.0 (abc1234')
  await expect(dialog).toContainText('to 0.3.0 (fedcba9')
  await expect(dialog).toContainText('keel-server on keel-1')
  await expect(dialog).toContainText('Keel restarts.')
  await expect(dialog).not.toContainText('Warning')
  const preview = await backend.waitForCmd('PreviewKeelUpdate')
  expect(preview.data.digest).toBe(AVAILABLE_DIGEST)
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const stepUp = page.getByRole('dialog', { name: 'Verify it’s you' })
  await stepUp.getByRole('button', { name: 'Use passkey' }).click()
  const id = await queuedRun(backend, runs)
  const sent = backend.received.filter((c) => c.type === 'Confirm').at(-1)
  expect(sent?.type === 'Confirm' && sent.data.step_up).toBeTruthy()
  await page.getByRole('button', { name: 'Done' }).click()

  // The card names the run in progress instead of offering Update again.
  await expect(card(page).locator('[data-update-run="in_progress"]')).toContainText('An update run is in progress.')
  await expect(card(page).getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
  await card(page).getByRole('link', { name: 'Open the update run' }).click()
  await expect(page).toHaveURL(new RegExp(`/runs/${id}$`))
  await expect(page.locator('[data-param="keel_server_image_digest"]')).toHaveText(AVAILABLE_DIGEST)
})

test('updating to an older build warns first in the preview', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('older_build'))
  await openSystem(page)
  await card(page).getByRole('button', { name: 'Update Keel' }).click()
  const dialog = page.getByRole('dialog', { name: 'Update Keel' })
  await expect(dialog).toContainText('Warning: the watched image is an older build; this goes back.')
  await expect(dialog).toContainText('to 0.1.9')
})

test('a run already in progress is opened, not started again', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available', { update_run: 'run-update-9' }))
  await openSystem(page)
  await expect(card(page).locator('[data-update-run="in_progress"]')).toBeVisible()
  await expect(card(page).getByRole('link', { name: 'Open the update run' })).toHaveAttribute('href', '/runs/run-update-9')
  await expect(card(page).getByRole('button', { name: 'Update Keel' })).toHaveCount(0)
})

test('a failed update says so, links the run and offers to try again', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  const runs = new FakeRuns(backend)
  await openSystem(page)
  await card(page).getByRole('button', { name: 'Update Keel' }).click()
  await page.getByRole('dialog', { name: 'Update Keel' }).getByRole('button', { name: 'Confirm' }).click()
  const id = await queuedRun(backend, runs)
  await page.getByRole('button', { name: 'Done' }).click()
  await expect(card(page).locator('[data-update-run="in_progress"]')).toBeVisible()

  // The run fails; the status no longer names a run, and the card reads it once more.
  runs.finish(id, 'failed', null)
  backend.updates.finishCheck({ ...backend.updates.status, update_run: null })
  const failed = card(page).locator('[data-update-run="failed"]')
  await expect(failed).toContainText('The update failed; Keel is still running 0.2.0.')
  await expect(failed.getByRole('link', { name: 'Open the failed run' })).toHaveAttribute('href', `/runs/${id}`)
  await expect(card(page).getByRole('button', { name: 'Try the update again' })).toBeVisible()
})

test('a refused preview says why and changes nothing', async ({ page, backend }) => {
  signIn(backend, 'owner', updateFixtures.status('update_available'))
  backend.updates.refusal = 'digest_stale'
  await openSystem(page)
  await card(page).getByRole('button', { name: 'Update Keel' }).click()
  const dialog = page.getByRole('dialog', { name: 'Update Keel' })
  await expect(dialog).toContainText('Keel refused this change. Nothing was changed.')
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})

for (const role of ['admin', 'user'] as const) {
  test(`${role === 'admin' ? 'an' : 'a'} ${role} never sees Update`, async ({ page, backend }) => {
    signIn(backend, role, updateFixtures.status('update_available'))
    await page.goto('/settings/system')
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
    await expect(page.getByRole('button', { name: /Update Keel|Try the update again/ })).toHaveCount(0)
    expect(backend.received.some((c) => c.type === 'PreviewKeelUpdate')).toBe(false)
  })
}
