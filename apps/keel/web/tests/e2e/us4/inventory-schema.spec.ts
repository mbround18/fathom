/**
 * US4 (spec 055): a managed inventory's var schema is edited on the inventory page, checked
 * against the newest revision as it is typed, then saved or removed through a preview. Against
 * the fake backend (tests/support/fake-inventories.ts).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'

const V1 = 'web:\n  hosts:\n    web1:\n      http_port: 80\n'

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
  if (text !== '') await page.keyboard.insertText(text)
}

test('a var schema is checked against the newest revision, then saved and removed', async ({ page, backend }) => {
  admin(backend)
  backend.inventories.seed('prod', [V1])
  await page.goto('/inventory')
  await page.getByRole('table', { name: 'Inventories' }).getByRole('button', { name: 'prod' }).click()
  const detail = page.getByRole('region', { name: 'Inventory prod' })
  const schema = page.getByRole('textbox', { name: 'Var schema JSON' })
  await expect(schema).toContainText('http_port')
  const save = detail.getByRole('button', { name: 'Save schema' })
  await expect(save).toBeDisabled()

  // A schema the newest revision fails cannot be saved.
  await replaceText(page, 'Var schema JSON', '{"properties": {"http_port": {"type": "string"}}}')
  const problems = detail.getByRole('list', { name: 'Schema problems' })
  await expect(problems).toContainText('web.hosts.web1.http_port')
  await expect(problems).toContainText('expected string')
  await expect(save).toBeDisabled()

  await replaceText(page, 'Var schema JSON', '{not json')
  await expect(problems).toContainText('schema')
  await expect(problems).toContainText('not valid JSON')
  await expect(save).toBeDisabled()
  expect(backend.received.some((c) => c.type === 'PreviewInventorySchema')).toBe(false)

  // One it passes previews, then replaces the stored schema.
  await replaceText(page, 'Var schema JSON', '{"required": ["http_port"], "properties": {"http_port": {"type": "integer"}}}')
  await expect(detail.getByText('Checked: the newest revision passes this schema.')).toBeVisible()
  await expect(problems).toBeHidden()
  await save.click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Check the vars of "prod" against the new schema from now on.')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
  expect(backend.inventories.schemaOf('prod')?.required).toEqual(['http_port'])
  await expect(schema).toContainText('required')
  await expect(detail.getByText('The schema has no unsaved changes.')).toBeVisible()

  // Emptying the editor removes it.
  await replaceText(page, 'Var schema JSON', '')
  await expect(detail.getByText('Saving removes the schema.')).toBeVisible()
  await detail.getByRole('button', { name: 'Remove schema' }).click()
  await expect(dialog).toContainText('Remove the var schema of "prod"; its vars are no longer checked.')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
  expect(backend.inventories.schemaOf('prod')).toBeNull()
  await expect(detail.getByText(/^No var schema: vars are not checked\./)).toBeVisible()
  await expect(detail).not.toContainText('vars checked against a schema')
})

test('a new inventory can start with a var schema', async ({ page, backend }) => {
  admin(backend)
  await page.goto('/inventory')
  await page.getByRole('button', { name: 'New inventory' }).click()
  const form = page.getByRole('form', { name: 'New inventory' })
  await form.getByLabel('Name').fill('edge')
  await replaceText(page, 'New inventory YAML', V1)
  await replaceText(page, 'New inventory var schema JSON (optional)', '[1, 2]')
  await form.getByRole('button', { name: 'Preview inventory' }).click()
  await expect(page.getByRole('alert')).toContainText('not a JSON object')

  await replaceText(page, 'New inventory var schema JSON (optional)', '{"required": ["http_port"]}')
  await form.getByRole('button', { name: 'Preview inventory' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
  expect(backend.inventories.schemaOf('edge')).toEqual({ required: ['http_port'] })
  await expect(page.getByRole('region', { name: 'Inventory edge' })).toContainText('vars checked against a schema')
})
