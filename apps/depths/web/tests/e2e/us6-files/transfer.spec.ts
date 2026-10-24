/**
 * Spec 053 US3, downloads and uploads (T048), against the fake backend: a file downloads under its
 * own name and a folder as `<name>.tar`; a folder over the server's limit says how big it is and
 * starts nothing; an upload names the folder and every file (replacements marked) before a byte is
 * sent, then shows progress; a cancelled upload writes nothing; a running service is warned about
 * and can be stopped first.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const listing = (page: Page) => page.locator('[data-pane="listing"]')
const row = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const fileDialog = (page: Page) => page.locator('[data-file-op]')

function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'notes.txt', 'hi\n')
  backend.files.write('web', data, 'logs/a.log', 'alpha\n')
  backend.files.write('web', data, 'logs/b.log', 'beta\n')
  return data
}

async function openFolder(page: Page, mount: string): Promise<void> {
  await page.goto(`/stacks/web/files/${mount}`)
  await expect(listing(page).locator('[data-entry]').first()).toBeVisible()
}

/**
 * What the link serves. A browser download skips Playwright's routing, so the bytes are fetched
 * from inside the page, through the fake, at the same href.
 */
async function served(page: Page, href: string): Promise<{ bytes: number[]; disposition: string | null }> {
  return page.evaluate(async (u) => {
    const r = await fetch(u)
    return { bytes: [...new Uint8Array(await r.arrayBuffer())], disposition: r.headers.get('content-disposition') }
  }, href)
}

const downloads = (backend: FakeBackend) => backend.http.filter((c) => c.path.endsWith('/files/download'))
const uploads = (backend: FakeBackend) => backend.http.filter((c) => c.path.endsWith('/files/upload'))

test('a file downloads under its own name with its bytes', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await row(page, 'notes.txt').locator('[data-row-actions]').click()
  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('[data-row-menu="notes.txt"] [data-action="download"]').click()])
  expect(download.suggestedFilename()).toBe('notes.txt')
  expect(download.url()).toMatch(new RegExp(`/api/v1/stacks/web/files/download\\?mount=${data}&path=notes\\.txt$`))
  const body = await served(page, download.url())
  expect(Buffer.from(body.bytes).toString()).toBe('hi\n')
  expect(body.disposition).toContain('notes.txt')
})

test('a folder downloads as a tar after a size check', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await row(page, 'logs').locator('[data-row-actions]').click()
  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('[data-row-menu="logs"] [data-action="download"]').click()])
  expect(download.suggestedFilename()).toBe('logs.tar')
  expect(download.url()).toMatch(/path=logs$/)
  const body = await served(page, download.url())
  expect(body.disposition).toContain('logs.tar')
  const tar = Buffer.from(body.bytes)
  expect(tar.subarray(257, 262).toString()).toBe('ustar')
  expect(tar.toString('latin1')).toContain('alpha\n')
  expect(backend.received.filter((c) => c.type === 'FolderSize')).toHaveLength(1)
  await expect(page.locator('[data-download-note="started"]')).toContainText('as a tar archive')
})

