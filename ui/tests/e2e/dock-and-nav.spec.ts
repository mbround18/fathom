/**
 * 043 US5 (T040, T045): a calm dock and a navigator a newcomer can follow.
 *
 * The dock holds only Logs and Events, plus Cluster history while the daemon is available; a
 * saved nine-tab dock loads filtered without being rewritten. The navigator shows the FR-020
 * groups in order, every entry carries its one-line description (tooltip and accessible
 * description), rarely used kinds sit under a persisted "More" toggle that search still sees,
 * and group labels saved before 043 map to the new groups. The status bar's left side is
 * unchanged at every width (FR-026).
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { DAEMON_CONTEXT, daemonStatus } from '../fixtures/daemon'
import type { UiConfig } from '../../src/protocol/types'
import { NAV_MODEL } from '../../src/resources/nav'
import { clearSent, emit, open, sent } from './helpers'

const hasDock = (info: TestInfo) => (info.project.metadata as { dock?: boolean }).dock === true

const dockTabs = (page: Page) => page.getByTestId('dock-tabs').getByRole('tab')

const nav = (page: Page) => page.getByRole('navigation', { name: 'Sections' })

const saves = async (page: Page) =>
  (await sent(page, true)).filter((c) => c.type === 'SaveUiConfig')

/** Show the navigator: the nav slot from laptop up, the drawer below. */
async function showNav(page: Page): Promise<void> {
  if (!(await nav(page).isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav(page)).toBeVisible()
}

const groupToggle = (page: Page, label: string) =>
  nav(page).locator(':scope > div > button[aria-expanded]', { hasText: label })

async function expandGroup(page: Page, label: string): Promise<void> {
  const toggle = groupToggle(page, label)
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
}

const GROUP_ORDER = [
  'Cluster',
  'Workloads',
  'Networking',
  'Storage',
  'Configuration',
  'Scaling',
  'Access Control',
  'Custom Resources',
  'Integrations',
  'Settings',
]

test.describe('calm dock (FR-024, FR-025)', () => {
  test('a fresh profile docks exactly Logs and Events', async ({ page }, info) => {
    test.skip(!hasDock(info), 'the dock exists from laptop width up')
    await open(page)
    await expect(dockTabs(page)).toHaveText(['Logs', 'Events'])
  })

  test('Cluster history joins the dock while the daemon is available', async ({ page }, info) => {
    test.skip(!hasDock(info), 'the dock exists from laptop width up')
    await open(page)
    await expect(dockTabs(page)).toHaveCount(2)
    await emit(page, daemonStatus('available', null, DAEMON_CONTEXT))
    await expect(dockTabs(page)).toHaveCount(3)
    const history = page.getByTestId('dock-tabs').locator('[role="tab"][data-tool="daemon"]')
    await expect(history).toBeVisible()
    await history.click()
    await expect(history).toHaveAttribute('aria-selected', 'true')
    // Unavailable again: the tab goes and the dock falls back to its first tab.
    await emit(page, daemonStatus('unavailable', 'connection refused', DAEMON_CONTEXT))
    await expect(dockTabs(page)).toHaveText(['Logs', 'Events'])
    await expect(dockTabs(page).first()).toHaveAttribute('aria-selected', 'true')
  })

  test('a saved nine-tab dock loads as Logs and Events, keeps its height, is not rewritten', async ({
    page,
  }, info) => {
    test.skip(!info.project.name.endsWith('-desktop'), 'one desktop-class profile is enough')
    await open(page)
    await clearSent(page)
    const saved: UiConfig = {
      schema: 1,
      theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
      density: {},
      layouts: {
        desktop: {
          slots: [
            { kind: 'nav', sizePct: 16 },
            { kind: 'list', sizePct: 48 },
            { kind: 'inspector', sizePct: 36 },
          ],
          dock: {
            open: true,
            heightPct: 44,
            tabs: [
              'logs',
              'events',
              'topology',
              'daemon',
              'velero',
              'certManager',
              'knative',
              'keda',
              'linter',
            ],
          },
        },
      },
    }
    await emit(
      page,
      batch([
        system(2, {
          type: 'UiConfigLoaded',
          data: { ui: saved, expanded_groups: [], mcp_stdio_enabled: false },
        }),
      ]),
    )
    await expect(dockTabs(page)).toHaveText(['Logs', 'Events'])
    // The saved height (44%) is kept: the dock panel's share of the row + dock group.
    const dockShare = async () => {
      const tabs = page.getByTestId('dock-tabs')
      const panel = await tabs
        .locator('xpath=ancestor::*[@data-slot="resizable-panel"][1]')
        .boundingBox()
      const group = await tabs
        .locator('xpath=ancestor::*[@data-slot="resizable-panel-group"][1]')
        .boundingBox()
      return panel && group ? Math.round((panel.height / group.height) * 100) : 0
    }
    await expect.poll(dockShare).toBeGreaterThanOrEqual(42)
    expect(await dockShare()).toBeLessThanOrEqual(46)
    // Loading and filtering never writes the file back (only a user layout change does).
    await page.waitForTimeout(800)
    expect(await saves(page)).toHaveLength(0)
  })
})

test.describe('newcomer navigator (FR-020 to FR-023)', () => {
  test('groups come in the FR-020 order', async ({ page }) => {
    await open(page)
    await showNav(page)
    // Each toggle also carries its decorative glyph after the label.
    await expect(nav(page).locator(':scope > div > button[aria-expanded]')).toHaveText(
      GROUP_ORDER.map((label) => new RegExp(`^${label}`)),
    )
  })

  test('every entry shows its description as tooltip and accessible description', async ({
    page,
  }) => {
    await open(page)
    await showNav(page)
    await expandGroup(page, 'Workloads')
    const pods = nav(page).getByRole('button', { name: 'Pods', exact: true })
    const description = NAV_MODEL.flatMap((g) => g.entries).find(
      (e) => e.section === 'workloads/pods',
    )?.description
    expect(description).toBeTruthy()
    await expect(pods).toHaveAttribute('title', description ?? '')
    await expect(pods).toHaveAccessibleDescription(description ?? '')
    // Every rendered entry has one, in every expanded group.
    for (const label of GROUP_ORDER) await expandGroup(page, label)
    const entries = nav(page).locator('button[data-section]')
    const count = await entries.count()
    expect(count).toBeGreaterThan(20)
    for (let i = 0; i < count; i++) {
      const entry = entries.nth(i)
      await expect(entry).toHaveAttribute('aria-describedby', /nav-desc-/)
      expect((await entry.getAttribute('title'))?.length ?? 0).toBeGreaterThan(10)
    }
  })

  test('"More" is collapsed by default, persists, and search still finds advanced kinds', async ({
    page,
  }) => {
    await open(page)
    await showNav(page)
    await expandGroup(page, 'Workloads')
    const more = nav(page).getByRole('button', { name: 'More Workloads', exact: true })
    await expect(more).toHaveAttribute('aria-expanded', 'false')
    await expect(nav(page).getByRole('button', { name: 'ReplicaSets', exact: true })).toHaveCount(0)

    // The palette indexes the collapsed kinds.
    await page.keyboard.press('Escape')
    await page.getByTestId('palette-open').click()
    await page.getByPlaceholder(/Jump to a resource/).fill('ReplicaSets')
    const option = page.getByRole('option', { name: /ReplicaSets/ })
    await expect(option).toBeVisible()
    await expect(option.getByTestId('palette-description')).toHaveText(
      'The copies a Deployment manages; usually read through it',
    )
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toHaveCount(0)

    await clearSent(page)
    await showNav(page)
    await more.click()
    await expect(more).toHaveAttribute('aria-expanded', 'true')
    await expect(nav(page).getByRole('button', { name: 'ReplicaSets', exact: true })).toBeVisible()
    await expect
      .poll(async () => {
        const last = (await saves(page)).at(-1) as
          { data: { expanded_groups: string[] } } | undefined
        return last?.data.expanded_groups ?? []
      })
      .toContain('Workloads/More')
  })

  test('group labels saved before 043 open the groups they now mean', async ({ page }) => {
    await open(page)
    await emit(
      page,
      batch([
        system(2, {
          type: 'UiConfigLoaded',
          data: { ui: null, expanded_groups: ['Overview', 'Config'], mcp_stdio_enabled: false },
        }),
      ]),
    )
    await showNav(page)
    for (const [label, open] of [
      ['Cluster', 'true'],
      ['Configuration', 'true'],
      ['Scaling', 'true'],
      ['Workloads', 'false'],
    ] as const) {
      await expect(groupToggle(page, label)).toHaveAttribute('aria-expanded', open)
    }
  })
})

test.describe('status bar (FR-026)', () => {
  test('the left side keeps connection state and context, with nothing new on the right', async ({
    page,
  }) => {
    await open(page)
    const bar = page.getByTestId('status-bar')
    await expect(bar).toBeVisible()
    await expect(bar.locator('[data-conn-status]')).toBeVisible()
    await expect(bar).toContainText(/ctx\s+kind-a/)
    // No controls on a quiet start: the right side stays free for Group 4.
    await expect(bar.locator('button, a')).toHaveCount(0)
  })
})
