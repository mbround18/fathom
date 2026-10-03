/**
 * Nav walk (mock backend): every Sections-nav entry (NAV_GROUPS) and every nav tool (Fleet +
 * NAV_TOOLS) opens a pane that is not an error, not the generic "Section unavailable" fallback,
 * and not placeholder text, without a page error. The live twin is tests/live/nav-walk.spec.ts.
 */
import { expect, test, type Page } from '@playwright/test'

import { NAV_GROUPS, NAV_TOOLS } from '../../src/resources/nav'

import { batch, system } from '../fixtures/builders'
import { VELERO_HEALTHY_AND_FAILED } from '../fixtures/velero'
import { emit, open, sentTypes } from './helpers'

/** Text that means a pane is unfinished rather than empty. */
const STUB =
  /section unavailable|coming soon|not implemented|under construction|\bTODO\b|lorem ipsum/i

// Phone (drawer + stacked panes), laptop (tools sheet) and wide (docked) cover every shell mode.
const MODES = /-(phone|laptop|wide)$/

async function openNav(page: Page) {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const drawer = !(await nav.isVisible())
  if (drawer) await page.getByRole('button', { name: 'Open sections' }).click()
  await expect(nav).toBeVisible()
  // Group toggles are the only `aria-expanded` buttons in the nav; open them all.
  const collapsed = nav.locator('button[aria-expanded="false"]')
  while ((await collapsed.count()) > 0) await collapsed.first().click()
  return { nav, drawer }
}

/** Click a nav entry; in drawer mode wait for the drawer to finish closing. */
async function pick(page: Page, selector: string) {
  const { nav, drawer } = await openNav(page)
  await nav.locator(selector).click()
  if (drawer) await expect(nav).toBeHidden()
}

function trackErrors(page: Page): string[] {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  return errors
}

async function expectHealthy(region: ReturnType<Page['locator']>) {
  await expect(region).toBeVisible()
  await expect(region.locator('[data-pane-state="error"]')).toHaveCount(0)
  await expect(region).not.toContainText(STUB)
}

test.describe('nav walk: every nav entry opens a working pane', () => {
  test.beforeEach(() => {
    const project = test.info().project.name
    test.skip(!MODES.test(project), 'phone, laptop and wide cover every shell mode')
  })

  for (const group of NAV_GROUPS) {
    test(`${group.label} sections`, async ({ page }) => {
      const errors = trackErrors(page)
      // The Containers group exists only where a container runtime does (048 FR-001), so walking
      // it needs a scenario whose daemon answers — with `default` the group is absent and every
      // section in it would be looked for in a nav that never listed it.
      await open(page, group.label === 'Containers' ? 'containers' : 'default')
      // Velero is listed only once detected (043 FR-032): report it installed so it is walked too.
      await emit(
        page,
        batch([system(1, { type: 'VeleroOverviewUpdated', data: VELERO_HEALTHY_AND_FAILED })]),
      )
      for (const s of group.sections) {
        await pick(page, `[data-section="${s.key}"]`)
        const pane = page.locator(`section[data-slot="pane"][data-section="${s.key}"]`).first()
        await expectHealthy(pane)
        expect(errors, `page errors after opening ${s.key}`).toEqual([])
      }
    })
  }

  // Stuck-loading is checked by the live walk; the fake backend leaves some lists unanswered.
  // Argo used to wait forever on a CRD catalog only the Custom Resources pane requested.
  test('Argo opened first asks for the CRD catalog itself', async ({ page }) => {
    await open(page)
    await pick(page, '[data-section="integrations/argo"]')
    await expect.poll(() => sentTypes(page)).toContain('RefreshCrds')
  })

  test('nav tools', async ({ page }) => {
    const errors = trackErrors(page)
    await open(page)
    const cls = await page.locator('html').getAttribute('data-width-class')
    for (const tool of ['fleet', ...NAV_TOOLS]) {
      await pick(page, `[data-tool="${tool}"]`)
      let region
      if (tool === 'fleet') region = page.getByTestId('fleet-dashboard')
      else if (cls !== 'phone' && cls !== 'tablet') region = page.getByTestId('tools-sheet')
      else region = page.locator(`section[data-slot="pane"][data-tool="${tool}"]`).first()
      await expectHealthy(region)
      if (tool === 'logs') await expect(region.getByTestId('logs-tool')).toBeVisible()
      expect(errors, `page errors after opening ${tool}`).toEqual([])
      // Fleet and the tools sheet are modal: close them before the next nav click.
      if (tool === 'fleet' || (cls !== 'phone' && cls !== 'tablet')) {
        await page.keyboard.press('Escape')
        await expect(region).toBeHidden()
      }
    }
  })
})
