/**
 * Spec 060 US3 (T036): System Settings → Tokens. Every enrolment token, filtered by state; a
 * pending one revoked, one rolled (the new token shows once); and an admin revoking someone
 * else's API token. Against the fake backend (tests/support/fake-settings.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../support/fake-backend'
import { governance } from '../support/fake-governance'
import { FakeHosts } from '../support/fake-hosts'
import { adminToken, enrollment, FakeSettings } from '../support/fake-settings'

/** What each `type` command sent so far asked for. */
function asked(backend: FakeBackend, type: 'ListEnrollments' | 'ListAllApiTokens'): unknown[] {
  return backend.received.flatMap((c) => (c.type === type ? [c.data] : []))
}

function signIn(backend: FakeBackend, instance_role: 'owner' | 'admin'): FakeSettings {
  backend.me = fixtures.me({ instance_role, grants: [], verification: fixtures.verification({ verified: true }) })
  governance(backend, (g) => {
    g.workspace('ops')
  })
  new FakeHosts(backend)
  const settings = new FakeSettings(backend)
  settings.enrollments = [
    enrollment({ id: 'enroll-web1', inventory_name: 'web-1', state: 'pending' }),
    enrollment({ id: 'enroll-db1', inventory_name: 'db-1', state: 'used', host: 'host-db1' }),
    enrollment({ id: 'enroll-old', inventory_name: 'old-1', state: 'expired' }),
  ]
  return settings
}

test('enrolment tokens are filtered, revoked and rolled', async ({ page, backend }) => {
  const settings = signIn(backend, 'owner')
  await page.goto('/settings/tokens')

  const pane = page.locator('section[data-pane="enrollment-tokens"]')
  const rows = pane.locator('[data-enrollment]')
  await expect(rows).toHaveCount(3)
  const used = pane.locator('[data-enrollment="enroll-db1"]')
  await expect(used.locator('[data-state]')).toHaveText('used')
  // A used token can be neither rolled nor revoked; an expired one only rolled.
  await expect(used.getByRole('button')).toHaveCount(0)
  await expect(pane.getByRole('button', { name: 'Revoke the token for old-1' })).toHaveCount(0)
  await expect(pane.getByRole('button', { name: 'Roll the token for old-1' })).toBeVisible()

  // Filter by state: the server is asked for that state only.
  await pane.getByLabel('State').selectOption('pending')
  await expect.poll(() => asked(backend, 'ListEnrollments')).toContainEqual({ state: 'pending', workspace: null })
  await expect(rows).toHaveCount(1)
  await expect(rows).toHaveAttribute('data-enrollment', 'enroll-web1')

  // Revoke the pending one: previewed with step-up, confirmed, then the filtered list is re-read.
  await pane.getByRole('button', { name: 'Revoke the token for web-1' }).click()
  expect((await backend.waitForCmd('PreviewEnrollmentRevoke')).data).toEqual({ enrollment: 'enroll-web1' })
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-step-up]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(pane).toContainText('No pending enrolment tokens.')
  expect(settings.enrollments.find((e) => e.id === 'enroll-web1')?.state).toBe('revoked')

  // Roll the expired one: the Enroll dialog's result step shows the new token, once.
  await pane.getByLabel('State').selectOption('')
  await pane.getByRole('button', { name: 'Roll the token for old-1' }).click()
  expect((await backend.waitForCmd('PreviewEnrollmentRoll')).data).toEqual({ enrollment: 'enroll-old' })
  await expect(dialog).toContainText('Roll enrolment token')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  const token = dialog.getByRole('region', { name: 'Enrolment token' })
  await expect(token).toContainText('Keel will not show it again')
  const minted = settings.minted[0] ?? 'missing'
  await token.getByRole('button', { name: 'Reveal token' }).click()
  await expect(token.locator('[data-copy="enroll-token"]')).toHaveText(minted)
  await expect(token.locator('[data-copy="enroll-snippet"]')).toContainText(minted)

  await dialog.getByRole('button', { name: 'Done' }).click()
  await dialog.getByRole('button', { name: 'Close anyway' }).click()
  await expect(dialog).toHaveCount(0)
  // The list shows the replacement as rolled, the old one revoked; the token is gone for good.
  await expect(pane.locator('[data-enrollment-state="pending"]')).toContainText('old-1 (rolled)')
  await expect(pane.locator('[data-enrollment="enroll-old"]')).toHaveAttribute('data-enrollment-state', 'revoked')
  await expect(page.getByText(minted)).toHaveCount(0)
})

test('revoking a token that is no longer pending is refused', async ({ page, backend }) => {
  const settings = signIn(backend, 'owner')
  await page.goto('/settings/tokens')
  const pane = page.locator('section[data-pane="enrollment-tokens"]')
  await expect(pane.locator('[data-enrollment]')).toHaveCount(3)

  // Someone used it after the list was loaded.
  settings.enrollments = settings.enrollments.map((e) => (e.id === 'enroll-web1' ? { ...e, state: 'used' as const } : e))
  await pane.getByRole('button', { name: 'Revoke the token for web-1' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Only a pending token can be revoked.')
  await expect(dialog.getByRole('button', { name: 'Confirm' })).toHaveCount(0)
})

test('an admin revokes another person’s API token', async ({ page, backend }) => {
  const settings = signIn(backend, 'admin')
  settings.apiTokens = [
    adminToken({ id: 'tok-grace', name: 'deploy-bot', holder_name: 'Grace Hopper' }),
    adminToken({ id: 'tok-linus', name: 'old-ci', holder_name: 'Linus', state: 'revoked' }),
  ]
  await page.goto('/settings/tokens')

  const pane = page.locator('section[data-pane="api-tokens"]')
  const grace = pane.locator('[data-api-token="tok-grace"]')
  await expect(grace).toContainText('Grace Hopper')
  await expect(grace).toContainText('never')
  await expect(pane.locator('[data-api-token="tok-linus"]').getByRole('button')).toHaveCount(0)

  await pane.getByLabel('State').selectOption('active')
  await expect.poll(() => asked(backend, 'ListAllApiTokens')).toContainEqual({ holder: null, state: 'active' })
  await expect(pane.locator('[data-api-token]')).toHaveCount(1)

  await grace.getByRole('button', { name: 'Revoke API token deploy-bot' }).click()
  expect((await backend.waitForCmd('PreviewApiTokenRevoke')).data).toEqual({ token: 'tok-grace' })
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Grace Hopper')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(pane).toContainText('No active API tokens.')
  expect(settings.apiTokens.find((t) => t.token.id === 'tok-grace')?.state).toBe('revoked')
})
