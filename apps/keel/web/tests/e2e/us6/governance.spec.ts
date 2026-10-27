/**
 * US6: team and user governance (spec 055 T034), against the fake backend's governance
 * (tests/support/fake-governance.ts).
 *
 * The signed-in person verified a passkey recently, so step-up previews confirm without a new
 * ceremony (the ceremony itself is covered by shell/passkeys.spec.ts).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test } from '../../support/fake-backend'
import { governance, people, type FakeGovernance } from '../../support/fake-governance'

const verified = fixtures.verification({ verified: true, enrolled: true, at: '2026-10-27T11:59:00Z' })

test.beforeEach(({ backend }) => {
  backend.me = fixtures.me({ instance_role: 'owner', verification: verified })
})

async function confirmChange(page: Page, title: string): Promise<void> {
  const dialog = page.getByRole('dialog', { name: title })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toHaveAttribute('data-change-dialog', 'done')
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(dialog).toBeHidden()
}

test('creates a workspace and a team', async ({ page, backend }) => {
  const gov = governance(backend)
  await page.goto('/teams')
  await expect(page.locator('[data-pane="workspaces"] [data-pane-state="empty"]')).toBeVisible()

  await page.getByRole('button', { name: 'New workspace' }).click()
  const wsForm = page.getByRole('form', { name: 'New workspace' })
  await wsForm.getByLabel('Slug').fill('payments')
  await wsForm.getByLabel('Name').fill('Payments')
  await wsForm.getByRole('button', { name: 'Preview' }).click()
  const wsDialog = page.getByRole('dialog', { name: 'New workspace' })
  await expect(wsDialog).toContainText('Create workspace payments (Payments)')
  expect((await backend.waitForCmd('PreviewWorkspaceCreate')).data).toEqual({ slug: 'payments', name: 'Payments' })
  await confirmChange(page, 'New workspace')
  await expect(page.locator('[data-workspace="payments"]')).toContainText('Payments')
  expect(gov.workspaces.map((w) => w.slug)).toEqual(['payments'])

  await page.getByRole('button', { name: 'New team' }).click()
  const teamForm = page.getByRole('form', { name: 'New team' })
  await teamForm.getByLabel('Team name').fill('Platform')
  await teamForm.getByRole('button', { name: 'Preview' }).click()
  await expect(page.getByRole('dialog', { name: 'New team' })).toContainText('Create team Platform')
  await confirmChange(page, 'New team')
  await expect(page.locator('[data-team="Platform"]')).toContainText('0 members')
})

test('adds a member and a grant; the grant preview lists every affected user', async ({ page, backend }) => {
  let ws = ''
  let team = ''
  const gov = governance(backend, (g) => {
    ws = g.workspace('payments', 'Payments').id
    g.workspace('billing')
    team = g.team('Platform', [people.ada(), people.grace()], [
      { workspace: ws, workspace_slug: 'payments', role: 'viewer', profiles: ['web'] },
    ]).id
  })
  await page.goto('/teams')
  await page.locator('[data-team="Platform"]').getByRole('button', { name: /Platform/ }).click()

  const memberForm = page.getByRole('form', { name: 'Add a member to Platform' })
  await memberForm.getByLabel('User').selectOption({ label: 'Linus Torvalds (linus@example.com)' })
  await memberForm.getByRole('button', { name: 'Add member' }).click()
  expect((await backend.waitForCmd('PreviewTeamMemberAdd')).data).toEqual({ team, user: people.linus().id })
  await expect(page.getByRole('dialog', { name: 'Add member' }).locator('[data-affected]')).toHaveText('Linus Torvalds')
  await confirmChange(page, 'Add member')
  const members = page.getByRole('list', { name: 'Members of Platform' })
  await expect(members.getByRole('listitem')).toHaveText(['Ada Lovelace', 'Grace Hopper', 'Linus Torvalds'])

  const grantForm = page.getByRole('form', { name: 'Add a grant to Platform' })
  await grantForm.getByLabel('Workspace', { exact: true }).selectOption({ label: 'billing' })
  await grantForm.getByLabel('Role').selectOption('operator')
  // A known profile from the multi-select, and one typed in.
  await grantForm.getByRole('checkbox', { name: 'web' }).click()
  await grantForm.getByLabel('Another profile').fill('db')
  await grantForm.getByRole('button', { name: 'Add profile' }).click()
  await expect(grantForm.locator('[data-chosen-profiles]')).toContainText('web')
  await expect(grantForm.locator('[data-chosen-profiles]')).toContainText('db')
  await grantForm.getByRole('button', { name: 'Add grant' }).click()

  const add = await backend.waitForCmd('PreviewGrantAdd')
  expect(add.data).toEqual({ team, workspace: gov.workspaces[1]?.id, role: 'operator', profiles: ['web', 'db'] })
  const dialog = page.getByRole('dialog', { name: 'Add grant' })
  await expect(dialog.locator('[data-summary]')).toHaveText('Give Platform operator on billing (profiles web, db)')
  await expect(dialog.locator('[data-affected] li')).toHaveText(['Ada Lovelace', 'Grace Hopper', 'Linus Torvalds'])
  await expect(dialog.locator('[data-step-up]')).toHaveText('Needs your passkey.')
  await confirmChange(page, 'Add grant')
  expect((await backend.waitForCmd('Confirm')).data).not.toHaveProperty('step_up')
  await expect(page.locator('[data-grant="billing:operator"]')).toContainText('web, db')
})

test('a Denied refusal is shown verbatim and nothing changes', async ({ page, backend }) => {
  const gov: FakeGovernance = governance(backend, (g) => {
    g.workspace('payments')
    g.team('Platform', [people.grace()])
    g.deny.set('PreviewGrantAdd', 'no_grant')
  })
  await page.goto('/teams')
  await page.locator('[data-team="Platform"]').getByRole('button', { name: /Platform/ }).click()
  const grantForm = page.getByRole('form', { name: 'Add a grant to Platform' })
  await grantForm.getByLabel('Workspace', { exact: true }).selectOption({ label: 'payments' })
  await grantForm.getByRole('button', { name: 'Add grant' }).click()

  const dialog = page.getByRole('dialog', { name: 'Add grant' })
  const alert = dialog.getByRole('alert')
  await expect(alert).toHaveText('Denied: preview_grant_add (no_grant)')
  await expect(alert.locator('[data-denied-reason]')).toHaveText('no_grant')
  await expect(dialog.getByRole('button', { name: 'Confirm' })).toHaveCount(0)
  await dialog.getByRole('button', { name: 'Dismiss' }).click()
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
  expect(gov.teams[0]?.grants).toEqual([])
})

test('someone who may not list users sees the refusal in place of the list', async ({ page, backend }) => {
  backend.me = fixtures.me({ instance_role: 'user', verification: verified })
  governance(backend, (g) => g.deny.set('ListUsers', 'instance_admin_only'))
  await page.goto('/users')
  await expect(page.locator('[data-pane="users-list"]').getByRole('alert')).toHaveText(
    'Denied: list_users (instance_admin_only)',
  )
})

test('changes a role, and shows a new token secret once', async ({ page, backend }) => {
  const gov = governance(backend, (g) => g.workspace('payments'))
  await page.goto('/users')

  await page.getByLabel('Role for Grace Hopper').selectOption('admin')
  const role = page.getByRole('dialog', { name: 'Change role' })
  await expect(role.locator('[data-summary]')).toHaveText('Make Grace Hopper admin')
  await confirmChange(page, 'Change role')
  await expect(page.getByLabel('Role for Grace Hopper')).toHaveValue('admin')

  await page.getByRole('button', { name: 'New token' }).click()
  const form = page.getByRole('form', { name: 'New API token' })
  await form.getByLabel('Token name').fill('ci')
  await form.getByLabel('Workspace', { exact: true }).selectOption({ label: 'payments' })
  await form.getByLabel('Actions').fill('list_workspaces, list_teams')
  await form.getByRole('button', { name: 'Preview' }).click()
  expect((await backend.waitForCmd('PreviewApiTokenCreate')).data).toEqual({
    name: 'ci',
    workspace: gov.workspaces[0]?.id,
    actions: ['list_workspaces', 'list_teams'],
    expires_in_days: 30,
  })
  const dialog = page.getByRole('dialog', { name: 'New API token' })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  const secret = dialog.locator('[data-token-secret]')
  await expect(secret).toHaveText(/^kt_fake-secret-/)
  const value = await secret.textContent()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(page.locator('[data-token="ci"]')).toContainText('list_workspaces, list_teams')
  expect(await page.content()).not.toContain(value ?? 'kt_')
})

test('narrow screens keep the governance pages in one column', async ({ page, backend }) => {
  governance(backend, (g) => {
    const ws = g.workspace('payments', 'Payments with a long display name for wrapping')
    g.team('Platform', [people.ada(), people.grace()], [
      { workspace: ws.id, workspace_slug: 'payments', role: 'ws_admin', profiles: ['web', 'db', 'cache', 'queue'] },
    ])
  })
  await page.setViewportSize({ width: 375, height: 700 })
  for (const path of ['/teams', '/users']) {
    await page.goto(path)
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
    if (path === '/teams') {
      await page.locator('[data-team="Platform"]').getByRole('button', { name: /Platform/ }).click()
      await expect(page.getByRole('form', { name: 'Add a grant to Platform' })).toBeVisible()
    }
    const width = await page.evaluate(() => document.documentElement.scrollWidth)
    expect(width).toBeLessThanOrEqual(375)
  }
})
