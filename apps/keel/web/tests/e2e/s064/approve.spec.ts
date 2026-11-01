/**
 * Spec 064 US1 (T013): an approver approves a held run from its page. The panel names what the
 * policy needs; Approve previews what the approval does, then confirms it (with a passkey when the
 * session has not just verified). One approval of one queues the run; one of two leaves it waiting
 * and the panel says the person approved. A refusal shows in the dialog in the server's words.
 */
import { expect, fixtures, test } from '../../support/fake-backend'
import { HEAD } from '../../support/fake-repos'
import { approvalEntry, hold } from '../../support/fake-runs'
import { virtualAuthenticator } from '../../support/webauthn'
import { registerPasskey } from '../s061/support'

import { decision, freezeClock, openRun, signIn } from './support'

test.beforeEach(async ({ page }) => {
  await freezeClock(page)
})

test('the last approval a run needs queues it', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0001', job_name: 'web' }))

  const panel = await openRun(page, 'run-0001')
  await expect(panel).toHaveAttribute('data-approval-panel', 'act')
  await expect(panel.locator('[data-approval-policy]')).toHaveText('Needs 1 approval')
  await expect(panel.locator('[data-approval-count]')).toHaveText('0 of 1 approval')
  await expect(panel.locator('[data-approval-window]')).toContainText('Window closes tomorrow (2026-10-28 12:00 UTC)')

  await panel.locator('[data-approve]').click()
  const dialog = decision(page, 'approve')
  await expect(dialog.locator('[data-preview-summary]')).toHaveText(
    `Approve run of job web at ${HEAD.slice(0, 12)} on 2 host(s) from inventory prod: this queues the run.`,
  )
  await expect(dialog.locator('[data-affected]')).toContainText('web-1')
  await expect(dialog.locator('[data-step-up]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  await expect(dialog).toBeHidden()
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'queued')
  await expect(panel).toHaveAttribute('data-approval-panel', 'closed')
  await expect(panel.locator('[data-approval-count]')).toHaveText('1 of 1 approval')
  await expect(panel.getByRole('list', { name: 'Approvals' })).toContainText('Ada Lovelace (approver)')
  await expect(panel.locator('[data-approve]')).toHaveCount(0)
  // The session had just verified: no passkey was asked for.
  const sent = backend.received.filter((c) => c.type === 'Confirm').at(-1)
  expect(sent?.type === 'Confirm' && sent.data.step_up).toBeFalsy()
})

test('one approval of two leaves the run waiting, and says so', async ({ page, backend }) => {
  const runs = signIn(backend, 'ws_admin')
  runs.add(hold({ id: 'run-0002' }, { needed: 2, four_eyes: true }))

  const panel = await openRun(page, 'run-0002')
  await expect(panel.locator('[data-approval-policy]')).toHaveText('Needs 2 approvals · four-eyes')
  await panel.locator('[data-approve]').click()
  const dialog = decision(page, 'approve')
  await expect(dialog.locator('[data-preview-summary]')).toContainText(': this adds approval 1 of 2.')
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  await expect(dialog).toBeHidden()
  await expect(panel).toHaveAttribute('data-approval-panel', 'answered')
  await expect(panel.locator('[data-approval-note]')).toHaveText('You approved this run.')
  await expect(panel.locator('[data-approval-count]')).toHaveText('1 of 2 approvals')
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'pending_approval')
  await expect(panel.locator('[data-approve]')).toHaveCount(0)
})

test('confirming asks for a passkey when the check has lapsed; closing it records nothing', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  const runs = signIn(backend, 'approver', { verification: fixtures.verification() })
  runs.add(hold({ id: 'run-0003' }))
  await page.goto('/runs')
  await registerPasskey(page)
  const verification = fixtures.verification({ enrolled: true, verified: false })
  backend.me = { ...(backend.me ?? fixtures.me()), verification }
  backend.push({ type: 'Verification', data: verification })

  const panel = await openRun(page, 'run-0003')
  await panel.locator('[data-approve]').click()
  const dialog = decision(page, 'approve')
  await expect(dialog.locator('[data-preview-summary]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  // Closing the passkey check sends nothing: the run keeps waiting and the dialog stays.
  const stepUp = page.getByRole('dialog', { name: 'Verify it’s you' })
  await expect(stepUp).toBeVisible()
  await stepUp.getByRole('button', { name: 'Cancel' }).click()
  await expect(stepUp).toBeHidden()
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'pending_approval')

  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await stepUp.getByRole('button', { name: 'Use passkey' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'queued')
  const sent = backend.received.filter((c) => c.type === 'Confirm').at(-1)
  expect(sent?.type === 'Confirm' && sent.data.step_up).toBeTruthy()
})

test('a run answered meanwhile is refused in the server’s words, and the page catches up', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0004' }))

  const panel = await openRun(page, 'run-0004')
  await panel.locator('[data-approve]').click()
  const dialog = decision(page, 'approve')
  await expect(dialog.locator('[data-preview-summary]')).toBeVisible()
  // Someone else approves it first, from another browser.
  runs.runs = runs.runs.map((r) =>
    r.id === 'run-0004' && r.approval
      ? { ...r, state: 'queued' as const, approval: { ...r.approval, approvals: [approvalEntry()] } }
      : r,
  )
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  await expect(dialog.locator('[data-refusal="error"]')).toContainText('This run has already been answered; it is now queued.')
  await expect(dialog.locator('[data-error-code]')).toHaveText('run_answered')
  await expect(page.locator('[data-run-state]')).toHaveAttribute('data-run-state', 'queued')
  await expect(panel).toHaveAttribute('data-approval-panel', 'closed')
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(dialog).toBeHidden()
  await expect(panel.getByRole('list', { name: 'Approvals' })).toContainText('Bob Builder (approver)')
})

test('a viewer sees the panel without buttons', async ({ page, backend }) => {
  const runs = signIn(backend, 'viewer')
  runs.add(hold({ id: 'run-0005' }))
  const panel = await openRun(page, 'run-0005')
  await expect(panel).toHaveAttribute('data-approval-panel', 'restricted')
  await expect(panel.locator('[data-approval-note]')).toHaveText('An approver or workspace admin can approve this.')
  await expect(panel.getByRole('button', { name: 'Approve' })).toHaveCount(0)
})
