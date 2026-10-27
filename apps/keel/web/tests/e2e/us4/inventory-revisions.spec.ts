/**
 * US4 (spec 055 T055): edit a managed inventory, see validation errors before saving, diff two
 * revisions, and restore an earlier one. Against the fake backend (tests/support/fake-inventories.ts).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'

const yaml = (port: string, extra = '') =>
  `web:\n  hosts:\n    web1:\n      http_port: ${port}\n${extra}  vars:\n    tier: front\n`

const V1 = yaml('80')
const V2 = yaml('8080')

function admin(backend: FakeBackend): void {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
  })
}

/** Replaces the editor's text, as a paste would. */
async function replaceText(page: Page, label: string, text: string): Promise<void> {
  const editor = page.getByRole('textbox', { name: label })
  await editor.click()
  await page.keyboard.press('ControlOrMeta+A')
  await page.keyboard.press('Delete')
  await page.keyboard.insertText(text)
}

async function openProd(page: Page, backend: FakeBackend) {
  admin(backend)
  backend.inventories.seed('prod', [V1, V2])
  await page.goto('/inventory')
  await expect(page.getByRole('heading', { name: 'Inventory', level: 1 })).toBeVisible()
  await page.getByRole('table', { name: 'Inventories' }).getByRole('button', { name: 'prod' }).click()
  const detail = page.getByRole('region', { name: 'Inventory prod' })
  await expect(detail.getByRole('heading', { name: 'Revision 2 (newest)' })).toBeVisible()
  return detail
}

test('an edit is checked as it is typed, and errors block saving', async ({ page, backend }) => {
  const detail = await openProd(page, backend)
  const save = detail.getByRole('button', { name: 'Save revision' })
  await expect(page.getByRole('textbox', { name: 'Inventory YAML' })).toContainText('http_port: 8080')
  await expect(save).toBeDisabled()

  await replaceText(page, 'Inventory YAML', yaml('eighty'))
  const problems = detail.getByRole('list', { name: 'Problems' })
  await expect(problems).toContainText('hosts.web1.http_port')
  await expect(problems).toContainText('expected integer')
  await expect(detail.getByText('Fix the errors below before saving.')).toBeVisible()
  await expect(save).toBeDisabled()
  expect(backend.received.some((c) => c.type === 'PreviewInventoryEdit')).toBe(false)
  expect(backend.inventories.revisions('prod')).toHaveLength(2)

  // Fixed, and a host added: the check passes and the save previews before anything is stored.
  await replaceText(page, 'Inventory YAML', yaml('9090', '    web2:\n      http_port: 9090\n'))
  await expect(detail.getByText('Checked: no errors.')).toBeVisible()
  await expect(problems).toBeHidden()
  await save.click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Save revision 3 of "prod" with 2 host(s).')
  expect(backend.inventories.revisions('prod')).toHaveLength(2)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()

  await expect(detail.getByRole('heading', { name: 'Revision 3 (newest)' })).toBeVisible()
  await expect(detail.getByRole('table', { name: 'Revisions' }).getByRole('row')).toHaveCount(4)
  await expect(detail.getByRole('table', { name: 'Hosts' })).toContainText('web2')
  await expect(detail.getByText('No unsaved changes.')).toBeVisible()
  expect(backend.inventories.revisions('prod')).toHaveLength(3)
})

test('two revisions diff, and restoring an earlier one makes a new revision', async ({ page, backend }) => {
  const detail = await openProd(page, backend)

  // Diff: 1 against 2.
  const compare = detail.getByRole('form', { name: 'Compare revisions' })
  await compare.getByLabel('From').selectOption('1')
  await compare.getByLabel('To').selectOption('2')
  await compare.getByRole('button', { name: 'Compare' }).click()
  const changes = detail.getByRole('table', { name: 'Changes from 1 to 2' })
  const row = changes.getByRole('row').filter({ hasText: 'web.hosts.web1.http_port' })
  await expect(row).toContainText('changed')
  await expect(row).toContainText('80')
  await expect(row).toContainText('8080')
  await expect(changes.getByRole('row')).toHaveCount(2)

  // An earlier revision reads only.
  await detail.getByRole('button', { name: 'View revision 1' }).click()
  await expect(detail.getByRole('heading', { name: 'Revision 1' })).toBeVisible()
  await expect(page.getByRole('textbox', { name: 'Inventory revision 1' })).toContainText('http_port: 80')
  await expect(page.getByRole('textbox', { name: 'Inventory revision 1' })).toHaveAttribute('aria-readonly', 'true')

  // Restore it: a preview first, then revision 3 equal to revision 1.
  await detail.getByRole('button', { name: 'Restore revision 1' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Restore revision 1 of "prod" as revision 3.')
  expect(backend.inventories.revisions('prod')).toHaveLength(2)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()

  await expect(detail.getByRole('heading', { name: 'Revision 3 (newest)' })).toBeVisible()
  await expect(page.getByRole('textbox', { name: 'Inventory YAML' })).toContainText('http_port: 80')
  await expect(detail.getByRole('table', { name: 'Revisions' })).toContainText('restored from 1')
  const revisions = backend.inventories.revisions('prod')
  expect(revisions).toHaveLength(3)
  expect(revisions[2]?.source).toBe(V1)
  expect(revisions[2]?.summary.restored_from).toBe(1)

  // The two hold the same content now.
  await compare.getByLabel('From').selectOption('1')
  await compare.getByLabel('To').selectOption('3')
  await compare.getByRole('button', { name: 'Compare' }).click()
  await expect(detail.getByText('The two revisions hold the same content.')).toBeVisible()
})
