/**
 * Spec 053 US2, editing a file in a volume (T041), against the fake backend: a save previews the
 * server's diff and writes only on Confirm; a file that changed on disk offers reload or compare
 * and save over; a running service is named, with **Save and restart <service>** going through
 * the ordinary lifecycle preview; parse problems are marked on their line and do not block a
 * save; a read-only mount opens read-only; leaving with unsaved changes asks first.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const fileView = (page: Page) => page.locator('[data-pane="file"]')
const editor = (page: Page) => fileView(page).locator('[data-code-editor]')
const fileDialog = (page: Page) => page.locator('[data-file-op]')
const opDialog = (page: Page) => page.locator('[data-preview]')
const saveButton = (page: Page) => fileView(page).locator('[data-save]')

const PROPS = 'motd=Hello\nmax-players=20\n'

function seed(backend: FakeBackend, over: Parameters<FakeBackend['files']['addMount']>[1] = {}): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data', ...over })
  backend.files.write('web', data, 'config/server.properties', PROPS)
  return data
}

async function openFile(page: Page, mount: string, path: string, shows: string): Promise<void> {
  await page.goto(`/stacks/web/files/${mount}/${path}`)
  await expect(editor(page).locator('.cm-content')).toContainText(shows)
}

/** Replaces the whole text, as typing would. */
async function setText(page: Page, text: string): Promise<void> {
  await editor(page).locator('.cm-content').click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText(text)
}

test('edit, preview the diff, confirm: the file is saved and shown saved', async ({ page, backend }) => {
  const data = seed(backend)
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await expect(saveButton(page)).toBeDisabled()

  await setText(page, 'motd=Welcome\nmax-players=20\n')
  await expect(fileView(page).locator('[data-dirty]')).toHaveText('Unsaved changes')
  await saveButton(page).click()

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toMatchObject({ kind: 'save', options: { mount: data, path: 'config/server.properties', text: 'motd=Welcome\nmax-players=20\n' } })
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'save')
  await expect(fileDialog(page).locator('[data-diff-line="removed"]').first()).toContainText('motd=Hello')
  await expect(fileDialog(page).locator('[data-diff-line="added"]').first()).toContainText('motd=Welcome')
  // Nothing is written by the preview.
  expect(backend.files.read('web', data, 'config/server.properties')).toBe(PROPS)

  await fileDialog(page).locator('[data-confirm]').click()
  await backend.waitForCmd('Confirm')
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(fileView(page).locator('[data-save-status]')).toHaveText('Saved.')
  await expect(saveButton(page)).toBeDisabled()
  await expect(editor(page).locator('.cm-content')).toContainText('motd=Welcome')
  expect(backend.files.read('web', data, 'config/server.properties')).toBe('motd=Welcome\nmax-players=20\n')
})

test('cancel closes the preview and writes nothing', async ({ page, backend }) => {
  const data = seed(backend)
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Nope\n')
  await saveButton(page).click()
  await expect(fileDialog(page)).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(fileDialog(page)).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(0)
  expect(backend.files.read('web', data, 'config/server.properties')).toBe(PROPS)
  await expect(fileView(page).locator('[data-dirty]')).toHaveText('Unsaved changes')
})

test('changed on disk: reload drops the edit', async ({ page, backend }) => {
  const data = seed(backend)
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Mine\n')
  backend.files.write('web', data, 'config/server.properties', 'motd=Theirs\n')
  await saveButton(page).click()
  await fileDialog(page).locator('[data-confirm]').click()

  const conflict = fileView(page).locator('[data-conflict]')
  await expect(conflict).toContainText('changed on disk')
  await expect(fileDialog(page)).toHaveCount(0)
  expect(backend.files.read('web', data, 'config/server.properties')).toBe('motd=Theirs\n')

  await conflict.locator('[data-reload]').click()
  await expect(editor(page).locator('.cm-content')).toContainText('motd=Theirs')
  await expect(conflict).toHaveCount(0)
  await expect(saveButton(page)).toBeDisabled()
})

