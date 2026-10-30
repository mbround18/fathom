/**
 * Spec 060 US1 (T011): who is offered Enroll, and the dialog's one-time result. An instance owner
 * with no grants sees the button and every workspace; the token and snippet show once; closing
 * warns first; someone who cannot operate on any workspace sees no button. Against the fake
 * backend (tests/support/fake-hosts.ts, workspaces from tests/support/fake-governance.ts).
 */
import { expect, fixtures, test } from '../support/fake-backend'
import { governance } from '../support/fake-governance'
import { FakeHosts } from '../support/fake-hosts'

test('an owner with no grants enrolls into any workspace and sees the token once', async ({ page, backend }) => {
  backend.me = fixtures.me({ instance_role: 'owner', grants: [], verification: fixtures.verification({ verified: true }) })
  const g = governance(backend, (gov) => {
    gov.workspace('ops')
    gov.workspace('db')
  })
  const hosts = new FakeHosts(backend)

  await page.goto('/hosts')
  await page.getByRole('button', { name: 'Enroll host' }).click()
  const dialog = page.getByRole('dialog')
  const form = dialog.getByRole('form', { name: 'Enroll a host' })
  // Every workspace, though the owner holds no grant on any.
  await expect(form.getByLabel('Workspace').locator('option')).toHaveText(['ops', 'db'].map((s) => new RegExp(s)))
  const db = g.workspaces[1]?.id ?? 'missing'
  await form.getByLabel('Workspace').selectOption(db)
  await form.getByLabel('Inventory name').fill('db-1')
  await dialog.getByRole('button', { name: 'Continue' }).click()
  expect((await backend.waitForCmd('PreviewHostEnroll')).data).toEqual({ workspace: db, inventory_name: 'db-1', profiles: ['base'] })
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  // The token and the snippet, once.
  const token = dialog.getByRole('region', { name: 'Enrolment token' })
  await expect(token).toContainText('Keel will not show it again')
  const minted = hosts.tokens[0]?.token ?? 'missing'
  await token.getByRole('button', { name: 'Reveal token' }).click()
  await expect(token.locator('[data-copy="enroll-token"]')).toHaveText(minted)
  await expect(token.locator('[data-copy="enroll-snippet"]')).toContainText(minted)

  // Closing warns; Escape is a close too.
  await page.keyboard.press('Escape')
  await expect(dialog.locator('[data-close-warning]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Close anyway' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByText(minted)).toHaveCount(0)

  // Opening Enroll again starts a new form: nothing brings the token back.
  await page.getByRole('button', { name: 'Enroll host' }).click()
  await expect(dialog.getByRole('form', { name: 'Enroll a host' })).toBeVisible()
  await expect(dialog.locator('[data-copy="enroll-token"]')).toHaveCount(0)
})

test('someone who cannot operate on any workspace is not offered Enroll', async ({ page, backend }) => {
  const g = governance(backend, (gov) => {
    gov.workspace('ops')
  })
  const ops = g.workspaces[0]?.id ?? 'missing'
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: ops, workspace_slug: 'ops', role: 'viewer', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  new FakeHosts(backend)

  await page.goto('/hosts')
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  // An empty list says who can enroll instead (spec 063 FR-013).
  await expect(page.locator('[data-empty-page="hosts"]')).toHaveAttribute('data-empty', 'restricted')
  await expect(page.getByRole('button', { name: 'Enroll host' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Enroll a host' })).toHaveCount(0)
})
