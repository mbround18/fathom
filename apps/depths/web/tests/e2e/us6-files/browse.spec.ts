/**
 * Spec 053 US1, browsing a stack's volumes (T031), against the fake backend: the mount list, a
 * folder listing that never climbs above its mount's root, a text file in the read-only editor
 * view, a binary file offered for download, and the cases that are listed but not opened (an
 * outside link, a risky bind, a volume that does not exist before the first deploy).
 *
 * SC-002 (50,000 entries visible in under 2 s, the filter answering in under 100 ms) is measured
 * in the page, so Playwright's own round trips do not count against it.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const filesTab = (page: Page) => page.locator('[data-tab="files"]')
const mounts = (page: Page) => page.locator('[data-pane="mounts"]')
const listing = (page: Page) => page.locator('[data-pane="listing"]')
const fileView = (page: Page) => page.locator('[data-pane="file"]')
const entry = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const crumbs = (page: Page) => listing(page).getByRole('navigation', { name: 'Folder path' })

/** `web` with one ready volume `web_data` (`app` at `/data`) holding a small tree; returns its id. */
function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'config/server.properties', 'motd=Hello\nmax-players=20\n')
  backend.files.write('web', data, 'config/world/level.json', '{"seed": 42}\n')
  backend.files.write('web', data, 'readme.txt', 'hi\n')
  return data
}

function noSidewaysScroll(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
}

