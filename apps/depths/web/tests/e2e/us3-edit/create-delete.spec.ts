/**
 * US3: creating and deleting stacks (T059), against the fake backend. Both are admin only; create
 * refuses a bad or taken name with its reason, and delete keeps the directory unless removing it is
 * switched on, which the preview then names. With editing off, editing is hidden and refused.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const dialog = (page: Page) => page.getByRole('dialog')
const COMPOSE = 'services:\n  app:\n    image: redis:8\n'

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web')])
}

async function openNew(page: Page): Promise<void> {
  await page.goto('/new')
  await expect(page.getByRole('heading', { name: 'New stack', level: 1 })).toBeVisible()
  await expect(page.locator('[data-code-editor="yaml"] .cm-content')).toBeVisible()
}

async function fillCreate(page: Page, name: string): Promise<void> {
  await page.getByLabel('Name', { exact: true }).fill(name)
  await page.locator('[data-code-editor="yaml"] .cm-content').click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText(COMPOSE)
}

test('the stack list offers New stack to an admin', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await page.locator('[data-action="new-stack"]').click()
  await expect(page).toHaveURL(/\/new$/)
})

test('a name that is not a compose project name is refused here, with the reason', async ({ page, backend }) => {
  seed(backend)
  await openNew(page)
  await fillCreate(page, 'My App')
  await expect(page.locator('[data-field="name-hint"]')).toHaveText(
    'Use lowercase letters, digits, - and _, starting with a letter or digit.',
  )
  await expect(page.locator('[data-action="create"]')).toBeDisabled()
  expect(backend.received.some((c) => c.type === 'PreviewSave')).toBe(false)
})

test("a taken name is refused by the server, with the server's reason", async ({ page, backend }) => {
  seed(backend)
  await openNew(page)
  await fillCreate(page, 'web')
  await page.locator('[data-action="create"]').click()
  const sent = await backend.waitForCmd('PreviewSave')
  expect(sent.data.draft.base_hash).toBeUndefined()
  await expect(dialog(page).locator('[data-refusal="invalid"]')).toHaveText('a stack named web already exists')
  await expect(dialog(page).locator('[data-confirm]')).toHaveCount(0)
})

test('create & deploy previews validate and up, then shows the output', async ({ page, backend }) => {
  seed(backend)
  await openNew(page)
  await fillCreate(page, 'cache')
  await page.locator('[data-action="create-deploy"]').click()
  const sent = await backend.waitForCmd('PreviewSave')
  expect(sent.data).toEqual({ draft: { stack: 'cache', compose: COMPOSE }, deploy: true })
  await expect(dialog(page).locator('[data-step]')).toHaveCount(2)
  await dialog(page).locator('[data-confirm]').click()
  await backend.waitForCmd('Confirm')
  await expect(page.locator('[data-operation="create"]')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Open cache' })).toBeVisible()
})

test('a user who is not an admin cannot create', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'])
  await page.goto('/')
  await expect(page.locator('[data-action="new-stack"]')).toHaveCount(0)
  await page.goto('/new')
  await expect(page.getByText('Only admins can create stacks.')).toBeVisible()
  await expect(page.locator('[data-code-editor]')).toHaveCount(0)
})

test('delete keeps the directory by default; removing it is named in the preview', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web')
  await page.locator('[data-action="delete"]').click()
  const first = await backend.waitForCmd('PreviewOp')
  expect(first.data).toEqual({ stack: 'web', op: { kind: 'delete', options: { remove_directory: false } } })

  const d = dialog(page)
  await expect(d.locator('[data-kept]')).toHaveText('The stack directory is kept.')
  const removeDir = d.getByRole('switch', { name: /also delete the stack directory/i })
  await expect(removeDir).not.toBeChecked()

  await removeDir.click()
  const next = await backend.waitForCmd('PreviewOp')
  expect(next.data.op).toEqual({ kind: 'delete', options: { remove_directory: true } })
  await expect(d.locator('[data-destructive]')).toContainText('the stack directory will be deleted')
  await expect(d.locator('[data-kept]')).toHaveCount(0)

  // Destructive: the name must be typed.
  const confirm = d.locator('[data-confirm]')
  await expect(confirm).toBeDisabled()
  await d.getByLabel(/type web to confirm/i).fill('web')
  await confirm.click()
  await backend.waitForCmd('Confirm')
  await expect(page.locator('[data-operation="delete"]')).toBeVisible()
})

test('a user who is not an admin cannot delete', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'])
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(page.locator('[data-action="delete"]')).toHaveCount(0)
})

test('with editing off, editing is hidden and the server refuses it', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.admin({ features: { shell: true, edit: false } })
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(page.locator('[data-action="edit"]')).toHaveCount(0)

  await page.goto('/')
  await expect(page.locator('[data-action="new-stack"]')).toHaveCount(0)

  await page.goto('/stacks/web/edit')
  await expect(page.getByText('Editing is turned off on this server.')).toBeVisible()
  await expect(page.locator('[data-code-editor]')).toHaveCount(0)

})

test('a save after editing was turned off on the server is refused there', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web/edit')
  const content = page.locator('[data-code-editor="yaml"] .cm-content')
  await expect(content).toContainText('nginx:1.29')
  // The server's config changes under the open editor: the server is the gate, not the button.
  backend.me = sessions.admin({ features: { shell: true, edit: false } })
  await content.click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText(COMPOSE)
  await page.locator('[data-action="save"]').click()
  await backend.waitForCmd('PreviewSave')
  await expect(dialog(page).locator('[data-refusal="denied"]')).toHaveText('This feature is turned off on this server.')
  await expect(dialog(page).locator('[data-confirm]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})
