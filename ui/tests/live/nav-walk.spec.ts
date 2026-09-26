/**
 * LIVE, read-only: open every Sections-nav entry (NAV_GROUPS) and every nav tool (Fleet +
 * NAV_TOOLS) against the real backend. Each must settle (no pane left loading), show no error
 * state, no generic "Section unavailable" fallback, no placeholder text, and raise no page error.
 * A not-installed integration passes: it renders an explanatory empty state, not an error.
 * The mock-backend twin is tests/e2e/nav-walk.spec.ts.
 */
import type { Locator, Page } from '@playwright/test'

import { NAV_GROUPS, NAV_TOOLS } from '../../src/resources/nav'

import { expect, test } from './helpers'

const STUB =
  /section unavailable|coming soon|not implemented|under construction|\bTODO\b|lorem ipsum/i
const SETTLE_MS = 30_000

async function pick(page: Page, selector: string) {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const drawer = !(await nav.isVisible())
  if (drawer) await page.getByRole('button', { name: 'Open sections' }).click()
  await expect(nav).toBeVisible()
  // Group toggles are the only `aria-expanded` buttons in the nav; open them all.
  const collapsed = nav.locator('button[aria-expanded="false"]')
  while ((await collapsed.count()) > 0) await collapsed.first().click()
  await nav.locator(selector).click()
  if (drawer) await expect(nav).toBeHidden()
}

async function expectSettled(region: Locator, what: string) {
  await expect(region, `${what} visible`).toBeVisible()
  await expect(region.locator('[data-pane-state="loading"]'), `${what} still loading`).toHaveCount(
    0,
    { timeout: SETTLE_MS },
  )
  await expect(region.locator('[data-pane-state="error"]'), `${what} shows an error`).toHaveCount(0)
  await expect(region, `${what} shows placeholder text`).not.toContainText(STUB)
}

test.describe('live nav walk', () => {
  test.beforeEach(async ({ app }) => {
    await app.open()
    await app.selectNamespace('all')
  })

  for (const group of NAV_GROUPS) {
    test(`${group.label} sections settle without errors`, async ({ page }) => {
      test.setTimeout(group.sections.length * (SETTLE_MS + 5_000) + 30_000)
      const errors: string[] = []
      page.on('pageerror', (e) => errors.push(e.message))
      for (const s of group.sections) {
        await pick(page, `[data-section="${s.key}"]`)
        const pane = page.locator(`section[data-slot="pane"][data-section="${s.key}"]`).first()
        await expectSettled(pane, s.key)
        expect(errors, `page errors after opening ${s.key}`).toEqual([])
      }
    })
  }

  test('nav tools open and settle', async ({ page }) => {
    test.setTimeout(NAV_TOOLS.length * (SETTLE_MS + 5_000) + 30_000)
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    const cls = await page.locator('html').getAttribute('data-width-class')
    const sheet = cls !== 'phone' && cls !== 'tablet'
    // Not Fleet: it polls every kubeconfig context, and live runs must touch only the pinned
    // kind cluster. The mock walk (tests/e2e/nav-walk.spec.ts) covers Fleet.
    for (const tool of NAV_TOOLS) {
      await pick(page, `[data-tool="${tool}"]`)
      const region = sheet
        ? page.getByTestId('tools-sheet')
        : page.locator(`section[data-slot="pane"][data-tool="${tool}"]`).first()
      await expectSettled(region, tool)
      if (tool === 'logs') await expect(region.getByTestId('logs-tool')).toBeVisible()
      expect(errors, `page errors after opening ${tool}`).toEqual([])
      if (sheet) {
        await page.keyboard.press('Escape')
        await expect(region).toBeHidden()
      }
    }
  })
})
