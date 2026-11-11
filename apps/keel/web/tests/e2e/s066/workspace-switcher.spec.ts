/**
 * Spec 066 T035 (R1, contracts/ui.md "Shell"): the workspace switcher in the top bar. The current
 * workspace is `?ws=`, then the stored choice, then the first in slug order; choosing one replaces
 * the address and every workspace-scoped page loads for it. Owners and admins create a workspace
 * from the switcher, and it becomes current once confirmed. Against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance, type FakeGovernance } from '../../support/fake-governance'

function owner(backend: FakeBackend): FakeGovernance {
  backend.me = fixtures.me({ instance_role: 'owner', verification: fixtures.verification({ verified: true }) })
  return governance(backend, (g) => {
    g.workspace('payments', 'Payments')
    g.workspace('ops', 'Ops')
  })
}

const switcher = (page: Page) => page.getByTestId('workspace-switcher')

test('the first workspace in slug order is current, and choosing another replaces the address', async ({ page, backend }) => {
  const g = owner(backend)
  const [payments, ops] = g.workspaces
  await page.goto('/bindings')
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'ops')
  await expect(page).toHaveURL(/\/bindings\?ws=ops$/)
  expect((await backend.waitForCmd('ListBindings')).data.workspace).toBe(ops?.id)

  await switcher(page).click()
  await expect(page.locator('[data-workspace-option]')).toHaveCount(2)
  await page.locator('[data-workspace-option="payments"]').click()
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'payments')
  await expect(page).toHaveURL(/\/bindings\?ws=payments$/)
  expect((await backend.waitForCmd('ListBindings')).data.workspace).toBe(payments?.id)

  // Back leaves the page rather than stepping through workspaces.
  const length = await page.evaluate(() => history.length)
  expect(length).toBeLessThanOrEqual(2)
})

test('a reload, or a link with no ws, stays on the stored workspace; a link with ws wins', async ({ page, backend }) => {
  owner(backend)
  await page.goto('/runs?ws=payments')
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'payments')
  await page.goto('/hosts')
  await expect(page).toHaveURL(/\/hosts\?ws=payments$/)
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'payments')
  await page.goto('/hosts?ws=ops')
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'ops')
})

test('an owner creates a workspace from the switcher and lands in it', async ({ page, backend }) => {
  const g = owner(backend)
  await page.goto('/runs')
  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'ops')
  await switcher(page).click()
  await page.locator('[data-action="create-workspace"]').click()

  const form = page.getByRole('form', { name: 'Create workspace' })
  await form.getByLabel('Name').fill('Payments Prod')
  await expect(form.getByLabel('Slug')).toHaveValue('payments-prod')
  await page.getByRole('button', { name: 'Preview' }).click()
  expect((await backend.waitForCmd('PreviewWorkspaceCreate')).data).toEqual({ slug: 'payments-prod', name: 'Payments Prod' })

  const dialog = page.getByRole('dialog', { name: 'New workspace' })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toHaveAttribute('data-change-dialog', 'done')
  await dialog.getByRole('button', { name: 'Done' }).click()

  await expect(switcher(page)).toHaveAttribute('data-current-workspace', 'payments-prod')
  await expect(page).toHaveURL(/\/runs\?ws=payments-prod$/)
  expect(g.workspaces.map((w) => w.slug)).toContain('payments-prod')
})

test('a slug the server would refuse cannot be previewed', async ({ page, backend }) => {
  owner(backend)
  await page.goto('/runs')
  await switcher(page).click()
  await page.locator('[data-action="create-workspace"]').click()
  const form = page.getByRole('form', { name: 'Create workspace' })
  await form.getByLabel('Name').fill('Ops')
  await form.getByLabel('Slug').fill('-Ops')
  await expect(form.getByLabel('Slug')).toHaveAttribute('aria-invalid', 'true')
  await expect(page.getByRole('button', { name: 'Preview' })).toBeDisabled()
})

test('a user with no workspace is told who grants one, and cannot create one', async ({ page, backend }) => {
  backend.me = fixtures.me({ instance_role: 'user' })
  governance(backend)
  await page.goto('/hosts')
  await expect(page.locator('[data-empty-page="workspace-picker"]')).toContainText('No workspace yet')
  await switcher(page).click()
  await expect(page.locator('[data-action="create-workspace"]')).toHaveCount(0)
})