test('changed on disk: compare, then save over it', async ({ page, backend }) => {
  const data = seed(backend)
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Mine\n')
  backend.files.write('web', data, 'config/server.properties', 'motd=Theirs\n')
  await saveButton(page).click()
  await fileDialog(page).locator('[data-confirm]').click()

  const conflict = fileView(page).locator('[data-conflict]')
  await conflict.locator('[data-compare]').click()
  const diff = conflict.locator('[data-compare-diff]')
  await expect(diff.locator('[data-diff-line="removed"]')).toContainText('motd=Theirs')
  await expect(diff.locator('[data-diff-line="added"]')).toContainText('motd=Mine')

  await diff.locator('[data-save-over]').click()
  await backend.waitForCmd('PreviewFileOp')
  const second = await backend.waitForCmd('PreviewFileOp')
  expect(second.data.op).toMatchObject({ kind: 'save', options: { base: backend.files.fingerprint('web', data, 'config/server.properties') } })
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(fileView(page).locator('[data-save-status]')).toHaveText('Saved.')
  expect(backend.files.read('web', data, 'config/server.properties')).toBe('motd=Mine\n')
})

test('a running service is named; Save and restart goes through the lifecycle preview', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.setRunning('web', data, ['app'])
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Restarted\n')
  await saveButton(page).click()

  await expect(fileDialog(page).locator('[data-running]')).toContainText('app is running')
  await fileDialog(page).locator('[data-confirm-restart="app"]').click()
  await backend.waitForCmd('Confirm')
  expect(backend.files.read('web', data, 'config/server.properties')).toBe('motd=Restarted\n')

  const op = await backend.waitForCmd('PreviewOp')
  expect(op.data).toEqual({ stack: 'web', op: { kind: 'service_restart', options: { service: 'app' } } })
  await expect(opDialog(page)).toHaveAttribute('data-preview', 'service_restart')
  await opDialog(page).locator('[data-confirm]').click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.preview_id).toMatch(/^pv-/)
})

test('plain Save with a running service saves without a restart', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.setRunning('web', data, ['app'])
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Quiet\n')
  await saveButton(page).click()
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(fileView(page).locator('[data-save-status]')).toHaveText('Saved.')
  expect(backend.received.filter((c) => c.type === 'PreviewOp')).toHaveLength(0)
})

for (const [name, bad, line] of [
  ['settings.json', '{\n  "a": 1,\n}\n', '3'],
  ['config.yaml', 'a: 1\nb: c: d\n', '2'],
  ['server.toml', 'a = 1\nb = \n', '2'],
  ['layout.xml', '<a>\n<b>\n</a>\n', '3'],
  ['game.ini', '[server]\nport=1\njust words\n', '3'],
] as const) {
  test(`invalid ${name} is marked on line ${line} and still saves after a confirm`, async ({ page, backend }) => {
    const data = seed(backend)
    backend.files.write('web', data, name, 'x\n')
    await page.goto(`/stacks/web/files/${data}/${name}`)
    await expect(editor(page).locator('.cm-content')).toBeVisible()
    await setText(page, bad)

    const problem = editor(page).locator('[data-problem-line]').first()
    await expect(problem).toHaveAttribute('data-problem-line', line)
    await expect(problem).toHaveAttribute('data-problem-severity', 'warning')

    await saveButton(page).click()
    const ask = fileView(page).locator('[data-save-anyway]')
    await expect(ask).toBeVisible()
    expect(backend.received.filter((c) => c.type === 'PreviewFileOp')).toHaveLength(0)
    await ask.getByRole('button', { name: 'Save anyway' }).click()
    await fileDialog(page).locator('[data-confirm]').click()
    await expect(fileView(page).locator('[data-save-status]')).toHaveText('Saved.')
    expect(backend.files.read('web', data, name)).toBe(bad)
  })
}

test('a read-only mount opens read-only with the reason and no Save', async ({ page, backend }) => {
  const data = seed(backend, { writable: false })
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await expect(fileView(page).locator('[data-read-only-reason]')).toContainText('read-only in every container')
  await expect(editor(page)).toHaveAttribute('data-read-only', 'true')
  await expect(saveButton(page)).toHaveCount(0)
})

test('leaving with unsaved changes asks first', async ({ page, backend }) => {
  const data = seed(backend)
  await openFile(page, data, 'config/server.properties', 'motd=Hello')
  await setText(page, 'motd=Draft\n')

  const asked: string[] = []
  page.once('dialog', (d) => {
    asked.push(d.message())
    void d.dismiss()
  })
  await fileView(page).getByRole('link', { name: 'Back to config' }).click()
  await expect.poll(() => asked).toEqual(['You have unsaved changes. Leave and discard them?'])
  await expect(page).toHaveURL(new RegExp(`/files/${data}/config/server.properties$`))

  page.once('dialog', (d) => void d.accept())
  await fileView(page).getByRole('link', { name: 'Back to config' }).click()
  await expect(page).toHaveURL(new RegExp(`/files/${data}/config$`))
})
