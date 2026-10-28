/**
 * US8 (spec 055 T095): a signed-mode review lists its commits with their signature status, flags
 * `.keel/`, inventory and `keel_runner_*` changes, pages through the diff file by file, and is
 * approved with a passkey over the override's own challenge and a required comment. Against the
 * fake backend (tests/support/fake-reviews.ts), with Chromium's virtual authenticator.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeReviews, SIGNER_FP, TARGET } from '../../support/fake-reviews'
import { virtualAuthenticator } from '../../support/webauthn'

function reviewer(backend: FakeBackend): FakeReviews {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
  })
  return new FakeReviews(backend)
}

/** Registers a passkey on the virtual authenticator through the account menu. */
async function registerPasskey(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: 'Register a passkey' }).click()
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await enrol.getByLabel('Name').fill('Work laptop')
  await enrol.getByRole('button', { name: 'Register' }).click()
  await expect(page.getByRole('dialog', { name: 'Passkey registered' })).toBeVisible()
  await page.getByRole('button', { name: 'Done' }).click()
}

async function openReview(page: Page): Promise<void> {
  await page.goto('/reviews')
  await expect(page.getByRole('heading', { name: 'Reviews', level: 1 })).toBeVisible()
  const row = page.getByRole('table', { name: 'Reviews' }).locator('tr[data-review-state="open"]')
  await expect(row).toContainText('web-1, web-2')
  await row.getByRole('button', { name: /Open review of web/ }).click()
}

test('a review shows commits, flags and a paged diff, and is approved with a passkey and a comment', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  const reviews = reviewer(backend)
  await page.goto('/reviews')
  await registerPasskey(page)
  const passkey = backend.passkeys[0]?.id
  expect(passkey).toBeDefined()
  await openReview(page)

  const review = page.getByRole('region', { name: 'Review' })
  // Commits, each with its signature status.
  const commits = review.getByRole('region', { name: 'Commits' })
  await expect(commits.locator(`[data-commit="${TARGET}"]`)).toHaveAttribute('data-signature', 'missing')
  await expect(commits.locator(`[data-commit="${TARGET}"]`)).toContainText('Unsigned')
  await expect(commits.locator('[data-signature="good"]')).toContainText('Signed, trusted')
  await expect(commits.locator('[data-signature="good"]')).toContainText(SIGNER_FP)
  await expect(commits.locator('[data-signature="unknown_key"]')).toContainText('Unknown key')

  // Trust and targeting changes are flagged.
  const flags = review.getByRole('region', { name: 'Flagged changes' })
  await expect(flags.locator('[data-flag="keel_config"]')).toContainText('.keel/jobs.yaml')
  await expect(flags.locator('[data-flag="inventory"]')).toContainText('inventories/prod/group_vars/web.yml')
  await expect(flags.locator('[data-flag="runner_vars"]')).toContainText('keel_runner_*')
  await expect(flags.locator('[data-flag="runner_vars"]')).toContainText('playbooks/web.yaml')

  // The diff, file by file and page by page.
  const files = review.getByRole('list', { name: 'Changed files' })
  await expect(files.getByRole('listitem')).toHaveCount(4)
  await files.getByRole('button', { name: 'playbooks/web.yaml' }).click()
  const diff = review.getByRole('region', { name: 'Diff of playbooks/web.yaml' })
  await expect(diff.locator('[data-diff-page]')).toHaveText('Page 1 of 2')
  await expect(diff).toContainText('+    keel_runner_user: root')
  await expect(diff.getByRole('button', { name: 'Previous page' })).toBeDisabled()
  await diff.getByRole('button', { name: 'Next page' }).click()
  await expect(diff.locator('[data-diff-page]')).toHaveText('Page 2 of 2')
  await expect(diff).toContainText('task 30')
  await expect(diff).not.toContainText('keel_runner_user')
  await expect(diff.getByRole('button', { name: 'Next page' })).toBeDisabled()
  await files.getByRole('button', { name: '.keel/jobs.yaml' }).click()
  const jobs = review.getByRole('region', { name: 'Diff of .keel/jobs.yaml' })
  await expect(jobs.locator('[data-diff-page]')).toHaveText('Page 1 of 1')
  await expect(jobs).toContainText('paths: ["**"]')
  await files.getByRole('button', { name: 'roles/web/files/logo.png' }).click()
  await expect(review.getByRole('region', { name: 'Diff of roles/web/files/logo.png' })).toContainText('Binary file')

  // A comment is required.
  const decide = review.getByRole('form', { name: 'Decide' })
  const approve = decide.getByRole('button', { name: 'Approve with passkey' })
  await expect(approve).toBeDisabled()
  await decide.getByLabel('Comment (required)').fill('   ')
  await expect(approve).toBeDisabled()
  await decide.getByLabel('Comment (required)').fill('Checked the runner user change with the web team.')
  await approve.click()
  expect((await backend.waitForCmd('PreviewOverride')).data).toEqual({
    group: 'group-0001',
    comment: 'Checked the runner user change with the web team.',
  })

  // The dialog shows exactly what the passkey signs; signing approves.
  const dialog = page.getByRole('dialog', { name: 'Approve with passkey' })
  await expect(dialog.locator('[data-canonical]')).toContainText(TARGET)
  await expect(dialog.locator('[data-override-expires]')).toHaveText('2026-10-28T12:00:00Z')
  await dialog.getByRole('button', { name: 'Sign and approve' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(review.locator('[data-state]')).toHaveText('approved')
  await expect(review.getByRole('region', { name: 'Decisions' })).toContainText('Checked the runner user change')
  await expect(review.getByRole('form', { name: 'Decide' })).toHaveCount(0)
  expect(reviews.decisions).toEqual([
    { group: 'group-0001', approve: true, comment: 'Checked the runner user change with the web team.', credential: passkey },
  ])
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.step_up?.credential.response).toHaveProperty('signature')
})

