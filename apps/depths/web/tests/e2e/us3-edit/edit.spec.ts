/**
 * US3: editing a stack's files (T058), against the fake backend. The editor highlights and checks
 * YAML as you type, anchors the server's errors to their lines, previews a save as a diff (plus
 * `up` for Save & deploy) before anything is written, and shows the other change on a conflict.
 * `.env` stays masked until the audited reveal.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const ORIGINAL = 'services:\n  app:\n    image: nginx:1.29\n'
const CHANGED = 'services:\n  app:\n    image: nginx:1.30\n'

const dialog = (page: Page) => page.getByRole('dialog')
const composeEditor = (page: Page) => page.locator('[data-code-editor="yaml"]')
const saveButton = (page: Page) => page.locator('[data-action="save"]')

function seed(backend: FakeBackend, over: Parameters<typeof f.stack>[1] = {}): void {
  backend.stacks = f.stackList([f.stack('web', over)])
}

async function openEditor(page: Page): Promise<void> {
  await page.goto('/stacks/web/edit')
  await expect(page.getByRole('heading', { name: 'Edit web', level: 1 })).toBeVisible()
  await expect(composeEditor(page).locator('.cm-content')).toContainText('nginx:1.29')
}

/** Replace the whole compose text, as typing would. */
async function setCompose(page: Page, text: string): Promise<void> {
  await composeEditor(page).locator('.cm-content').click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText(text)
}

test('the compose file opens highlighted, with nothing to save yet', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  await expect(composeEditor(page).locator('.cm-line span').first()).toBeVisible()
  await expect(page.locator('[data-field="dirty"]')).toHaveText('No changes')
  await expect(saveButton(page)).toBeDisabled()
})

test('the stack page links to the editor while editing is on', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web')
  await page.locator('[data-action="edit"]').click()
  await expect(page).toHaveURL(/\/stacks\/web\/edit$/)
})

test('a broken indent is anchored to its line and blocks saving', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  await setCompose(page, 'services:\n  app:\n    image: nginx:1.30\n   ports: []\n')
  const problem = composeEditor(page).locator('[data-problem-source="yaml"]').first()
  await expect(problem).toHaveAttribute('data-problem-line', '4')
  await expect(problem).toContainText('Line 4:')
  await expect(page.locator('[data-field="save-blocked"]')).toBeVisible()
  await expect(saveButton(page)).toBeDisabled()
  await expect(page.locator('[data-action="save-deploy"]')).toBeDisabled()

  // Fixing it clears the problem and allows saving.
  await setCompose(page, CHANGED)
  await expect(composeEditor(page).locator('[data-problems]')).toHaveCount(0)
  await expect(saveButton(page)).toBeEnabled()
})

test("the server's validation error is anchored to its line and nothing is written", async ({ page, backend }) => {
  seed(backend)
  backend.validationErrors.web = [{ line: 3, message: 'service "app" refers to undefined volume data' }]
  await openEditor(page)
  await setCompose(page, CHANGED)
  await saveButton(page).click()
  await backend.waitForCmd('PreviewSave')

  const problem = composeEditor(page).locator('[data-problem-source="server"]')
  await expect(problem).toHaveAttribute('data-problem-line', '3')
  await expect(problem).toHaveText('Compose: Line 3: service "app" refers to undefined volume data')
  await expect(page.locator('[data-field="server-errors"]')).toBeVisible()
  await expect(dialog(page)).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
})

test('the diff shows only the change, in the editor and in the preview', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  await setCompose(page, CHANGED)
  await expect(page.locator('[data-field="dirty"]')).toHaveText('Unsaved changes')

  await page.getByRole('button', { name: 'Diff', exact: true }).click()
  const diff = page.locator('[data-editor-diff]')
  await expect(diff.getByTestId('diff-added')).toHaveCount(1)
  await expect(diff.getByTestId('diff-added')).toContainText('nginx:1.30')
  await expect(diff.getByTestId('diff-removed')).toHaveCount(1)
  await expect(diff.getByTestId('diff-removed')).toContainText('nginx:1.29')

  await saveButton(page).click()
  const sent = await backend.waitForCmd('PreviewSave')
  expect(sent.data).toEqual({ draft: { stack: 'web', base_hash: 'hash-web-1', compose: CHANGED }, deploy: false })
  const changes = dialog(page).getByRole('region', { name: 'File changes' })
  await expect(changes.locator('[data-diff-line="added"]')).toHaveText(['+    image: nginx:1.30'])
  await expect(changes.locator('[data-diff-line="removed"]')).toHaveText(['-    image: nginx:1.29'])
})