test('mounts are listed per service with type, container path and read-only state', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', {
    source: 'web_data',
    services: [
      { service: 'app', destination: '/data', read_only: false },
      { service: 'backup', destination: '/backup/data', read_only: true },
    ],
  })
  const conf = backend.files.addMount('web', {
    kind: 'bind',
    source: '/srv/stacks/web/conf',
    services: [{ service: 'app', destination: '/etc/app', read_only: true }],
    writable: false,
  })
  await page.goto('/stacks/web/files')

  const app = mounts(page).locator('[data-service="app"]')
  await expect(app.getByRole('heading', { name: 'app' })).toBeVisible()
  const row = app.locator(`[data-mount-row="${data}"]`)
  await expect(row).toContainText('web_data')
  await expect(row).toContainText('volume')
  await expect(row).toContainText('/data')
  await expect(row).toContainText('read-write')
  const bind = app.locator(`[data-mount-row="${conf}"]`)
  await expect(bind).toContainText('bind')
  await expect(bind).toContainText('/srv/stacks/web/conf')
  await expect(bind).toContainText('/etc/app')
  await expect(bind).toContainText('read-only')

  // A mount shared by two services is listed under each, with that service's path and state.
  const backup = mounts(page).locator('[data-service="backup"]').locator(`[data-mount-row="${data}"]`)
  await expect(backup).toContainText('/backup/data')
  await expect(backup).toContainText('read-only')

  await row.getByRole('link', { name: 'web_data' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}$`))
  await expect(listing(page)).toBeVisible()
})

test('walk into a subfolder and back to the root, never above it', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)

  // Folders first, then files.
  await expect(listing(page).locator('[data-entry]').first()).toHaveAttribute('data-entry', 'config')
  await expect(entry(page, 'readme.txt')).toHaveAttribute('data-kind', 'file')
  await expect(listing(page).getByRole('link', { name: 'Up one folder' })).toHaveCount(0)

  await entry(page, 'config').getByRole('link', { name: 'config' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}/config$`))
  await expect(entry(page, 'world')).toHaveAttribute('data-kind', 'dir')
  await expect(entry(page, 'server.properties')).toBeVisible()
  await entry(page, 'world').getByRole('link', { name: 'world' }).click()
  await expect(entry(page, 'level.json')).toBeVisible()
  await expect(crumbs(page).getByRole('link')).toHaveText(['web_data', 'config'])
  await expect(crumbs(page).locator('[aria-current="page"]')).toHaveText('world')

  await listing(page).getByRole('link', { name: 'Up one folder' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}/config$`))
  await crumbs(page).getByRole('link', { name: 'web_data' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}$`))
  await expect(entry(page, 'readme.txt')).toBeVisible()
  // The root is the top: no way up, and the first crumb is the mount itself.
  await expect(listing(page).getByRole('link', { name: 'Up one folder' })).toHaveCount(0)
  await expect(crumbs(page).locator('[aria-current="page"]')).toHaveText('web_data')

  // Keyboard alone: the rows are one tab stop, arrows move between them, Enter opens.
  await entry(page, 'config').getByRole('link').focus()
  await page.keyboard.press('ArrowDown')
  await expect(entry(page, 'readme.txt').getByRole('link')).toBeFocused()
  await page.keyboard.press('Home')
  await expect(entry(page, 'config').getByRole('link')).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}/config$`))
})

test('a text file opens with line numbers and highlighting', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/config`)
  await entry(page, 'server.properties').getByRole('link', { name: 'server.properties' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}/config/server\\.properties$`))

  const view = fileView(page)
  await expect(view).toHaveAttribute('data-path', 'config/server.properties')
  await expect(view.locator('[data-file-header]')).toContainText('config/server.properties')
  await expect(view.locator('[data-file-header]')).toContainText('rw-r--r--')
  await expect(view.locator('[data-code-editor="properties"]')).toBeVisible()
  await expect(view.locator('.cm-lineNumbers .cm-gutterElement', { hasText: /^2$/ })).toBeVisible()
  await expect(view.locator('.cm-content')).toContainText('motd=Hello')
  await expect(view.locator('.cm-line span').first()).toBeVisible()
  // The listing stays beside the file on a wide screen.
  await expect(entry(page, 'server.properties')).toHaveAttribute('aria-current', 'true')
})

test('a read-only mount opens its files read-only, with the reason', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const ro = backend.files.addMount('web', {
    source: 'web_ro',
    writable: false,
    services: [{ service: 'app', destination: '/ro', read_only: true }],
  })
  backend.files.write('web', ro, 'a.ini', '[s]\nk=v\n')
  await page.goto(`/stacks/web/files/${ro}/a.ini`)
  await expect(fileView(page).locator('[data-code-editor]')).toHaveAttribute('data-read-only', 'true')
  await expect(fileView(page).locator('[data-read-only-reason]')).toContainText('read-only in every container')
})

test('a binary file shows its size and type, and a download', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.write('web', data, 'world.dat', new Uint8Array([1, 2, 0, 3, 4, 5, 6, 7]))
  await page.goto(`/stacks/web/files/${data}`)
  await entry(page, 'world.dat').getByRole('link', { name: 'world.dat' }).click()

  const view = fileView(page)
  await expect(view).toHaveAttribute('data-binary', 'binary')
  await expect(view).toContainText('8 B')
  await expect(view).toContainText('Binary file')
  await expect(view.locator('[data-code-editor]')).toHaveCount(0)
  const download = view.getByRole('link', { name: 'Download' })
  await expect(download).toHaveAttribute('href', `/api/v1/stacks/web/files/download?mount=${data}&path=world.dat`)
  const [file] = await Promise.all([page.waitForEvent('download'), download.click()])
  expect(file.suggestedFilename()).toBe('world.dat')
})

test('viewers get no Files tab', async ({ page, backend }) => {
  seed(backend)
  backend.me = f.me({ role: 'user', stacks: ['web'], features: { shell: true, edit: false, host_metrics: false, files: 'full' } })
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
  await expect(page.getByRole('navigation', { name: 'Stack views' })).toHaveCount(0)
  await page.goto('/stacks/web/files')
  await expect(filesTab(page)).toHaveCount(0)
})

test('a stack never deployed explains that its named volumes appear after the first deploy', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web', { status: 'not_deployed' })])
  const pending = backend.files.addMount('web', { source: 'web_data', state: 'not_created' })
  await page.goto('/stacks/web/files')
  const row = mounts(page).locator(`[data-mount-row="${pending}"]`)
  await expect(row).toHaveAttribute('data-state', 'not_created')
  await expect(row).toContainText('web_data')
  await expect(row.getByRole('link')).toHaveCount(0)
  await expect(mounts(page)).toContainText('Named volumes appear after the first deploy')
})

test('a link out of the mount shows its target and does not open', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.symlink('web', data, 'passwd', '/etc/passwd')
  backend.files.symlink('web', data, 'cfg', 'config')
  backend.files.special('web', data, 'pipe')
  await page.goto(`/stacks/web/files/${data}`)

  const out = entry(page, 'passwd')
  await expect(out).toHaveAttribute('data-kind', 'link')
  await expect(out).toContainText('/etc/passwd')
  await expect(out).toContainText('outside the mount')
  await expect(out.getByRole('link')).toHaveCount(0)
  await expect(entry(page, 'pipe').getByRole('link')).toHaveCount(0)

  // A link that stays inside is followed.
  await expect(entry(page, 'cfg')).toContainText('config')
  await entry(page, 'cfg').getByRole('link', { name: 'cfg' }).click()
  await expect(entry(page, 'server.properties')).toBeVisible()
})

test('a bind of / is listed with the reason and cannot be browsed', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const root = backend.files.addMount('web', {
    kind: 'bind',
    source: '/',
    services: [{ service: 'app', destination: '/host', read_only: true }],
    writable: false,
    browsable: { ok: false, reason: { code: 'risky_bind', message: 'the host root filesystem' } },
  })
  await page.goto('/stacks/web/files')
  const row = mounts(page).locator(`[data-mount-row="${root}"]`)
  await expect(row).toHaveAttribute('data-browsable', 'false')
  await expect(row).toContainText('the host root filesystem')
  await expect(row.getByRole('link')).toHaveCount(0)

  // A typed URL is refused by the server, and the page says why.
  await page.goto(`/stacks/web/files/${root}`)
  await expect(listing(page).locator('[data-refusal="not_browsable"]')).toContainText('the host root filesystem')
})

test('50,000 entries are visible in under 2 s and the filter answers in under 100 ms (SC-002)', async ({ page, backend }) => {
  const data = seed(backend)
  backend.files.seedFolder('web', data, 'big', 50_000)
  await page.goto(`/stacks/web/files/${data}`)
  await expect(entry(page, 'big')).toBeVisible()

  const started = await page.evaluate(() => performance.now())
  await entry(page, 'big').getByRole('link', { name: 'big' }).click()
  await expect(listing(page).locator('[data-entry-count]')).toHaveText('50,000 entries')
  await expect(entry(page, 'file-00001.txt')).toBeVisible()
  const listed = await page.evaluate(() => performance.now())
  expect(listed - started).toBeLessThan(2000)
  // Virtualized: a screenful of rows is in the DOM, not 50,000.
  expect(await listing(page).locator('[data-entry]').count()).toBeLessThan(200)

  // The filter, timed in the page from the input event to the count changing on screen.
  const filterMs = await page.evaluate(async () => {
    const input = document.querySelector<HTMLInputElement>('[data-pane="listing"] input[type="search"]')
    const count = document.querySelector('[data-pane="listing"] [data-entry-count]')
    if (!input || !count) throw new Error('no filter')
    // eslint-disable-next-line @typescript-eslint/unbound-method -- the native setter, so React sees the input event
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    const t0 = performance.now()
    setValue?.call(input, '49999')
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await new Promise<void>((resolve) => {
      const check = () => (count.textContent.startsWith('1 of') ? resolve() : requestAnimationFrame(check))
      check()
    })
    return performance.now() - t0
  })
  expect(filterMs).toBeLessThan(100)
  await expect(entry(page, 'file-49999.txt')).toBeVisible()
})

test('360 px wide: one pane at a time and no sideways scroll', async ({ page, backend }) => {
  const data = seed(backend)
  await page.setViewportSize({ width: 360, height: 740 })
  await page.goto('/stacks/web/files')
  await expect(mounts(page)).toBeVisible()
  expect(await noSidewaysScroll(page)).toBeLessThanOrEqual(0)

  await page.goto(`/stacks/web/files/${data}/config`)
  await expect(entry(page, 'server.properties')).toBeVisible()
  expect(await noSidewaysScroll(page)).toBeLessThanOrEqual(0)

  await entry(page, 'server.properties').getByRole('link', { name: 'server.properties' }).click()
  await expect(fileView(page).locator('.cm-content')).toContainText('motd=Hello')
  // The file takes the width; the listing is one step back.
  await expect(listing(page)).toBeHidden()
  expect(await noSidewaysScroll(page)).toBeLessThanOrEqual(0)
  await fileView(page).getByRole('link', { name: 'Back to config' }).click()
  await expect(page).toHaveURL(new RegExp(`/stacks/web/files/${data}/config$`))
  await expect(entry(page, 'server.properties')).toBeVisible()
})

test('a deep link to a file reopens the same file after a reload', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/config/world/level.json`)
  await expect(fileView(page)).toHaveAttribute('data-path', 'config/world/level.json')
  await expect(fileView(page).locator('[data-code-editor="json"]')).toBeVisible()
  await expect(fileView(page).locator('.cm-content')).toContainText('"seed": 42')
  await page.reload()
  await expect(fileView(page)).toHaveAttribute('data-path', 'config/world/level.json')
  await expect(fileView(page).locator('.cm-content')).toContainText('"seed": 42')
  await expect(entry(page, 'level.json')).toBeVisible()
})