test('the folder on screen downloads from the toolbar', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/logs`)
  await expect(row(page, 'a.log')).toBeVisible()
  const [download] = await Promise.all([page.waitForEvent('download'), listing(page).locator('[data-download-folder]').click()])
  expect(download.suggestedFilename()).toBe('logs.tar')
})

test('a folder over the download limit says how big it is and starts nothing', async ({ page, backend }) => {
  const data = seed(backend)
  backend.setFiles('full', { download_max: 4 })
  let started = 0
  page.on('download', () => (started += 1))
  await openFolder(page, data)
  await row(page, 'logs').locator('[data-row-actions]').click()
  await page.locator('[data-row-menu="logs"] [data-action="download"]').click()
  const note = page.locator('[data-download-note="too_large"]')
  // The helper stops counting once past the limit, so the size is a floor.
  await expect(note).toHaveText(/^This folder is more than \d+ B, over this server's download limit of 4 B, so nothing was downloaded\./)
  expect(backend.received.filter((c) => c.type === 'FolderSize')).toHaveLength(1)
  expect(downloads(backend)).toHaveLength(0)
  expect(started).toBe(0)
  await note.getByRole('button', { name: 'Dismiss' }).click()
  await expect(note).toHaveCount(0)
})

test('upload: the confirm names the folder and every file before a byte is sent', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/logs`)
  await expect(row(page, 'a.log')).toBeVisible()
  await listing(page)
    .locator('[data-upload-input]')
    .setInputFiles([
      { name: 'a.log', mimeType: 'text/plain', buffer: Buffer.from('new alpha\n') },
      { name: 'c.log', mimeType: 'text/plain', buffer: Buffer.from('gamma\n') },
    ])

  const cmd = await backend.waitForCmd('PreviewFileOp')
  expect(cmd.data.op).toEqual({
    kind: 'upload',
    options: { mount: data, dir: 'logs', files: [{ name: 'a.log', size: 10 }, { name: 'c.log', size: 6 }] },
  })
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'upload')
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Upload 2 files to logs')
  await expect(fileDialog(page).locator('[data-upload="a.log"]')).toHaveAttribute('data-replaces', 'true')
  await expect(fileDialog(page).locator('[data-upload="a.log"]')).toContainText('replaces the existing file')
  await expect(fileDialog(page).locator('[data-upload="c.log"]')).not.toHaveAttribute('data-replaces')
  expect(uploads(backend)).toHaveLength(0)

  await fileDialog(page).locator('[data-confirm]').click()
  await expect(fileDialog(page)).toHaveCount(0)
  const progress = page.locator('[data-upload-progress="finished"]')
  await expect(progress).toContainText('Uploaded 2 files.')
  await expect(progress.locator('[data-upload-item="a.log"]')).toHaveAttribute('data-state', 'done')
  await expect(progress.getByRole('progressbar', { name: 'Uploading c.log' })).toBeVisible()
  expect(uploads(backend).map((c) => c.csrf)).toEqual(['csrf-fake-1', 'csrf-fake-1'])
  expect(backend.files.read('web', data, 'logs/a.log')).toBe('new alpha\n')
  expect(backend.files.read('web', data, 'logs/c.log')).toBe('gamma\n')
  await expect(row(page, 'c.log')).toBeVisible()
})

test('a cancelled upload writes nothing', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  await listing(page)
    .locator('[data-upload-input]')
    .setInputFiles([{ name: 'new.txt', mimeType: 'text/plain', buffer: Buffer.from('x') }])
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Upload 1 file to /')
  await fileDialog(page).getByRole('button', { name: 'Cancel' }).click()
  await expect(fileDialog(page)).toHaveCount(0)
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(0)
  expect(uploads(backend)).toHaveLength(0)
  expect(backend.files.exists('web', data, 'new.txt')).toBe(false)
  await expect(page.locator('[data-upload-progress]')).toHaveCount(0)
})

test('files dropped on the listing upload the same way', async ({ page, backend }) => {
  const data = seed(backend)
  await openFolder(page, data)
  const drop = await page.evaluateHandle(() => {
    const dt = new DataTransfer()
    dt.items.add(new File(['dropped\n'], 'drop.txt', { type: 'text/plain' }))
    return dt
  })
  const target = page.locator('[data-drop-target]')
  await target.dispatchEvent('dragenter', { dataTransfer: drop })
  await target.dispatchEvent('dragover', { dataTransfer: drop })
  await target.dispatchEvent('drop', { dataTransfer: drop })
  await expect(fileDialog(page).locator('[data-field="summary"]')).toHaveText('Upload 1 file to /')
  await fileDialog(page).locator('[data-confirm]').click()
  await expect(page.locator('[data-upload-progress="finished"]')).toContainText('Uploaded 1 file.')
  expect(backend.files.read('web', data, 'drop.txt')).toBe('dropped\n')
})

test('a running service is warned about and can be stopped first', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.setRunning('web', data, ['app'])
  await openFolder(page, data)
  await listing(page)
    .locator('[data-upload-input]')
    .setInputFiles([{ name: 'new.txt', mimeType: 'text/plain', buffer: Buffer.from('x') }])
  await expect(fileDialog(page).locator('[data-running="app"]')).toContainText('app is running')
  await fileDialog(page).locator('[data-stop-first="app"]').click()
  await expect(page.locator('[data-preview="service_stop"]')).toBeVisible()
  expect(uploads(backend)).toHaveLength(0)
})

test('a read-only mount still downloads but offers no upload', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data', writable: false })
  backend.files.write('web', data, 'notes.txt', 'hi\n')
  await openFolder(page, data)
  await expect(listing(page).locator('[data-upload-pick]')).toHaveCount(0)
  await expect(page.locator('[data-drop-target]')).toHaveCount(0)
  const [download] = await Promise.all([page.waitForEvent('download'), listing(page).locator('[data-download-folder]').click()])
  expect(download.suggestedFilename()).toBe('web_data.tar')
})