test('approving without a passkey asks for one; denying needs only a comment', async ({ page, backend }) => {
  const reviews = reviewer(backend)
  await openReview(page)
  const review = page.getByRole('region', { name: 'Review' })
  const decide = review.getByRole('form', { name: 'Decide' })
  await decide.getByLabel('Comment (required)').fill('Not reviewed yet.')
  await decide.getByRole('button', { name: 'Approve with passkey' }).click()
  await expect(page.getByRole('alert')).toContainText('Register a passkey first')
  expect(reviews.decisions).toHaveLength(0)

  await decide.getByRole('button', { name: 'Deny' }).click()
  expect((await backend.waitForCmd('PreviewOverrideDeny')).data).toEqual({ group: 'group-0001', comment: 'Not reviewed yet.' })
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-preview-summary]')).toContainText('denied')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(review.locator('[data-state]')).toHaveText('denied')
  expect(reviews.decisions).toEqual([{ group: 'group-0001', approve: false, comment: 'Not reviewed yet.', credential: null }])
  // A decided group leaves the open list.
  await expect(page.getByRole('table', { name: 'Reviews' })).toContainText('Nothing to review')
})

test('trusted signers are added and removed with step-up', async ({ page, backend }) => {
  const reviews = reviewer(backend)
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  await page.goto('/reviews')
  const table = page.getByRole('table', { name: 'Trusted signers' })
  await expect(table).toContainText('No trusted signers')

  const form = page.getByRole('form', { name: 'Add a signer' })
  await form.getByLabel('Public key').fill('not a key')
  await form.getByLabel('Label').fill('ci')
  await form.getByRole('button', { name: 'Add signer' }).click()
  await expect(page.getByRole('alert')).toContainText('not an SSH public key')

  await form.getByLabel('Public key').fill('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICiSigner ci@example.org')
  await form.getByRole('button', { name: 'Add signer' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('asks for your passkey')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(table).toContainText(SIGNER_FP)
  expect(reviews.signers.map((s) => s.label)).toEqual(['ci'])

  await table.getByRole('button', { name: 'Remove signer ci' }).click()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(table).toContainText('No trusted signers')
  expect(reviews.signers).toHaveLength(0)
})
