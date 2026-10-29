/**
 * Spec 062 US1 (T034): see every stack that has a newer image.
 *
 * - Checks off: the page says what checking does and turns checks on through a preview.
 * - Every state the fixture holds shows its label and reason; can't-check names the registry;
 *   Depths' own container shows the manual steps instead of an update.
 * - Filters by state and stack; the badge counts stacks with an update and follows the feed.
 * - Check now: started, busy, and refused while checks are off.
 * - A user sees no Updates link or page, and sees states only on their own stack's page.
 *
 * Nothing here talks to a registry: the fake backend answers every command.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const rows = (page: Page) => page.locator('[data-update-row]')
const row = (page: Page, id: string) => page.locator(`[data-update-row="${id}"]`)

function seed(backend: FakeBackend, view = f.updatesView()): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('blog'), f.stack('media')])
  backend.setUpdates(view)
}

for (const width of [360, 1280]) {
  test.describe(`at ${String(width)} px`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: 900 })
    })

    test('every state shows its label and reason, and nothing overflows', async ({ page, backend }) => {
      seed(backend)
      await page.goto('/admin/updates')

      await expect(rows(page)).toHaveCount(7)
      await expect(row(page, 'web/app').locator('[data-update-state="available"]')).toHaveText('Update available')
      await expect(row(page, 'web/app')).toContainText('1.1.0 (9999ffff8888)')
      await expect(row(page, 'web/app')).toContainText('1.0.0 (1111aaaa2222)')
      await expect(row(page, 'web/cache').locator('[data-update-state]')).toHaveText('Up to date')
      await expect(row(page, 'blog/db').locator('[data-update-state]')).toHaveText("Can't check")
      await expect(row(page, 'blog/db').locator('[data-field="reason"]')).toContainText(
        "can't check: registry.example.com answered 401",
      )
      await expect(row(page, 'media/jellyfin').locator('[data-update-state]')).toHaveText('Not checked')
      await expect(row(page, 'media/jellyfin')).toContainText('no container')
      await expect(row(page, 'media/local').locator('[data-field="reason"]')).toHaveText('a local image with no registry')
      const self = row(page, 'depths/depths')
      await expect(self.locator('[data-field="reason"]')).toContainText('Depths never updates itself')
      await expect(self.locator('[data-field="self-steps"]')).toContainText('docker compose pull depths')

      await expect(page.getByTestId('updates-schedule')).toContainText('Checks every 6 hours')
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
      expect(overflow).toBeLessThanOrEqual(0)
    })

    test('filters narrow by state and by stack', async ({ page, backend }) => {
      seed(backend)
      await page.goto('/admin/updates')
      await expect(rows(page)).toHaveCount(7)

      await page.getByLabel('State').selectOption('available')
      await expect(rows(page)).toHaveCount(2)
      await expect(page.getByTestId('updates-count')).toHaveText('2 of 7')

      await page.getByLabel('Stack', { exact: true }).selectOption('blog')
      await expect(rows(page)).toHaveCount(1)
      await expect(row(page, 'blog/ghost')).toBeVisible()

      await page.getByLabel('State').selectOption('all')
      await expect(rows(page)).toHaveCount(2)
      await page.getByLabel('Stack', { exact: true }).selectOption('')
      await expect(rows(page)).toHaveCount(7)
    })
  })
}

test('checks off: the page explains them and turns them on through a preview', async ({ page, backend }) => {
  seed(backend, f.updatesView({ settings: f.updateSettings(), next_check: null }))
  await page.goto('/admin/updates')

  const off = page.locator('[data-pane-state="checks-off"]')
  await expect(off).toContainText('Update checks are off.')
  await expect(off).toContainText('never pulls an image or restarts a container')
  await expect(page.locator('[data-action="check-now"]')).toHaveCount(0)

  await page.getByLabel('Check interval').selectOption(String(24 * 3600))
  await page.locator('[data-action="turn-checks-on"]').click()
  const dialog = page.getByTestId('updates-settings-dialog')
  await expect(dialog.locator('[data-field="summary"]')).toContainText('update checks: off -> on')
  await expect(dialog.locator('[data-field="summary"]')).toContainText('check interval: every 6 h -> every 24 h')
  await dialog.locator('[data-action="confirm"]').click()

  await expect(dialog).toBeHidden()
  await expect(off).toHaveCount(0)
  await expect(page.getByTestId('updates-schedule')).toContainText('Checks every day')
  expect(backend.audit.some((e) => e.action === 'update_settings')).toBe(true)
})

test('Check now: started, then busy', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')

  await page.locator('[data-action="check-now"]').click()
  await expect(page.locator('[data-check="started"]')).toBeVisible()

  backend.checkBusySecs = 120
  await page.locator('[data-action="check-now"]').click()
  await expect(page.locator('[data-check="busy"]')).toContainText('Try again in 2 min')
})

test('the badge counts stacks with an update and follows the feed', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')

  const link = page.getByRole('link', { name: /^Updates/ })
  await expect(link).toHaveAccessibleName('Updates: 2 stacks have an update')
  await expect(page.getByTestId('updates-badge')).toHaveText('2')

  const services = f.updatesView().services.map((s) => (s.stack === 'blog' && s.state === 'available' ? { ...s, state: 'up_to_date' as const, available: null } : s))
  backend.pushUpdates(services)
  await expect(page.getByTestId('updates-badge')).toHaveText('1')

  backend.pushUpdates(services.map((s) => (s.state === 'available' ? { ...s, state: 'up_to_date' as const, available: null } : s)))
  await expect(page.getByTestId('updates-badge')).toHaveCount(0)
  await expect(link).toHaveAccessibleName('Updates')
})

test('the page follows UpdatesChanged', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')
  await expect(row(page, 'web/cache').locator('[data-update-state]')).toHaveText('Up to date')

  backend.pushUpdates(f.updatesView().services.map((s) => (s.service === 'cache' ? { ...s, state: 'available' as const } : s)))
  await expect(row(page, 'web/cache').locator('[data-update-state]')).toHaveText('Update available')
})

test('updates off: no link and no page', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  await page.goto('/admin/updates')
  await expect(page.locator('[data-page="updates"]')).toHaveCount(0)
  await expect(page.getByRole('link', { name: /^Updates/ })).toHaveCount(0)
})

test('a user sees no Updates page, and states only on their own stack', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web'], { features: { ...f.me().features, updates: true } })
  const services = f.updatesView().services
  backend.details.web = f.detail(f.stack('web'), { updates: services.filter((s) => s.stack === 'web') })

  await page.goto('/admin/updates')
  await expect(page.locator('[data-page="updates"]')).toHaveCount(0)
  await expect(page.getByRole('link', { name: /^Updates/ })).toHaveCount(0)
  await expect(page.getByTestId('updates-badge')).toHaveCount(0)

  await page.goto('/stacks/web')
  const app = page.locator('[data-service="app"] [data-field="update"]')
  await expect(app.locator('[data-update-state]')).toHaveText('Update available')
  await expect(app).toContainText('1.1.0 (9999ffff8888)')
  // Never asked for the page's list, and never sent it.
  expect(backend.received.filter((c) => c.type === 'ListUpdates')).toHaveLength(0)
})

test('a restricted network sees the list but cannot check or change settings', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.untrusted({ needs_answer: false, features: { ...f.me().features, updates: true } })
  await page.goto('/admin/updates')
  await expect(rows(page)).toHaveCount(7)
  await expect(page.locator('[data-action="check-now"]')).toHaveCount(0)
  await expect(page.getByLabel('Check interval')).toHaveCount(0)
})
