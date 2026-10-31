/**
 * US7 (spec 055 T079, reworked for spec 060 US1): enrol a host through the Enroll dialog. The token
 * is shown once, masked until revealed, with the snippet to run; the host comes online, then is
 * revoked. Against the fake backend (tests/support/fake-hosts.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'
import { CERT_AUTHORITY, FakeHosts } from '../../support/fake-hosts'

function admin(backend: FakeBackend): FakeHosts {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  backend.on('ListWorkspaces', () => [
    {
      type: 'Workspaces',
      data: {
        items: [
          { id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false },
        ],
      },
    },
  ])
  return new FakeHosts(backend)
}

test('a host is enrolled with a one-time token, comes online and is revoked', async ({ page, context, backend }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const hosts = admin(backend)
  hosts.instructions = 'Open **port 7443** on the firewall first.'

  await page.goto('/hosts')
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  const table = page.getByRole('table', { name: 'Hosts' })
  await expect(page.locator('[data-empty-page="hosts"]')).toContainText('Enroll host')

  // Enroll: the form, previewed in the same dialog, confirmed, then the token once.
  await page.locator('[data-enroll-open]').click()
  const dialog = page.getByRole('dialog')
  const form = dialog.getByRole('form', { name: 'Enroll host' })
  await expect(form.getByLabel('Workspace')).toHaveValue(WORKSPACE)
  await form.getByLabel('Inventory name').fill('web 1')
  await expect(form).toContainText('Letters, digits')
  await form.getByLabel('Inventory name').fill('web-1')
  await form.getByLabel('Profiles').fill('web')
  await form.getByLabel('Profiles').press('Enter')
  await expect(form.locator('[data-profile-chip]')).toHaveCount(2)
  await dialog.getByRole('button', { name: 'Continue' }).click()
  expect((await backend.waitForCmd('PreviewHostEnroll')).data).toEqual({
    workspace: WORKSPACE,
    inventory_name: 'web-1',
    profiles: ['base', 'web'],
  })
  await expect(dialog.locator('[data-preview-summary]')).toContainText('web-1')
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const token = dialog.getByRole('region', { name: 'Enrolment token' })
  await expect(token).toContainText('Keel will not show it again')
  const minted = hosts.tokens[0]?.token ?? 'missing'
  const value = token.locator('[data-copy="enroll-token"]')
  await expect(value).toHaveAttribute('data-masked', 'true')
  await expect(value).not.toContainText('keel-enroll-v1.')
  await token.getByRole('button', { name: 'Reveal token' }).click()
  await expect(value).toHaveText(minted)
  await expect(token.locator('[data-copy="enroll-snippet"]')).toContainText(minted)
  await expect(token.locator('[data-expires]')).toContainText('Valid for')
  await expect(token.getByRole('region', { name: 'Instructions' }).locator('strong')).toHaveText('port 7443')

  await token.getByRole('button', { name: 'Copy token' }).click()
  await expect(token.getByRole('button', { name: 'Copy token' })).toHaveText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(minted)

  // Done warns first, then closes; nothing brings the token back.
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(dialog.locator('[data-close-warning]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Close anyway' }).click()
  await expect(dialog).toHaveCount(0)
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  await expect(page.locator('[data-copy="enroll-token"]')).toHaveCount(0)

  // The SSH CA line runners pin.
  await page.getByText('SSH certificate authority').click()
  await expect(page.locator('[data-copy="cert-authority"]')).toHaveText(CERT_AUTHORITY)

  // The runner enrols with the token and connects.
  const web1 = hosts.connect('web-1')
  const row = table.locator('tr', { hasText: 'web-1' })
  await expect(row).toHaveAttribute('data-host-state', 'online')
  await expect(row).toContainText('base, web')
  await expect(row).toContainText('2.21.4')

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
  const dialog = page.getByRole('dialog')
  const form = dialog.getByRole('form', { name: 'Enroll host' })

  await page.locator('[data-enroll-open]').click()
  await form.getByLabel('Inventory name').fill('db-1')
  await form.getByLabel('Profiles').fill('db')
  await form.getByLabel('Profiles').press('Enter')
  await dialog.getByRole('button', { name: 'Continue' }).click()
  await expect(dialog).toContainText('no profile db')
  expect(hosts.tokens).toHaveLength(0)

  // Back keeps what was typed out of the way: the form starts again.
  await dialog.getByRole('button', { name: 'Back' }).click()
  await expect(form).toBeVisible()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveCount(0)

  hosts.connect('web-1')
  await expect(page.getByRole('table', { name: 'Hosts' }).locator('tr[data-host-state="online"]')).toHaveCount(1)
  await page.locator('[data-enroll-open]').click()
  await form.getByLabel('Inventory name').fill('web-1')
  await dialog.getByRole('button', { name: 'Continue' }).click()
  await expect(dialog).toContainText('already enrolled')
  expect(hosts.tokens).toHaveLength(0)
})

test('cancelling the enrol preview mints no token', async ({ page, backend }) => {
  const hosts = admin(backend)
  await page.goto('/hosts')
  await page.locator('[data-enroll-open]').click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Inventory name').fill('web-2')
  await dialog.getByRole('button', { name: 'Continue' }).click()
  await expect(dialog.locator('[data-preview-summary]')).toContainText('web-2')
  await dialog.getByRole('button', { name: 'Back' }).click()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.locator('[data-copy="enroll-token"]')).toHaveCount(0)
  expect(hosts.tokens).toHaveLength(0)
})

test('a user who cannot operate anywhere is not offered Enroll', async ({ page, backend }) => {
  admin(backend)
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'viewer', profiles: [] }],
  })
  await page.goto('/hosts')
  // An empty list says who can enroll instead (spec 063 FR-013).
  await expect(page.locator('[data-empty-page="hosts"]')).toContainText('No hosts yet')
  await expect(page.locator('[data-empty-page="hosts"]')).toHaveAttribute('data-empty', 'restricted')
  await expect(page.getByRole('button', { name: 'Enroll host' })).toHaveCount(0)
})
