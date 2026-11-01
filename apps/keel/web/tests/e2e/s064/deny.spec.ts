/**
 * Spec 064 US2 (T017): an approver denies a held run, with an optional reason of up to 500
 * characters. The run ends as denied, reads "Approval denied", and everyone who opens it sees who
 * denied it and why. Whoever started a four-eyes run may deny it but not approve it.
 */
import { expect, test } from '../../support/fake-backend'
import { HEAD } from '../../support/fake-repos'
import { approvalEntry, hold } from '../../support/fake-runs'

import { decision, freezeClock, openRun, signIn } from './support'

test.beforeEach(async ({ page }) => {
  await freezeClock(page)
})

test('a denial with a reason ends the run, and the reason stays on it', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0001' }, { needed: 2, approvals: [approvalEntry()] }))

  const panel = await openRun(page, 'run-0001')
  await panel.locator('[data-deny]').click()
  const dialog = decision(page, 'deny')
  await expect(dialog.locator('[data-preview-summary]')).toHaveText(
    `Deny run of job web at ${HEAD.slice(0, 12)} on 2 host(s) from inventory prod: it will not run.`,
  )
  const reason = dialog.getByLabel('Reason (optional)')
  await expect(reason).toHaveAttribute('maxlength', '500')
  await reason.fill('Freeze until the incident review.')
  await expect(dialog.locator('[data-reason-count]')).toHaveText('33/500')
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  await expect(dialog).toBeHidden()
  // The preview the person read had no reason: the one confirmed carries it.
  const reasons = backend.received.flatMap((c) => (c.type === 'PreviewRunDeny' ? [c.data.reason] : []))
  expect(reasons).toEqual([null, 'Freeze until the incident review.'])
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'denied')
  await expect(page.getByText('Approval denied', { exact: true })).toBeVisible()
  await expect(panel).toHaveAttribute('data-approval-panel', 'closed')
  await expect(panel.locator('[data-approval-denial]')).toContainText('Denied by Ada Lovelace (approver)')
  await expect(panel.locator('[data-approval-reason]')).toHaveText('Freeze until the incident review.')
  await expect(panel.locator('[data-deny]')).toHaveCount(0)
})

test('a denial needs no reason', async ({ page, backend }) => {
  const runs = signIn(backend, 'ws_admin')
  runs.add(hold({ id: 'run-0002' }))
  const panel = await openRun(page, 'run-0002')
  await panel.locator('[data-deny]').click()
  const dialog = decision(page, 'deny')
  await expect(dialog.locator('[data-preview-summary]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
  expect(backend.received.filter((c) => c.type === 'PreviewRunDeny')).toHaveLength(1)
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'denied')
  await expect(panel.locator('[data-approval-denial]')).toContainText('Denied by Ada Lovelace (ws admin)')
  await expect(panel.locator('[data-approval-reason]')).toHaveCount(0)
})

test('the four-eyes starter may only deny', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  const me = backend.me?.id ?? null
  runs.add(hold({ id: 'run-0003', requested_by: me, trigger: 'manual' }, { four_eyes: true }))
  const panel = await openRun(page, 'run-0003')
  await expect(panel).toHaveAttribute('data-approval-panel', 'requester')
  await expect(panel.locator('[data-approval-note]')).toHaveText('Someone else must approve this run.')
  await expect(panel.locator('[data-approve]')).toHaveCount(0)
  await panel.locator('[data-deny]').click()
  await decision(page, 'deny').getByRole('button', { name: 'Confirm' }).click()
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'denied')
})
