/**
 * Spec 053, the files feature's modes (T068), against the fake backend: `off` hides the Files tab;
 * `read_only` keeps browse, open and download and hides every control that changes a file; and an
 * admin sees why the server turned files off when it says (`files_reason`).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const views = (page: Page) => page.getByRole('navigation', { name: 'Stack views' })
const listing = (page: Page) => page.locator('[data-pane="listing"]')
const row = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const fileView = (page: Page) => page.locator('[data-pane="file"]')

function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'server.properties', 'motd=Now\n')
  backend.files.keepVersion('web', data, 'server.properties', 'motd=First\n')
  backend.files.mkdir('web', data, 'world')
  return data
}

test('off: no Files tab, and its URL says files are not available', async ({ page, backend }) => {
  seed(backend)
  backend.setFiles('off')
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(views(page)).toHaveCount(0)
  await page.goto('/stacks/web/files')
  await expect(page.locator('[data-pane-state="unavailable"]')).toBeVisible()
  expect(backend.received.filter((c) => c.type === 'ListMounts')).toHaveLength(0)
})

test('read_only: browse, open and download, and nothing that changes a file', async ({ page, backend }) => {
  const data = seed(backend)
  backend.setFiles('read_only')
  await page.goto(`/stacks/web/files/${data}`)
  await expect(row(page, 'server.properties')).toBeVisible()
  await expect(row(page, 'world')).toBeVisible()

  // No change controls in the listing.
  for (const sel of ['[data-new]', '[data-upload-pick]', '[data-upload-input]', '[data-deleted-toggle]']) {
    await expect(listing(page).locator(sel)).toHaveCount(0)
  }
  await expect(page.locator('[data-drop-target]')).toHaveCount(0)
  await expect(listing(page).locator('[data-download-folder]')).toBeVisible()

  // The row menu only downloads.
  await row(page, 'server.properties').locator('[data-row-actions]').click()
  const menu = page.locator('[data-row-menu="server.properties"]')
  await expect(menu.getByRole('menuitem')).toHaveText(['Download'])
  await expect(menu.locator('[data-action="download"]')).toHaveAttribute('href', /\/files\/download/)
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)

  // F2 and Delete on a focused row do nothing.
  const link = row(page, 'server.properties').locator('[data-row-link]')
  await link.focus()
  await page.keyboard.press('F2')
  await page.keyboard.press('Delete')
  await expect(page.locator('[data-name-dialog], [role="dialog"]')).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'PreviewFileOp')).toHaveLength(0)

  // A file opens read-only: no save, the reason shown, history without restore.
  await link.press('Enter')
  await expect(page).toHaveURL(new RegExp(`/files/${data}/server\\.properties$`))
  await expect(fileView(page).locator('[data-code-editor] .cm-content')).toContainText('motd=Now')
  await expect(fileView(page).locator('[data-read-only-reason]')).toContainText('Files are read-only on this server')
  await expect(fileView(page).locator('[data-save]')).toHaveCount(0)
  await fileView(page).locator('[data-history-toggle]').click()
  await expect(fileView(page).locator('[data-history] [data-version]')).toHaveCount(1)
  await expect(fileView(page).locator('[data-version-restore]')).toHaveCount(0)
  expect(backend.files.read('web', data, 'server.properties')).toBe('motd=Now\n')
})

test('an admin sees why files are off', async ({ page, backend }) => {
  seed(backend)
  backend.me = f.me({ features: { shell: true, edit: true, host_metrics: false, files: 'off', files_reason: 'the helper image is missing' } })
  await page.goto('/stacks/web')
  await expect(page.locator('[data-files-reason]')).toHaveText('Files are off on this server: the helper image is missing')
  await expect(views(page)).toHaveCount(0)
})

test('with no reason given, no note shows', async ({ page, backend }) => {
  seed(backend)
  backend.me = f.me({ role: 'user', stacks: ['web'], features: { shell: true, edit: true, host_metrics: false, files: 'off' } })
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(page.locator('[data-files-reason]')).toHaveCount(0)
})