test('a conflict shows the other change and writes nothing', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  // Someone else saves after the editor opened.
  const theirs = 'services:\n  app:\n    image: nginx:1.31\n'
  backend.details.web = f.detail(f.stack('web'), { compose_text: theirs, content_hash: 'hash-web-2' })

  await setCompose(page, CHANGED)
  await saveButton(page).click()
  await backend.waitForCmd('PreviewSave')
  const conflict = page.locator('[data-conflict]')
  await expect(conflict).toContainText('Someone else saved first. Nothing was written.')
  await expect(conflict.locator('[data-their-change]').getByTestId('diff-added')).toContainText('nginx:1.31')
  await expect(dialog(page)).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)
  // My draft is still there.
  await expect(composeEditor(page).locator('.cm-content')).toContainText('nginx:1.30')

  // Keeping mine saves against their version.
  await conflict.locator('[data-action="keep-mine"]').click()
  await expect(conflict).toHaveCount(0)
  await saveButton(page).click()
  const again = await backend.waitForCmd('PreviewSave')
  expect(again.data.draft.base_hash).toBe('hash-web-2')
  await expect(dialog(page).locator('[data-confirm]')).toBeEnabled()
})

test('using their version replaces the draft', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  const theirs = 'services:\n  app:\n    image: nginx:1.31\n'
  backend.details.web = f.detail(f.stack('web'), { compose_text: theirs, content_hash: 'hash-web-2' })
  await setCompose(page, CHANGED)
  await saveButton(page).click()
  await backend.waitForCmd('PreviewSave')
  await page.locator('[data-action="take-theirs"]').click()
  await expect(composeEditor(page).locator('.cm-content')).toContainText('nginx:1.31')
  await expect(page.locator('[data-field="dirty"]')).toHaveText('No changes')
})

test('Save & deploy previews the diff and `up` together, then runs on the stack page', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  await setCompose(page, CHANGED)
  await page.locator('[data-action="save-deploy"]').click()
  const sent = await backend.waitForCmd('PreviewSave')
  expect(sent.data.deploy).toBe(true)
  expect(sent.data.draft.env).toBeUndefined()

  const d = dialog(page)
  await expect(d.locator('[data-step="0"] [data-argv]')).toHaveAttribute(
    'data-argv',
    JSON.stringify(['docker', 'compose', '-f', '-', 'config', '--quiet']),
  )
  await expect(d.locator('[data-step="0"] [data-field="stdin"]')).toBeVisible()
  await expect(d.locator('[data-step="1"] [data-argv]')).toHaveAttribute(
    'data-argv',
    JSON.stringify(['docker', 'compose', '-p', 'web', '-f', 'compose.yaml', 'up', '-d', '--remove-orphans']),
  )
  await expect(d.getByRole('region', { name: 'File changes' }).locator('[data-diff-line="added"]')).toHaveCount(1)

  await d.locator('[data-confirm]').click()
  await backend.waitForCmd('Confirm')
  await expect(page).toHaveURL(/\/stacks\/web$/)
  await expect(page.locator('[data-operation="save"]')).toBeVisible()
})

test('leaving with unsaved changes asks first', async ({ page, backend }) => {
  seed(backend)
  await openEditor(page)
  await setCompose(page, CHANGED)
  const back = page.getByRole('link', { name: 'web', exact: true })

  page.once('dialog', (d) => void d.dismiss())
  await back.click()
  await expect(page).toHaveURL(/\/stacks\/web\/edit$/)

  page.once('dialog', (d) => void d.accept())
  await back.click()
  await expect(page).toHaveURL(/\/stacks\/web$/)
})

test('.env is masked until revealed, and a user on an assigned stack can reveal it', async ({ page, backend }) => {
  seed(backend, { has_env: true })
  backend.me = sessions.user(['web'])
  backend.env.web = 'PORT=8080\nDB_PASSWORD=hunter2\n'
  await openEditor(page)

  const masked = page.locator('[data-code-view="env"]')
  await expect(masked).toContainText('DB_PASSWORD=********')
  await expect(page.getByText('hunter2')).toHaveCount(0)
  await expect(page.locator('[data-code-editor="env"]')).toHaveCount(0)

  await page.locator('[data-action="reveal-env"]').click()
  const asked = await backend.waitForCmd('RevealEnv')
  expect(asked.data).toEqual({ name: 'web' })
  const env = page.locator('[data-code-editor="env"] .cm-content')
  await expect(env).toContainText('DB_PASSWORD=hunter2')

  // Editing .env sends it; the compose text goes unchanged.
  await env.click()
  await page.keyboard.press('ControlOrMeta+End')
  await page.keyboard.insertText('DEBUG=1\n')
  await saveButton(page).click()
  const sent = await backend.waitForCmd('PreviewSave')
  expect(sent.data.draft.env).toBe('PORT=8080\nDB_PASSWORD=hunter2\nDEBUG=1\n')
  expect(sent.data.draft.compose).toBe(ORIGINAL)
})

test('a refused reveal keeps .env masked', async ({ page, backend }) => {
  seed(backend, { has_env: true })
  backend.deny('RevealEnv')
  await openEditor(page)
  await page.locator('[data-action="reveal-env"]').click()
  await backend.waitForCmd('RevealEnv')
  await expect(page.getByRole('alert').filter({ hasText: 'You are not permitted to do this.' })).toBeVisible()
  await expect(page.locator('[data-code-editor="env"]')).toHaveCount(0)
})
