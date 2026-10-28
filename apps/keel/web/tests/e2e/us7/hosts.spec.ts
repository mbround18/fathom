/**
 * US7 (spec 055 T079): enrol a host through the base profile. The token is shown once with a
 * copyable `-e keel_enroll_token=…`; the host comes online, then is revoked. Against the fake
 * backend (tests/support/fake-hosts.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'
import { CERT_AUTHORITY, FakeHosts } from '../../support/fake-hosts'

function admin(backend: FakeBackend): FakeHosts {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  return new FakeHosts(backend)
}

test('a host is enrolled with a one-time token, comes online and is revoked', async ({ page, context, backend }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const hosts = admin(backend)

  await page.goto('/hosts')
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  const table = page.getByRole('table', { name: 'Hosts' })
  await expect(table).toContainText('No hosts yet')

  // Enrol: previewed, confirmed, then the token once.
  const form = page.getByRole('form', { name: 'Enrol a host' })
  await form.getByLabel('Inventory name').fill('web-1')
  await form.getByLabel('Profiles').fill('base, web')
  await form.getByRole('button', { name: 'Enrol' }).click()
  expect((await backend.waitForCmd('PreviewHostEnroll')).data).toEqual({
    workspace: WORKSPACE,
    inventory_name: 'web-1',
    profiles: ['base', 'web'],
  })
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-preview-summary]')).toContainText('web-1')
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const token = page.getByRole('region', { name: 'Enrolment token' })
  await expect(token).toContainText('Keel will not show it again')
  await expect(token).toContainText('KEEL_RUNNER_SERVER')
  const arg = token.locator('[data-copy="enroll-arg"]')
  await expect(arg).toHaveText(`-e keel_enroll_token=${hosts.tokens[0]?.token ?? 'missing'}`)
  await expect(arg).toContainText('-e keel_enroll_token=keel-enroll-v1.')

  await token.getByRole('button', { name: 'Copy extra var' }).click()
  await expect(token.getByRole('button', { name: 'Copy extra var' })).toHaveText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(await arg.textContent())

  // Done hides it, and nothing brings it back.
  await token.getByRole('button', { name: 'Done' }).click()
  await expect(token).toHaveCount(0)
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  await expect(page.getByRole('region', { name: 'Enrolment token' })).toHaveCount(0)
  await expect(page.locator('[data-copy="enroll-arg"]')).toHaveCount(0)

  // The SSH CA line runners pin.
  await page.getByText('SSH certificate authority').click()
  await expect(page.locator('[data-copy="cert-authority"]')).toHaveText(CERT_AUTHORITY)

  // The runner enrols with the token and connects.
  const web1 = hosts.connect('web-1')
  const row = table.locator('tr', { hasText: 'web-1' })
  await expect(row).toHaveAttribute('data-host-state', 'online')
  await expect(row).toContainText('base, web')
  await expect(row).toContainText('2.17.4')

  // Revoke: previewed with step-up (the session just verified), then the host is revoked.
  await row.getByRole('button', { name: 'Revoke web-1' }).click()
  expect((await backend.waitForCmd('PreviewHostRevoke')).data).toEqual({ host: web1.id })
  await expect(dialog).toContainText('asks for your passkey')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(row).toHaveAttribute('data-host-state', 'revoked')
  await expect(row.getByRole('button', { name: 'Revoke web-1' })).toHaveCount(0)
})

test('an unknown profile or a name already enrolled is refused', async ({ page, backend }) => {
  const hosts = admin(backend)
  await page.goto('/hosts')
  const form = page.getByRole('form', { name: 'Enrol a host' })

  await form.getByLabel('Inventory name').fill('db-1')
  await form.getByLabel('Profiles').fill('base, db')
  await form.getByRole('button', { name: 'Enrol' }).click()
  await expect(page.getByRole('alert')).toContainText('no profile db')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  expect(hosts.tokens).toHaveLength(0)

  hosts.connect('web-1')
  await expect(page.getByRole('table', { name: 'Hosts' }).locator('tr[data-host-state="online"]')).toHaveCount(1)
  await form.getByLabel('Inventory name').fill('web-1')
  await form.getByLabel('Profiles').fill('base')
  await form.getByRole('button', { name: 'Enrol' }).click()
  await expect(page.getByRole('alert')).toContainText('already enrolled')
  expect(hosts.tokens).toHaveLength(0)
})

test('cancelling the enrol preview mints no token', async ({ page, backend }) => {
  const hosts = admin(backend)
  await page.goto('/hosts')
  const form = page.getByRole('form', { name: 'Enrol a host' })
  await form.getByLabel('Inventory name').fill('web-2')
  await form.getByRole('button', { name: 'Enrol' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'Enrolment token' })).toHaveCount(0)
  expect(hosts.tokens).toHaveLength(0)
})
