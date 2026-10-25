/**
 * Spec 053 US5, file history (T060), against the fake backend: open a file's history (newest
 * first), compare a version with the file as it is now, restore it through preview → confirm well
 * inside 30 s of interaction (SC-005), and bring back a deleted file from its folder's view while
 * its history is kept.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const listing = (page: Page) => page.locator('[data-pane="listing"]')
const row = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const fileView = (page: Page) => page.locator('[data-pane="file"]')
const fileDialog = (page: Page) => page.locator('[data-file-op]')
const editor = (page: Page) => fileView(page).locator('[data-code-editor] .cm-content')

function seed(backend: FakeBackend): { data: string; older: string; newer: string } {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  const older = backend.files.keepVersion('web', data, 'server.properties', 'motd=First\n').id
  const newer = backend.files.keepVersion('web', data, 'server.properties', 'motd=Second\n').id
  backend.files.write('web', data, 'server.properties', 'motd=Now\n')
  return { data, older, newer }
}

async function openHistory(page: Page, data: string): Promise<void> {
  await page.goto(`/stacks/web/files/${data}/server.properties`)
  await expect(editor(page)).toContainText('motd=Now')
  await fileView(page).locator('[data-history-toggle]').click()
}

test('history lists the kept versions, newest first', async ({ page, backend }) => {
  const { data, older, newer } = seed(backend)
  await openHistory(page, data)
  await expect(fileView(page).locator('[data-history-toggle]')).toHaveAttribute('aria-expanded', 'true')
  const versions = fileView(page).locator('[data-history] [data-version]')
  await expect(versions).toHaveCount(2)
  expect(await versions.evaluateAll((els) => els.map((e) => e.getAttribute('data-version')))).toEqual([newer, older])
  await expect(versions.first()).toContainText('before a save')
  const cmd = await backend.waitForCmd('ListHistory')
  expect(cmd.data).toEqual({ stack: 'web', mount: data, path: 'server.properties' })
})

test('compare shows the file now against a version', async ({ page, backend }) => {
  const { data, older } = seed(backend)
  await openHistory(page, data)
  const version = fileView(page).locator(`[data-version="${older}"]`)
  await version.locator('[data-version-compare]').click()
  const diff = version.locator('[data-version-diff] [data-diff]')
  await expect(diff).toContainText('-motd=Now')
  await expect(diff).toContainText('+motd=First')
  const read = await backend.waitForCmd('ReadVersion')
  expect(read.data).toMatchObject({ mount: data, path: 'server.properties', version: older })
  // Nothing changed.
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=Now\n')
})

test('restore a version through preview and confirm, well inside 30 s', async ({ page, backend }) => {
  const { data, older } = seed(backend)
  const started = Date.now()
  await openHistory(page, data)
  await fileView(page).locator(`[data-version="${older}"] [data-version-restore]`).click()

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toEqual({ kind: 'restore', options: { mount: data, path: 'server.properties', version: older } })
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'restore')
  await expect(fileDialog(page).locator('[data-diff]')).toContainText('+motd=First')
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=Now\n')

  await fileDialog(page).locator('[data-confirm]').click()
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(editor(page)).toContainText('motd=First')
  expect(Date.now() - started).toBeLessThan(30_000)
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=First\n')
  // What it replaced is kept too.
  await expect(fileView(page).locator('[data-history] [data-version]')).toHaveCount(3)
  await expect(fileView(page).locator('[data-history] [data-version]').first()).toHaveAttribute('data-reason', 'restore')
})

test('a cancelled restore changes nothing', async ({ page, backend }) => {
  const { data, older } = seed(backend)
  await openHistory(page, data)
  await fileView(page).locator(`[data-version="${older}"] [data-version-restore]`).click()
  await expect(fileDialog(page).locator('[data-confirm]')).toBeEnabled()
  await fileDialog(page).getByRole('button', { name: 'Cancel' }).click()
  await expect(fileDialog(page)).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(0)
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=Now\n')
})

test('a deleted file comes back from its folder, its history kept', async ({ page, backend }) => {
  const { data } = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  await row(page, 'server.properties').locator('[data-row-actions]').click()
  await page.locator('[data-row-menu="server.properties"] [data-action="delete"]').click()
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(row(page, 'server.properties')).toHaveCount(0)
  expect(backend.files.exists('web', data, 'server.properties')).toBe(false)
  // The delete kept the content as a version, beside the two before it.
  expect(backend.files.history('web', data, 'server.properties').map((v) => v.reason)).toEqual(['delete', 'save', 'save'])

  await listing(page).locator('[data-deleted-toggle]').click()
  await listing(page).locator('[data-deleted-file="server.properties"]').click()
  const versions = listing(page).locator('[data-history="server.properties"] [data-version]')
  await expect(versions).toHaveCount(3)
  await expect(versions.first()).toContainText('when it was deleted')
  await versions.first().locator('[data-version-compare]').click()
  await expect(versions.first().locator('[data-diff]')).toContainText('+motd=Now')

  await versions.first().locator('[data-version-restore]').click()
  await expect(fileDialog(page).locator('[data-field="summary"]')).toContainText('Restore server.properties')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(page).toHaveURL(new RegExp(`/files/${data}/server\\.properties$`))
  await expect(editor(page)).toContainText('motd=Now')
  await expect(row(page, 'server.properties')).toBeVisible()
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=Now\n')
})

test('a file deleted before this session is found by name', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'keep.txt', 'x')
  backend.files.keepVersion('web', data, 'gone.txt', 'old\n', 'delete')
  await page.goto(`/stacks/web/files/${data}`)
  await expect(row(page, 'keep.txt')).toBeVisible()
  await listing(page).locator('[data-deleted-toggle]').click()
  await expect(listing(page).locator('[data-deleted]')).toContainText('No files were deleted here in this session.')
  await listing(page).locator('[data-deleted-name]').fill('gone.txt')
  await listing(page).locator('[data-deleted-name]').press('Enter')
  await expect(listing(page).locator('[data-history="gone.txt"] [data-version]')).toHaveCount(1)
})

test('a read-only mount shows history but offers no restore', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data', writable: false })
  backend.files.keepVersion('web', data, 'server.properties', 'motd=First\n')
  backend.files.write('web', data, 'server.properties', 'motd=Now\n')
  await openHistory(page, data)
  await expect(fileView(page).locator('[data-history] [data-version]')).toHaveCount(1)
  await expect(fileView(page).locator('[data-version-restore]')).toHaveCount(0)
  await expect(listing(page).locator('[data-deleted-toggle]')).toHaveCount(0)
})
