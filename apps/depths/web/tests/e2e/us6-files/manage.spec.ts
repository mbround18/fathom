/**
 * Spec 053 US4, managing files (T054), against the fake backend: create a file and a folder,
 * rename, move a file into another folder of the same mount, and delete with a confirm that names
 * the entry and counts what is inside; a delete that stops part way says what it removed and the
 * system's error; no move to another mount is offered. Every change previews and writes only on
 * Confirm.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const listing = (page: Page) => page.locator('[data-pane="listing"]')
const row = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const nameDialog = (page: Page) => page.locator('[data-name-dialog]')
const fileDialog = (page: Page) => page.locator('[data-file-op]')

function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'config/server.properties', 'motd=Hello\n')
  backend.files.write('web', data, 'notes.txt', 'hi\n')
  backend.files.mkdir('web', data, 'logs')
  return data
}

async function openFolder(page: Page, mount: string, path = ''): Promise<void> {
  await page.goto(`/stacks/web/files/${mount}${path === '' ? '' : `/${path}`}`)
  await expect(listing(page).locator('[data-entry]').first()).toBeVisible()
}

async function actions(page: Page, name: string, action: 'rename' | 'delete'): Promise<void> {
  await row(page, name).locator('[data-row-actions]').click()
  await page.locator(`[data-row-menu="${name}"] [data-action="${action}"]`).click()
}

test('create a file: name, preview, confirm; it opens', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data, 'config')
  await listing(page).locator('[data-new="file"]').click()
  await nameDialog(page).getByRole('textbox').fill('extra.yaml')
  await nameDialog(page).locator('[data-name-submit]').click()

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toEqual({ kind: 'create', options: { mount: data, parent: 'config', name: 'extra.yaml', kind: 'file' } })
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Create file config/extra.yaml')
  expect(backend.files.exists('web', data, 'config/extra.yaml')).toBe(false)

  await fileDialog(page).locator('[data-confirm]').click()
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(page).toHaveURL(new RegExp(`/files/${data}/config/extra\\.yaml$`))
  await expect(row(page, 'extra.yaml')).toBeVisible()
  expect(backend.files.read('web', data, 'config/extra.yaml')).toBe('')
})

test('create a folder; a name with "/" is caught before anything is sent', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await listing(page).locator('[data-new="dir"]').click()
  const input = nameDialog(page).getByRole('textbox')
  await input.fill('a/b')
  await input.press('Enter')
  await expect(nameDialog(page).getByRole('alert')).toContainText('cannot hold')
  expect(backend.received.filter((c) => c.type === 'PreviewFileOp')).toHaveLength(0)

  await input.fill('backups')
  await input.press('Enter')
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Create folder backups')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'backups')).toHaveAttribute('data-kind', 'dir')
  expect(backend.files.exists('web', data, 'backups')).toBe(true)
})

test('an existing name is refused by the server and nothing changes', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await listing(page).locator('[data-new="file"]').click()
  await nameDialog(page).getByRole('textbox').fill('notes.txt')
  await nameDialog(page).locator('[data-name-submit]').click()
  await expect(fileDialog(page).locator('[data-refusal="exists"]')).toContainText('already exists')
  await expect(fileDialog(page).locator('[data-confirm]')).toHaveCount(0)
  expect(backend.files.read('web', data, 'notes.txt')).toBe('hi\n')
})

test('rename from the actions menu shows old → new and renames on confirm', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await actions(page, 'notes.txt', 'rename')
  const input = nameDialog(page).getByRole('textbox')
  await expect(input).toHaveValue('notes.txt')
  await input.fill('readme.txt')
  await input.press('Enter')

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toEqual({ kind: 'rename', options: { mount: data, from: 'notes.txt', to: 'readme.txt' } })
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Rename notes.txt → readme.txt')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'readme.txt')).toBeVisible()
  await expect(row(page, 'notes.txt')).toHaveCount(0)
  expect(backend.files.read('web', data, 'readme.txt')).toBe('hi\n')
})

test('move a file into another folder of the same mount, by keyboard (F2)', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await row(page, 'notes.txt').locator('[data-row-link]').focus()
  await page.keyboard.press('F2')
  const input = nameDialog(page).getByRole('textbox')
  await expect(input).toBeFocused()
  await input.fill('logs/notes.txt')
  await input.press('Enter')
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Rename notes.txt → logs/notes.txt')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'notes.txt')).toHaveCount(0)
  expect(backend.files.exists('web', data, 'notes.txt')).toBe(false)
  expect(backend.files.read('web', data, 'logs/notes.txt')).toBe('hi\n')
})

test('renaming the open file follows it to its new path', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/notes.txt`)
  await expect(page.locator('[data-pane="file"] [data-code-editor] .cm-content')).toContainText('hi')
  await actions(page, 'notes.txt', 'rename')
  await nameDialog(page).getByRole('textbox').fill('logs/kept.txt')
  await nameDialog(page).locator('[data-name-submit]').click()
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(page).toHaveURL(new RegExp(`/files/${data}/logs/kept\\.txt$`))
})

test('no move to another mount is offered; ".." is caught', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.addMount('web', { source: 'web_other' })
  await openFolder(page, data)
  await actions(page, 'notes.txt', 'rename')
  const dialog = nameDialog(page)
  await expect(dialog.locator('[data-mount-root]')).toHaveText('web_data/')
  await expect(dialog.getByRole('combobox')).toHaveCount(0)
  await expect(dialog).not.toContainText('web_other')

  await dialog.getByRole('textbox').fill('../web_other/notes.txt')
  await dialog.getByRole('textbox').press('Enter')
  await expect(dialog.getByRole('alert')).toContainText('inside this mount')
  expect(backend.received.filter((c) => c.type === 'PreviewFileOp')).toHaveLength(0)
})

test('delete a folder: the confirm names it and counts what is inside', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.write('web', data, 'logs/a.log', 'a')
  backend.files.write('web', data, 'logs/old/b.log', 'b')
  await openFolder(page, data)
  await actions(page, 'logs', 'delete')

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toEqual({ kind: 'delete', options: { mount: data, path: 'logs' } })
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'delete')
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Delete logs')
  await expect(fileDialog(page).locator('[data-field="entries"]')).toContainText('3 entries')
  expect(backend.files.exists('web', data, 'logs')).toBe(true)

  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'logs')).toHaveCount(0)
  expect(backend.files.exists('web', data, 'logs')).toBe(false)
})

test('delete a file by keyboard; cancelling leaves it', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await row(page, 'notes.txt').locator('[data-row-link]').focus()
  await page.keyboard.press('Delete')
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Delete notes.txt')
  await page.keyboard.press('Escape')
  await expect(fileDialog(page)).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(0)
  expect(backend.files.exists('web', data, 'notes.txt')).toBe(true)

  await row(page, 'notes.txt').locator('[data-row-link]').focus()
  await page.keyboard.press('Delete')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'notes.txt')).toHaveCount(0)
  expect(backend.files.exists('web', data, 'notes.txt')).toBe(false)
})

test('a right-click opens the same actions menu', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await row(page, 'notes.txt').click({ button: 'right' })
  const menu = page.locator('[data-row-menu="notes.txt"]')
  await expect(menu.getByRole('menuitem')).toHaveText(['Download', 'Rename or move…', 'Delete…'])
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
})

test('a delete that stops part way says what it removed and the system error', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await actions(page, 'logs', 'delete')
  await expect(fileDialog(page).locator('[data-confirm]')).toBeEnabled()
  backend.files.refuseNext('partial', 'Confirm')
  await fileDialog(page).locator('[data-confirm]').click()

  await expect(fileDialog(page).locator('[data-refusal="partial"]')).toHaveText(
    'Stopped after removing 3 entries: locked.db could not be removed: Permission denied (EACCES).',
  )
  const lists = backend.received.filter((c) => c.type === 'ListDir').length
  await fileDialog(page).getByRole('button', { name: 'Cancel' }).click()
  await expect(fileDialog(page)).toHaveCount(0)
  // The folder changed, so it is listed again.
  await expect.poll(() => backend.received.filter((c) => c.type === 'ListDir').length).toBeGreaterThan(lists)
})

test('a read-only mount offers no changes', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data', writable: false })
  backend.files.write('web', data, 'notes.txt', 'hi\n')
  await openFolder(page, data)
  await expect(listing(page).locator('[data-new]')).toHaveCount(0)
  await expect(listing(page).locator('[data-upload-pick]')).toHaveCount(0)
  await row(page, 'notes.txt').locator('[data-row-actions]').click()
  await expect(page.locator('[data-row-menu="notes.txt"]').getByRole('menuitem')).toHaveText(['Download'])
  await page.keyboard.press('Escape')
  await row(page, 'notes.txt').locator('[data-row-link]').focus()
  await page.keyboard.press('Delete')
  await page.keyboard.press('F2')
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(nameDialog(page)).toHaveCount(0)
})
