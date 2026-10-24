/**
 * Spec 053, the volume explorer: the Files tab on the stack page (T027). It is a URL of its own
 * (`/stacks/<stack>/files/<mount>/<path>`, FR-024), lazy-loaded, and only there for people with the
 * files feature on and edit rights. Later tasks add the browse, edit, transfer, manage and history
 * specs beside this one.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('web')])
}

const views = (page: Page) => page.getByRole('navigation', { name: 'Stack views' })
const filesLink = (page: Page) => views(page).getByRole('link', { name: 'Files' })
const filesTab = (page: Page) => page.locator('[data-tab="files"]')

test('the stack page opens', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
})

test('the Files tab opens from the stack page and back', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web')
  await expect(views(page).getByRole('link', { name: 'Overview' })).toHaveAttribute('aria-current', 'page')
  await filesLink(page).click()
  await expect(page).toHaveURL(/\/stacks\/web\/files$/)
  await expect(filesLink(page)).toHaveAttribute('aria-current', 'page')
  await expect(filesTab(page).getByRole('status')).toHaveText('No mounts yet.')
  await expect(page.locator('[data-layout="stack-body"]')).toHaveCount(0)

  await views(page).getByRole('link', { name: 'Overview' }).click()
  await expect(page).toHaveURL(/\/stacks\/web$/)
  await expect(page.locator('[data-layout="stack-body"]')).toBeVisible()
})

test('a deep link names the mount and path, and survives a reload', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web/files/ab12cd34ef56ab78/config/server%20one.properties')
  await expect(filesTab(page)).toHaveAttribute('data-mount', 'ab12cd34ef56ab78')
  await expect(filesTab(page)).toHaveAttribute('data-path', 'config/server one.properties')
  await page.reload()
  await expect(filesTab(page)).toHaveAttribute('data-path', 'config/server one.properties')
})

test('a path with an encoded slash is not a place in the stack', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/stacks/web/files/ab12/a%2Fb')
  await expect(page.locator('[data-pane-state="bad-path"]')).toBeVisible()
  await page.getByRole('link', { name: 'Show its mounts' }).click()
  await expect(page).toHaveURL(/\/stacks\/web\/files$/)
})

for (const [why, over] of [
  ['the server does not report files', { features: { shell: true, edit: true, host_metrics: false } }],
  ['files are off', { features: { shell: true, edit: true, host_metrics: false, files: 'off' as const } }],
  ['editing is off', { features: { shell: true, edit: false, host_metrics: false, files: 'full' as const } }],
] as const) {
  test(`no Files tab when ${why}`, async ({ page, backend }) => {
    seed(backend)
    backend.me = f.me(over)
    await page.goto('/stacks/web')
    await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
    await expect(views(page)).toHaveCount(0)
    await page.goto('/stacks/web/files')
    await expect(page.locator('[data-pane-state="unavailable"]')).toBeVisible()
    await expect(filesTab(page)).toHaveCount(0)
  })
}

test('read-only mode still shows the tab', async ({ page, backend }) => {
  seed(backend)
  backend.me = f.me({ features: { shell: true, edit: true, host_metrics: false, files: 'read_only' } })
  await page.goto('/stacks/web')
  await expect(filesLink(page)).toBeVisible()
})

test('360 px wide: the tabs fit without sideways scroll', async ({ page, backend }) => {
  seed(backend)
  await page.setViewportSize({ width: 360, height: 740 })
  await page.goto('/stacks/web/files')
  await expect(filesTab(page)).toBeVisible()
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  expect(overflow).toBeLessThanOrEqual(0)
})
